//! The server of Apeiron's pages and JSON API, the C ABI over HTTP. In local
//! mode (DESKTOP.md) it listens only on `127.0.0.1` for holders of a token;
//! in web mode (WEB.md) it serves one repository to the network, over TLS,
//! to a signed-in browser, or one for each tenant at its own address
//! (TENANTS.md).

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod audit;
mod oidc;
mod replicate;
mod shares;
mod tenants;
mod users;
mod web;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

pub use audit::{Actor, Audit, Event, Verified};
pub use oidc::{OidcConfig, Provider, check_provider};
pub use shares::{DEFAULT_DAYS, MAX_DAYS, Share, Shares};
pub use tenants::{Tenant, Tenants};
pub use users::{MIN_PASSWORD, Role, User, Users, hash_password};
pub use web::{TenantOptions, Tls, Web, WebOptions};
use web::{WebState, Who};

use apeiron::json::{self, JsonJob};
use apeiron::{Error, ErrorKind, ListRequest, Repository};
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path as UrlPath, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower::ServiceExt;

/// The pages, compiled into the program (DESKTOP.md §1).
const PAGES: &[(&str, &str, &str)] = &[
    (
        "/",
        "text/html; charset=utf-8",
        include_str!("../web/index.html"),
    ),
    (
        "/app.js",
        "text/javascript; charset=utf-8",
        include_str!("../web/app.js"),
    ),
    (
        "/i18n.js",
        "text/javascript; charset=utf-8",
        include_str!("../web/i18n.js"),
    ),
    (
        "/style.css",
        "text/css; charset=utf-8",
        include_str!("../web/style.css"),
    ),
];

/// The pages load nothing from elsewhere (DESKTOP.md §4).
const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self'; \
                   script-src 'self'; frame-ancestors 'none'; base-uri 'none'";

/// Who may call the server and what they may reach.
enum Mode {
    Local {
        token: String,
        /// `127.0.0.1:<port>`, the only `Host` accepted.
        host: String,
    },
    Web(Box<WebState>),
}

/// A running job, with the user who started it in web mode.
struct JobEntry {
    owner: Option<String>,
    operation: String,
    job: Box<dyn JsonJob>,
}

type Jobs = HashMap<u64, JobEntry>;

struct App {
    mode: Mode,
    repo: Mutex<Option<Repository>>,
    jobs: Mutex<Jobs>,
    next_job: AtomicU64,
    /// Where the last repository opened is remembered.
    memory: Option<PathBuf>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the server remembers between runs.
#[derive(Default, Serialize, Deserialize)]
struct Memory {
    last: Option<PathBuf>,
}

impl App {
    fn remembered(&self) -> Memory {
        self.memory
            .as_ref()
            .and_then(|p| fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn remember(&self, path: &Path) {
        if let Some(file) = &self.memory {
            let memory = Memory {
                last: Some(path.to_path_buf()),
            };
            // Forgetting the last repository is not worth failing for.
            if let (Some(dir), Ok(data)) = (file.parent(), serde_json::to_vec(&memory)) {
                let _ = fs::create_dir_all(dir).and_then(|()| fs::write(file, data));
            }
        }
    }

    fn repo(&self) -> Result<Repository, Error> {
        lock(&self.repo)
            .clone()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "no repository is open; open one first"))
    }
}

/// An error as JSON with an HTTP status (DESKTOP.md §3).
struct Failure(StatusCode, Value);

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        let status = match e.kind {
            ErrorKind::InvalidInput | ErrorKind::Ambiguous | ErrorKind::Unsupported => {
                StatusCode::BAD_REQUEST
            }
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::Conflict | ErrorKind::AlreadyExists | ErrorKind::Locked => {
                StatusCode::CONFLICT
            }
            ErrorKind::Key => StatusCode::UNAUTHORIZED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Failure(status, serde_json::to_value(&e).unwrap_or(Value::Null))
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, axum::Json(self.1)).into_response()
    }
}

