//! The local server of Apeiron (DESKTOP.md): serves the pages of the
//! desktop interface and a JSON API, the C ABI over HTTP, only on
//! `127.0.0.1` and only to holders of a token.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

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

struct App {
    token: String,
    /// `127.0.0.1:<port>`, the only `Host` accepted.
    host: String,
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

/// Refuses requests from elsewhere than the pages (DESKTOP.md §4).
async fn guard(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let header_is =
        |name, value: &str| headers.get(name).and_then(|v| v.to_str().ok()) == Some(value);
    let origin = format!("http://{}", app.host);
    if !header_is(header::HOST, &app.host)
        || headers.contains_key(header::ORIGIN) && !header_is(header::ORIGIN, &origin)
    {
        return Failure(
            StatusCode::FORBIDDEN,
            json!({"code": 4, "kind": "invalid_input", "message": "request from another site"}),
        )
        .into_response();
    }
    if request.uri().path().starts_with("/api/")
        && !header_is(header::AUTHORIZATION, &format!("Bearer {}", app.token))
    {
        return Failure(
            StatusCode::UNAUTHORIZED,
            json!({"code": 4, "kind": "invalid_input", "message": "missing or wrong token; open the interface again"}),
        )
        .into_response();
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

async fn open(State(app): State<Arc<App>>, bytes: Bytes) -> Answer {
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
    let request = body(&bytes)?;
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

/// The folders inside a folder, for choosing one (DESKTOP.md §3).
async fn folders(Query(q): Query<FolderQuery>) -> Answer {
    let answer = blocking(move || {
        let path = match q.path.filter(|p| !p.as_os_str().is_empty()) {
            Some(p) => p,
            None => dirs::home_dir().ok_or_else(|| invalid("no home folder"))?,
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
            "parent": path.parent(),
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

fn router(app: Arc<App>) -> Router {
    Router::new()
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

/// A running local server.
pub struct Local {
    url: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
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
            token,
            host,
            repo: Mutex::new(options.repo),
            jobs: Mutex::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            memory: options.memory,
        });
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_io()
            .thread_name("apeiron-server")
            .build()?;
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let listener = runtime.block_on(async { tokio::net::TcpListener::from_std(listener) })?;
        let thread = std::thread::Builder::new()
            .name("apeiron-server".to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    let serve = axum::serve(listener, router(app)).with_graceful_shutdown(async {
                        let _ = stopped.await;
                    });
                    // The server ends only when stopped or when its socket fails.
                    let _ = serve.await;
                });
            })?;
        Ok(Local {
            url,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// The address of the pages, with the token in the fragment.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Blocks until the server ends.
    pub fn wait(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
