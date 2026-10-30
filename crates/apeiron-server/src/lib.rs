//! The server of Apeiron's pages and JSON API, the C ABI over HTTP. In local
//! mode (DESKTOP.md) it listens only on `127.0.0.1` for holders of a token;
//! in web mode (WEB.md) it serves one repository to the network, over TLS,
//! to a signed-in browser.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

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

use web::WebState;
pub use web::{Tls, Web, WebOptions, hash_password, load_password, save_password};

use apeiron::json::{self, JsonJob};
use apeiron::{Error, ErrorKind, Repository};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

struct App {
    mode: Mode,
    repo: Mutex<Option<Repository>>,
    jobs: Mutex<HashMap<u64, Box<dyn JsonJob>>>,
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
async fn guard(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
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
            let open = matches!(request.uri().path(), "/api/session" | "/api/login");
            if api && !open && !web.signed_in(headers) {
                return Failure(
                    StatusCode::UNAUTHORIZED,
                    json!({"code": 4, "kind": "invalid_input", "message": "sign in first", "sign_in": true}),
                )
                .into_response();
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
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn page(request: Request) -> Response {
    let path = request.uri().path();
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

async fn call(
    State(app): State<Arc<App>>,
    UrlPath(operation): UrlPath<String>,
    bytes: Bytes,
) -> Answer {
    let request = body(&bytes)?;
    if let Mode::Web(web) = &app.mode {
        web.allow_call(&operation)?;
    }
    let repo = app.repo()?;
    Ok(axum::Json(
        blocking(move || json::call(&repo, &operation, request)).await?,
    ))
}

async fn start(
    State(app): State<Arc<App>>,
    UrlPath(operation): UrlPath<String>,
    bytes: Bytes,
) -> Answer {
    let mut request = body(&bytes)?;
    if let Mode::Web(web) = &app.mode {
        let web_request = request;
        request = blocking({
            let folders = web.folders.clone();
            let operation = operation.clone();
            move || web::allow_start(&folders, &operation, web_request)
        })
        .await?;
    }
    let repo = app.repo()?;
    let job = blocking(move || json::start(&repo, &operation, request)).await?;
    let id = app.next_job.fetch_add(1, Ordering::Relaxed);
    lock(&app.jobs).insert(id, job);
    Ok(axum::Json(json!({ "job": id })))
}

async fn job(State(app): State<Arc<App>>, UrlPath(id): UrlPath<u64>) -> Answer {
    let mut jobs = lock(&app.jobs);
    let job = jobs
        .get_mut(&id)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("job {id}")))?;
    let progress = job.progress();
    let answer = match job.wait(Some(Duration::ZERO)) {
        None => json!({"progress": progress, "done": false}),
        Some(result) => {
            jobs.remove(&id);
            match result {
                Ok(result) => json!({"progress": progress, "done": true, "result": result}),
                Err(e) => json!({"progress": progress, "done": true, "error": e}),
            }
        }
    };
    Ok(axum::Json(answer))
}

async fn cancel(State(app): State<Arc<App>>, UrlPath(id): UrlPath<u64>) -> Answer {
    let jobs = lock(&app.jobs);
    let job = jobs
        .get(&id)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("job {id}")))?;
    job.cancel();
    Ok(axum::Json(json!({})))
}

#[derive(Deserialize)]
struct FolderQuery {
    #[serde(default)]
    path: Option<PathBuf>,
}

/// The folders inside a folder, for choosing one (DESKTOP.md §3); in web
/// mode only inside the allowed folders (WEB.md §4).
async fn folders(State(app): State<Arc<App>>, Query(q): Query<FolderQuery>) -> Answer {
    let roots = match &app.mode {
        Mode::Local { .. } => None,
        Mode::Web(web) => Some(web.folders.clone()),
    };
    let answer = blocking(move || {
        let path = match (q.path.filter(|p| !p.as_os_str().is_empty()), &roots) {
            (Some(p), None) => p,
            (Some(p), Some(roots)) => web::inside(roots, &p)?,
            (None, Some(roots)) => roots
                .first()
                .cloned()
                .ok_or_else(|| invalid("no folder is allowed"))?,
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

async fn session(State(app): State<Arc<App>>, request: Request) -> Answer {
    Ok(axum::Json(match &app.mode {
        Mode::Local { .. } => json!({"mode": "local", "signed_in": true}),
        Mode::Web(web) => json!({"mode": "web", "signed_in": web.signed_in(request.headers())}),
    }))
}

async fn login(State(app): State<Arc<App>>, bytes: Bytes) -> Response {
    match &app.mode {
        Mode::Local { .. } => Failure::from(Error::new(
            ErrorKind::Unsupported,
            "the local interface needs no sign-in",
        ))
        .into_response(),
        Mode::Web(web) => web.login(bytes).await,
    }
}

async fn logout(State(app): State<Arc<App>>, request: Request) -> Response {
    match &app.mode {
        Mode::Local { .. } => axum::Json(json!({})).into_response(),
        Mode::Web(web) => web.logout(request.headers()),
    }
}

fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/api/session", get(session))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/state", get(state))
        .route("/api/open", post(open))
        .route("/api/init", post(init))
        .route("/api/call/{operation}", post(call))
        .route("/api/start/{operation}", post(start))
        .route("/api/jobs/{id}", get(job))
        .route("/api/jobs/{id}/cancel", post(cancel))
        .route("/api/folders", get(folders))
        .fallback(page)
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

/// A server running on a thread of its own.
struct Running {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Running {
    /// Serves `app` on `listener`, over TLS when `tls` is given.
    fn start(
        listener: TcpListener,
        app: Arc<App>,
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
                    let router = router(app);
                    match tls {
                        None => {
                            let serve =
                                axum::serve(listener, router).with_graceful_shutdown(async {
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
        let Ok((tcp, _)) = listener.accept().await else {
            // Running out of file handles passes; wait instead of spinning.
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let acceptor = acceptor.clone();
        let service = hyper_util::service::TowerToHyperService::new(router.clone());
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
        let running = Running::start(listener, app, None)?;
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