type Answer = Result<axum::Json<Value>, Failure>;

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

fn body(bytes: &Bytes) -> Result<Value, Error> {
    if bytes.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(bytes).map_err(|e| invalid(format!("request: {e}")))
}

/// Runs blocking work of the API off the server's threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(Error::new(ErrorKind::Internal, e.to_string())))
}

fn refused(status: StatusCode, message: &str) -> Response {
    Failure(
        status,
        json!({"code": 4, "kind": "invalid_input", "message": message}),
    )
    .into_response()
}

/// Refuses requests from elsewhere than the pages (DESKTOP.md §4, WEB.md §5).
async fn guard(State(app): State<Arc<App>>, mut request: Request, next: Next) -> Response {
    let headers = request.headers();
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let api = request.uri().path().starts_with("/api/");
    match &app.mode {
        Mode::Local { token, host } => {
            let origin = format!("http://{host}");
            if header(header::HOST) != Some(host)
                || header(header::ORIGIN).is_some_and(|o| o != origin)
            {
                return refused(StatusCode::FORBIDDEN, "request from another site");
            }
            if api && header(header::AUTHORIZATION) != Some(&format!("Bearer {token}")) {
                return refused(
                    StatusCode::UNAUTHORIZED,
                    "missing or wrong token; open the interface again",
                );
            }
        }
        Mode::Web(web) => {
            if let Some(origin) = header(header::ORIGIN) {
                let host = origin
                    .strip_prefix("https://")
                    .or_else(|| origin.strip_prefix("http://"));
                if host.is_none() || host != header(header::HOST) {
                    return refused(StatusCode::FORBIDDEN, "request from another site");
                }
            }
            // A share link is used without signing in (SHARE.md §4).
            let open = matches!(
                request.uri().path(),
                "/api/session"
                    | "/api/login"
                    | "/api/oidc/start"
                    | "/api/oidc/callback"
                    | "/api/health"
            ) || request.uri().path().starts_with("/api/shared/");
            match web.current(headers) {
                Some(user) => {
                    let from = peer(&request);
                    request.extensions_mut().insert(Who {
                        user: Arc::new(user),
                        from,
                    });
                }
                None if api && !open => {
                    return Failure(
                        StatusCode::UNAUTHORIZED,
                        json!({"code": 4, "kind": "invalid_input", "message": "sign in first", "sign_in": true}),
                    )
                    .into_response();
                }
                None => {}
            }
        }
    }
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // A share link in the address must not leave with a request elsewhere.
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// The address of the connection, as `address:port`.
fn peer(request: &Request) -> Option<String> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.to_string())
}

async fn page(request: Request) -> Response {
    // A share link shows the same pages (SHARE.md §4).
    let path = match request.uri().path() {
        p if p.starts_with("/s/") => "/",
        p => p,
    };
    match PAGES.iter().find(|(p, ..)| *p == path) {
        Some((_, kind, text)) => ([(header::CONTENT_TYPE, *kind)], *text).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct Open {
    path: PathBuf,
    #[serde(default)]
    password: Option<String>,
}

fn not_in_web(app: &App, what: &str) -> Result<(), Error> {
    match app.mode {
        Mode::Local { .. } => Ok(()),
        Mode::Web(_) => Err(Error::new(
            ErrorKind::Unsupported,
            format!("{what} is not available in the web interface"),
        )),
    }
}

async fn open(State(app): State<Arc<App>>, bytes: Bytes) -> Answer {
    not_in_web(&app, "opening another repository")?;
    let r: Open = serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
    let path = r.path.clone();
    let repo = blocking(move || match &r.password {
        Some(p) if !p.is_empty() => Repository::unlock(&r.path, p),
        _ => Repository::open(&r.path),
    })
    .await?;
    let info = repo.info();
    *lock(&app.repo) = Some(repo);
    app.remember(&path);
    Ok(axum::Json(
        serde_json::to_value(info).unwrap_or(Value::Null),
    ))
}

async fn init(State(app): State<Arc<App>>, bytes: Bytes) -> Answer {
    not_in_web(&app, "creating a repository")?;
    let r: Open = serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
    let path = r.path.clone();
    let (repo, recovery_key) = blocking(move || match &r.password {
        Some(p) if !p.is_empty() => {
            Repository::init_encrypted(&r.path, p).map(|(repo, key)| (repo, Some(key)))
        }
        _ => Repository::init(&r.path).map(|repo| (repo, None)),
    })
    .await?;
    let info = repo.info();
    *lock(&app.repo) = Some(repo);
    app.remember(&path);
    Ok(axum::Json(
        json!({"info": info, "recovery_key": recovery_key}),
    ))
}

async fn state(State(app): State<Arc<App>>) -> Answer {
    let open = lock(&app.repo).as_ref().map(Repository::info);
    Ok(axum::Json(
        json!({"open": open, "last": app.remembered().last}),
    ))
}

/// The signed-in user in web mode, `None` in local mode.
fn who(app: &App, who: Option<Extension<Who>>) -> Result<Option<Who>, Error> {
    match (&app.mode, who) {
        (Mode::Local { .. }, _) => Ok(None),
        (Mode::Web(_), Some(Extension(who))) => Ok(Some(who)),
        (Mode::Web(_), None) => Err(invalid("sign in first")),
    }
}

/// Runs `f` with the repository and, in web mode, the request after the
/// user's checks (USERS.md §4), recorded in the audit log (AUDIT.md §1): a
/// short operation once it has run, so no result leaves unrecorded, and a
/// long one before it starts.
async fn checked<T: Send + 'static>(
    app: &Arc<App>,
    who: Option<Who>,
    long: bool,
    operation: String,
    request: Value,
    f: impl FnOnce(&Repository, &str, Value) -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let repo = app.repo()?;
    let app = app.clone();
    blocking(move || {
        let (Mode::Web(web), Some(who)) = (&app.mode, &who) else {
            return f(&repo, &operation, request);
        };
        let event = format!("{}:{operation}", if long { "start" } else { "call" });
        let checked = web.check(&repo, &who.user, long, &operation, request.clone());
        let detail = checked.as_ref().map_or(request, Value::clone);
        let checked = match checked {
            Err(e) => {
                web.audit
                    .record(&who.actor(), &event, detail, &outcome(Err(&e)))?;
                return Err(e);
            }
            Ok(checked) => checked,
        };
        if long {
            web.audit.record(&who.actor(), &event, detail, "ok")?;
            let result = f(&repo, &operation, checked);
            if let Err(e) = &result {
                web.audit
                    .record(&who.actor(), &event, Value::Null, &outcome(Err(e)))?;
            }
            result
        } else {
            let result = f(&repo, &operation, checked);
            web.audit.record(
                &who.actor(),
                &event,
                detail,
                &outcome(result.as_ref().map(|_| ())),
            )?;
            result
        }
    })
    .await
}

/// `ok` or the kind of the error, for the audit log.
fn outcome(result: Result<(), &Error>) -> String {
    match result {
        Ok(()) => "ok".to_owned(),
        Err(e) => serde_json::to_value(e.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "error".to_owned()),
    }
}

async fn call(
    State(app): State<Arc<App>>,
    UrlPath(operation): UrlPath<String>,
    who_: Option<Extension<Who>>,
    bytes: Bytes,
) -> Answer {
    let request = body(&bytes)?;
    let who = who(&app, who_)?;
    let result = checked(
        &app,
        who.clone(),
        false,
        operation.clone(),
        request,
        json::call,
    )
    .await?;
    Ok(axum::Json(match (&app.mode, &who) {
        (Mode::Web(web), Some(who)) => web.filter(&who.user, &operation, result),
        _ => result,
    }))
}

async fn start(
    State(app): State<Arc<App>>,
    UrlPath(operation): UrlPath<String>,
    who_: Option<Extension<Who>>,
    bytes: Bytes,
) -> Answer {
    let request = body(&bytes)?;
    let who = who(&app, who_)?;
    let owner = who.as_ref().map(|w| w.user.name.clone());
    let job = checked(&app, who, true, operation.clone(), request, json::start).await?;
    let id = app.next_job.fetch_add(1, Ordering::Relaxed);
    lock(&app.jobs).insert(
        id,
        JobEntry {
            owner,
            operation,
            job,
        },
    );
    Ok(axum::Json(json!({ "job": id })))
}

/// A job of the user, or of anyone in local mode.
fn own_job<'a>(jobs: &'a mut Jobs, id: u64, who: Option<&Who>) -> Result<&'a mut JobEntry, Error> {
    match jobs.get_mut(&id) {
        Some(entry) if entry.owner.as_deref() == who.map(|w| w.user.name.as_str()) => Ok(entry),
        _ => Err(Error::new(ErrorKind::NotFound, format!("job {id}"))),
    }
}

async fn job(
    State(app): State<Arc<App>>,
    UrlPath(id): UrlPath<u64>,
    who_: Option<Extension<Who>>,
) -> Answer {
    let who = who(&app, who_)?;
    let mut jobs = lock(&app.jobs);
    let entry = own_job(&mut jobs, id, who.as_ref())?;
    let progress = entry.job.progress();
    let answer = match entry.job.wait(Some(Duration::ZERO)) {
        None => json!({"progress": progress, "done": false}),
        Some(result) => {
            if let (Mode::Web(web), Some(who)) = (&app.mode, &who) {
                // An unrecorded result stays with the job, to be fetched again.
                let recorded = web.audit.record(
                    &who.actor(),
                    &format!("done:{}", entry.operation),
                    json!({ "job": id }),
                    &outcome(result.as_ref().map(|_| ())),
                );
                if let Err(e) = recorded {
                    return Err(e.into());
                }
            }
            if let (Mode::Web(web), "backup", Ok(_)) =
                (&app.mode, entry.operation.as_str(), &result)
            {
                web.replicate_now();
            }
            jobs.remove(&id);
            match result {
                Ok(result) => json!({"progress": progress, "done": true, "result": result}),
                Err(e) => json!({"progress": progress, "done": true, "error": e}),
            }
        }
    };
    Ok(axum::Json(answer))
}

async fn cancel(
    State(app): State<Arc<App>>,
    UrlPath(id): UrlPath<u64>,
    who_: Option<Extension<Who>>,
) -> Answer {
    let who = who(&app, who_)?;
    let mut jobs = lock(&app.jobs);
    let entry = own_job(&mut jobs, id, who.as_ref())?;
    if let (Mode::Web(web), Some(who)) = (&app.mode, &who) {
        web.audit
            .record(&who.actor(), "cancel", json!({ "job": id }), "ok")?;
    }
    entry.job.cancel();
    Ok(axum::Json(json!({})))
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: usize,
}

fn default_audit_limit() -> usize {
    200
}

/// The newest events of the audit log, newest first, for an `admin`
/// (AUDIT.md §4).
async fn audit(
    State(app): State<Arc<App>>,
    who_: Option<Extension<Who>>,
    Query(q): Query<AuditQuery>,
) -> Answer {
    let who = who(&app, who_)?;
    let events = blocking(move || match (&app.mode, &who) {
        (Mode::Web(web), Some(who)) if who.user.role == Role::Admin => {
            let mut events = web.audit.read(q.limit.min(1000), None)?;
            events.reverse();
            Ok(events)
        }
        _ => Err(Error::new(
            ErrorKind::Unsupported,
            "only an admin of the web interface reads the audit log",
        )),
    })
    .await?;
    Ok(axum::Json(
        serde_json::to_value(events).unwrap_or(Value::Null),
    ))
}

#[derive(Deserialize)]
struct FolderQuery {
    #[serde(default)]
    path: Option<PathBuf>,
}

/// The folders inside a folder, for choosing one (DESKTOP.md §3); in web
/// mode only inside the allowed folders (WEB.md §4).
async fn folders(
    State(app): State<Arc<App>>,
    who_: Option<Extension<Who>>,
    Query(q): Query<FolderQuery>,
) -> Answer {
    let who = who(&app, who_)?;
    let answer = blocking(move || {
        let roots = match (&app.mode, &who) {
            (Mode::Web(web), Some(who)) => Some(web.folders(&who.user)),
            _ => None,
        };
        let path = match (q.path.filter(|p| !p.as_os_str().is_empty()), &roots) {
            (Some(p), None) => p,
            (Some(p), Some(roots)) => web::inside(roots, &p)?,
            (None, Some(roots)) => roots
                .first()
                .cloned()
                .ok_or_else(|| invalid("you have no folders of this server"))?,
            (None, None) => dirs::home_dir().ok_or_else(|| invalid("no home folder"))?,
        };
        let parent = match &roots {
            Some(roots) if roots.contains(&path) => None,
            _ => path.parent().map(Path::to_path_buf),
        };
        let entries = fs::read_dir(&path).map_err(|e| {
            let kind = if e.kind() == io::ErrorKind::NotFound {
                ErrorKind::NotFound
            } else {
                ErrorKind::Io
            };
            Error::new(kind, format!("{}: {e}", path.display()))
        })?;
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        Ok(json!({
            "path": path,
            "parent": parent,
            "folders": names,
        }))
    })
    .await?;
    Ok(axum::Json(answer))
}

/// What a server is started with.
#[derive(Default)]
pub struct Options {
    /// A repository open from the start.
    pub repo: Option<Repository>,
    /// Where the last repository opened is remembered; `None` remembers
    /// nothing.
    pub memory: Option<PathBuf>,
}

impl Options {
    /// Remembers the last repository in the user's configuration folder.
    pub fn with_memory() -> Self {
        Options {
            repo: None,
            memory: dirs::config_dir().map(|d| d.join("apeiron").join("desktop.json")),
        }
    }
}

async fn session(State(app): State<Arc<App>>, user: Option<Extension<Who>>) -> Answer {
    Ok(axum::Json(match (&app.mode, user) {
        (Mode::Local { .. }, _) => json!({"mode": "local", "signed_in": true}),
        (Mode::Web(web), None) => json!({
            "mode": "web",
            "signed_in": false,
            "oidc": web.oidc_config().is_some(),
        }),
        (Mode::Web(_), Some(Extension(who))) => json!({
            "mode": "web",
            "signed_in": true,
            "user": {"name": who.user.name, "role": who.user.role},
        }),
    }))
}

async fn login(State(app): State<Arc<App>>, request: Request) -> Response {
    let from = peer(&request);
    let bytes = match axum::body::to_bytes(request.into_body(), 1 << 16).await {
        Ok(bytes) => bytes,
        Err(e) => return Failure::from(invalid(format!("request: {e}"))).into_response(),
    };
    match &app.mode {
        Mode::Local { .. } => Failure::from(Error::new(
            ErrorKind::Unsupported,
            "the local interface needs no sign-in",
        ))
        .into_response(),
        Mode::Web(web) => web.login(bytes, from).await,
    }
}

async fn logout(State(app): State<Arc<App>>, request: Request) -> Response {
    match &app.mode {
        Mode::Local { .. } => axum::Json(json!({})).into_response(),
        Mode::Web(web) => {
            let who = request.extensions().get::<Who>().cloned();
            web.logout(request.headers(), who.as_ref())
        }
    }
}

/// The web state and the signed-in user; sharing is only for the web
/// interface (SHARE.md §5).
fn web_user(app: &App, who: Option<Extension<Who>>) -> Result<(&WebState, Who), Error> {
    match (&app.mode, who) {
        (Mode::Web(web), Some(Extension(who))) => Ok((web, who)),
        (Mode::Web(_), None) => Err(invalid("sign in first")),
        (Mode::Local { .. }, _) => Err(Error::new(
            ErrorKind::Unsupported,
            "sharing is only for the web interface",
        )),
    }
}

#[derive(Deserialize)]
struct NewShare {
    snapshot: String,
    #[serde(default)]
    path: String,
    #[serde(default = "default_days")]
    days: u32,
}

fn default_days() -> u32 {
    DEFAULT_DAYS
}

/// Makes a share link; its secret is in the answer only (SHARE.md §5).
async fn share_add(
    State(app): State<Arc<App>>,
    who_: Option<Extension<Who>>,
    bytes: Bytes,
) -> Answer {
    let r: NewShare = serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
    let repo = app.repo()?;
    let answer = blocking(move || {
        let (web, who) = web_user(&app, who_)?;
        match web.share(&repo, &who.user, &r.snapshot, &r.path, r.days) {
            Ok((share, secret)) => {
                let detail = json!({
                    "id": share.id,
                    "snapshot": share.snapshot,
                    "path": share.path,
                    "expires": share.expires,
                });
                // A link that cannot be recorded does not stay.
                if let Err(e) = web.audit.record(&who.actor(), "share_add", detail, "ok") {
                    let _ = web.shares.remove(&share.id, |_| true);
                    return Err(e);
                }
                Ok(json!({"share": share.public(), "secret": secret}))
            }
            Err(e) => {
                let detail = json!({"snapshot": r.snapshot, "path": r.path, "days": r.days});
                web.audit
                    .record(&who.actor(), "share_add", detail, &outcome(Err(&e)))?;
                Err(e)
            }
        }
    })
    .await?;
    Ok(axum::Json(answer))
}

/// The user's live links; an `admin` sees everyone's.
async fn share_list(State(app): State<Arc<App>>, who_: Option<Extension<Who>>) -> Answer {
    let shares = blocking(move || {
        let (web, who) = web_user(&app, who_)?;
        Ok(web
            .shares
            .list()?
            .into_iter()
            .filter(|s| who.user.role == Role::Admin || s.owner == who.user.name)
            .map(|s| s.public())
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(axum::Json(
        serde_json::to_value(shares).unwrap_or(Value::Null),
    ))
}

/// Removes a link of the user; an `admin` may remove anyone's.
async fn share_remove(
    State(app): State<Arc<App>>,
    UrlPath(id): UrlPath<String>,
    who_: Option<Extension<Who>>,
) -> Answer {
    blocking(move || {
        let (web, who) = web_user(&app, who_)?;
        let user = &who.user;
        let removed = web
            .shares
            .remove(&id, |s| user.role == Role::Admin || s.owner == user.name);
        web.audit.record(
            &who.actor(),
            "share_remove",
            json!({ "id": id }),
            &outcome(removed.as_ref().map(|_| ())),
        )?;
        removed.map(|_| ())
    })
    .await?;
    Ok(axum::Json(json!({})))
}

#[derive(Deserialize)]
struct SharedQuery {
    #[serde(default)]
    path: String,
}

/// Runs `f` for a request through a share link, recorded with the link's
/// id and the path asked for, without a user (SHARE.md §6).
async fn through_link<T: Send + 'static>(
    app: Arc<App>,
    secret: String,
    from: Option<Extension<ConnectInfo<SocketAddr>>>,
    event: &'static str,
    sub: String,
    f: impl FnOnce(&WebState, &Repository, &Share) -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let actor = Actor {
        user: None,
        from: from.map(|c| c.0.0.to_string()),
    };
    blocking(move || {
        let Mode::Web(web) = &app.mode else {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "sharing is only for the web interface",
            ));
        };
        let repo = app.repo()?;
        let (share, result) = match web.shared(&repo, &secret) {
            Ok(share) => {
                let result = f(web, &repo, &share);
                (Some(share), result)
            }
            Err(e) => (None, Err(e)),
        };
        let detail = json!({"id": share.map(|s| s.id), "path": sub});
        web.audit
            .record(&actor, event, detail, &outcome(result.as_ref().map(|_| ())))?;
        result
    })
    .await
}

/// What a share link shows at a path below it (SHARE.md §4).
async fn shared_list(
    State(app): State<Arc<App>>,
    UrlPath(secret): UrlPath<String>,
    from: Option<Extension<ConnectInfo<SocketAddr>>>,
    Query(q): Query<SharedQuery>,
) -> Answer {
    let sub = q.path.clone();
    let answer = through_link(
        app,
        secret,
        from,
        "shared:list",
        q.path,
        move |web, repo, share| web.shared_list(repo, share, &sub),
    )
    .await?;
    Ok(axum::Json(answer))
}

/// Writes into a channel, for a body read as it is written.
struct ToChannel(tokio::sync::mpsc::Sender<Result<Bytes, io::Error>>);

impl io::Write for ToChannel {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .blocking_send(Ok(Bytes::copy_from_slice(buf)))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A file below a share link, sent as it is read (SHARE.md §4).
async fn shared_file(
    State(app): State<Arc<App>>,
    UrlPath(secret): UrlPath<String>,
    from: Option<Extension<ConnectInfo<SocketAddr>>>,
    Query(q): Query<SharedQuery>,
) -> Result<Response, Failure> {
    let sub = q.path.clone();
    let (repo, snapshot, path, size) = through_link(
        app,
        secret,
        from,
        "shared:read",
        q.path,
        move |web, repo, share| {
            let (path, size) = web.shared_file(repo, share, &sub)?;
            Ok((repo.clone(), share.snapshot, path, size))
        },
    )
    .await?;
    let name = path.rsplit('/').next().unwrap_or("file").to_owned();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::task::spawn_blocking(move || {
        let mut out = io::BufWriter::with_capacity(1 << 18, ToChannel(tx.clone()));
        let request = ListRequest {
            snapshot: snapshot.to_string(),
            path: path.as_str().into(),
        };
        // An error after the first bytes can only cut the download short.
        if let Err(e) = repo.read_file(&request, &mut out) {
            let _ = tx.blocking_send(Err(io::Error::other(e.to_string())));
        }
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    let disposition = format!("attachment; filename*=UTF-8''{}", percent(&name));
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (header::CONTENT_LENGTH, size.to_string()),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response())
}

/// `text` with every byte but letters, digits and `-._~` as `%XX`.
fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Sends the browser to the identity provider (IDENTITY.md §3).
async fn oidc_start(State(app): State<Arc<App>>) -> Response {
    match &app.mode {
        Mode::Web(web) => web.oidc_start().await,
        Mode::Local { .. } => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Where the identity provider sends the browser back.
async fn oidc_callback(
    State(app): State<Arc<App>>,
    Query(query): Query<HashMap<String, String>>,
    from: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let from = from.map(|c| c.0.0.to_string());
    match &app.mode {
        Mode::Web(web) => web.oidc_callback(&headers, &query, from).await,
        Mode::Local { .. } => StatusCode::NOT_FOUND.into_response(),
    }
}

/// For a load balancer or a monitor: whether the repository can be read
/// and whether this is a standby, without signing in (REPLICATION.md §5).
async fn health(State(app): State<Arc<App>>) -> Response {
    let standby = match &app.mode {
        Mode::Web(web) => web.standby(),
        Mode::Local { .. } => false,
    };
    let role = if standby { "standby" } else { "primary" };
    let app = app.clone();
    match blocking(move || app.repo()?.refs()).await {
        Ok(_) => axum::Json(json!({"ok": true, "role": role})).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"ok": false, "role": role})),
        )
            .into_response(),
    }
}

fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/api/session", get(session))
        .route("/api/health", get(health))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/oidc/start", get(oidc_start))
        .route("/api/oidc/callback", get(oidc_callback))
        .route("/api/state", get(state))
        .route("/api/open", post(open))
        .route("/api/init", post(init))
        .route("/api/call/{operation}", post(call))
        .route("/api/start/{operation}", post(start))
        .route("/api/jobs/{id}", get(job))
        .route("/api/jobs/{id}/cancel", post(cancel))
        .route("/api/folders", get(folders))
        .route("/api/audit", get(audit))
        .route("/api/shares", get(share_list).post(share_add))
        .route("/api/shares/{id}/remove", post(share_remove))
        .route("/api/shared/{secret}", get(shared_list))
        .route("/api/shared/{secret}/file", get(shared_file))
        .fallback(page)
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

/// The host name a `Host` header names, without the port, lowercase.
fn host_name(host: &str) -> String {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => host.split(':').next().unwrap_or_default(),
    };
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// Sends each request to the site of the host its `Host` header names; no
/// site answers for other hosts (TENANTS.md §3).
fn by_host(sites: HashMap<String, Router>) -> Router {
    let sites = Arc::new(sites);
    Router::new().fallback(move |request: Request| {
        let sites = sites.clone();
        async move {
            let site = request
                .headers()
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .and_then(|h| sites.get(&host_name(h)));
            match site {
                Some(site) => match site.clone().oneshot(request).await {
                    Ok(response) => response,
                    Err(never) => match never {},
                },
                None => refused(
                    StatusCode::MISDIRECTED_REQUEST,
                    "this server has no site at this address",
                ),
            }
        }
    })
}

/// A server running on a thread of its own.
struct Running {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Running {
    /// Serves `router` on `listener`, over TLS when `tls` is given.
    fn start(
        listener: TcpListener,
        router: Router,
        tls: Option<Arc<rustls::ServerConfig>>,
    ) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_io()
            .enable_time()
            .thread_name("apeiron-server")
            .build()?;
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let listener = runtime.block_on(async { tokio::net::TcpListener::from_std(listener) })?;
        let thread = std::thread::Builder::new()
            .name("apeiron-server".to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    match tls {
                        None => {
                            let serve = axum::serve(
                                listener,
                                router.into_make_service_with_connect_info::<SocketAddr>(),
                            )
                            .with_graceful_shutdown(async {
                                let _ = stopped.await;
                            });
                            // The server ends only when stopped or when its socket fails.
                            let _ = serve.await;
                        }
                        Some(config) => {
                            tokio::select! {
                                () = serve_tls(listener, router, config) => {}
                                _ = stopped => {}
                            }
                        }
                    }
                });
            })?;
        Ok(Running {
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    fn wait(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How long a client may take to finish the TLS handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// Accepts TLS connections and serves each on a task of its own.
async fn serve_tls(
    listener: tokio::net::TcpListener,
    router: Router,
    config: Arc<rustls::ServerConfig>,
) {
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    loop {
        let Ok((tcp, address)) = listener.accept().await else {
            // Running out of file handles passes; wait instead of spinning.
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let acceptor = acceptor.clone();
        let service = hyper_util::service::TowerToHyperService::new(
            router.clone().layer(Extension(ConnectInfo(address))),
        );
        tokio::spawn(async move {
            let Ok(Ok(stream)) = tokio::time::timeout(HANDSHAKE, acceptor.accept(tcp)).await else {
                return;
            };
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
        });
    }
}

/// A running local server.
pub struct Local {
    url: String,
    running: Running,
}

impl Local {
    /// Starts a server on a free port of `127.0.0.1` with a new token, on a
    /// thread of its own.
    pub fn start(options: Options) -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        listener.set_nonblocking(true)?;
        let host = listener.local_addr()?.to_string();
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|e| io::Error::other(e.to_string()))?;
        let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let url = format!("http://{host}/#token={token}");
        let app = Arc::new(App {
            mode: Mode::Local { token, host },
            repo: Mutex::new(options.repo),
            jobs: Mutex::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            memory: options.memory,
        });
        let running = Running::start(listener, router(app), None)?;
        Ok(Local { url, running })
    }

    /// The address of the pages, with the token in the fragment.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Blocks until the server ends.
    pub fn wait(self) {
        self.running.wait();
    }
}
