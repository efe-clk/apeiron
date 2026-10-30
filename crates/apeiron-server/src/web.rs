//! Web mode (WEB.md): one repository served to the network over TLS, to
//! signed-in users, with only the operations, backups and folders their
//! role and rights allow (USERS.md).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use apeiron::{Error, ErrorKind, LogRequest, ObjectId, Repository};
use argon2::Argon2;
use argon2::password_hash::PasswordVerifier;
use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::audit::{Actor, Audit};
use crate::users::{Role, User, Users, hash_password};
use crate::{App, Failure, Mode, Running, blocking, body, invalid, lock};

const COOKIE: &str = "apeiron_session";
/// A session ends after this long without a request (WEB.md §5).
const IDLE: Duration = Duration::from_secs(12 * 60 * 60);
/// A session ends after this long in any case.
const LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Wrong passwords in a row before sign-ins pause.
const TRIES: u32 = 5;
const PAUSE: Duration = Duration::from_secs(60);

/// The operations the pages use (WEB.md §4).
const CALLS: &[&str] = &[
    "info",
    "resolve",
    "log",
    "list",
    "diff",
    "diff_text",
    "refs",
    "reflog",
];
const STARTS: &[&str] = &["backup", "restore", "verify", "search"];

/// A certificate chain and its private key, as PEM.
#[derive(Clone)]
pub struct Tls {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
}

impl Tls {
    /// A new self-signed certificate for `names` (host names or addresses).
    pub fn self_signed(names: Vec<String>) -> Result<Tls, Error> {
        let made = rcgen::generate_simple_self_signed(names)
            .map_err(|e| Error::new(ErrorKind::Internal, format!("certificate: {e}")))?;
        Ok(Tls {
            cert_pem: made.cert.pem().into_bytes(),
            key_pem: made.signing_key.serialize_pem().into_bytes(),
        })
    }

    /// The certificate kept in `dir` (`web-cert.pem`, `web-key.pem`), made
    /// for `names` the first time (WEB.md §2).
    pub fn kept_in(dir: &Path, names: Vec<String>) -> Result<Tls, Error> {
        let (cert, key) = (dir.join("web-cert.pem"), dir.join("web-key.pem"));
        let io_error = |path: &Path, e: io::Error| {
            Error::new(ErrorKind::Io, format!("{}: {e}", path.display()))
        };
        if cert.exists() && key.exists() {
            return Ok(Tls {
                cert_pem: fs::read(&cert).map_err(|e| io_error(&cert, e))?,
                key_pem: fs::read(&key).map_err(|e| io_error(&key, e))?,
            });
        }
        let tls = Tls::self_signed(names)?;
        fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
        write_private(&key, &tls.key_pem).map_err(|e| io_error(&key, e))?;
        fs::write(&cert, &tls.cert_pem).map_err(|e| io_error(&cert, e))?;
        Ok(tls)
    }

    /// The SHA-256 fingerprint of the first certificate, as browsers show
    /// it: uppercase hex pairs joined by colons.
    pub fn fingerprint(&self) -> Result<String, Error> {
        let cert =
            self.certificates()?.into_iter().next().ok_or_else(|| {
                Error::new(ErrorKind::InvalidInput, "no certificate in the PEM file")
            })?;
        let digest = Sha256::digest(cert.as_ref());
        Ok(digest
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":"))
    }

    fn certificates(&self) -> Result<Vec<CertificateDer<'static>>, Error> {
        CertificateDer::pem_slice_iter(&self.cert_pem)
            .collect::<Result<_, _>>()
            .map_err(|e| invalid(format!("certificate: {e}")))
    }

    fn config(&self) -> Result<Arc<rustls::ServerConfig>, Error> {
        let key = PrivateKeyDer::from_pem_slice(&self.key_pem)
            .map_err(|e| invalid(format!("private key: {e}")))?;
        let certificates = self.certificates()?;
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .and_then(|b| b.with_no_client_auth().with_single_cert(certificates, key))
            .map_err(|e| invalid(format!("certificate: {e}")))?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }
}

/// Writes a file only its owner can read.
pub(crate) fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    io::Write::write_all(&mut options.open(path)?, data)
}

/// What a web server is started with (WEB.md §2, USERS.md).
pub struct WebOptions {
    /// The repository served.
    pub repo: Repository,
    pub listen: SocketAddr,
    /// `None` serves plain HTTP, for use behind a TLS proxy.
    pub tls: Option<Tls>,
    /// The users file, read again whenever it changes.
    pub users: PathBuf,
    /// The folders users may be given.
    pub folders: Vec<PathBuf>,
    /// Where every request is recorded (AUDIT.md).
    pub audit: Audit,
}

struct Session {
    user: String,
    /// The password hash at sign-in; a new password ends the session.
    password: String,
    made: Instant,
    used: Instant,
}

#[derive(Default)]
struct Attempts {
    wrong: u32,
    paused_until: Option<Instant>,
}

/// The users file as last read.
struct Loaded {
    modified: Option<SystemTime>,
    users: Users,
}

/// The signed-in user of a request and the address of the connection.
#[derive(Clone)]
pub(crate) struct Who {
    pub(crate) user: Arc<User>,
    pub(crate) from: Option<String>,
}

impl Who {
    pub(crate) fn actor(&self) -> Actor {
        Actor {
            user: Some(self.user.name.clone()),
            from: self.from.clone(),
        }
    }
}

/// Ref heads whose reachable snapshots are kept, at most.
const KEPT_HEADS: usize = 256;

pub(crate) struct WebState {
    pub(crate) audit: Audit,
    users_file: PathBuf,
    users: Mutex<Loaded>,
    /// Canonical `--folder` paths.
    roots: Vec<PathBuf>,
    sessions: Mutex<HashMap<String, Session>>,
    /// Held while a password is checked, so guesses go one at a time.
    attempts: tokio::sync::Mutex<Attempts>,
    /// The snapshots reachable from a ref head, by head.
    reachable: Mutex<HashMap<ObjectId, Arc<HashSet<ObjectId>>>>,
    /// Checked when the name is unknown, so that takes as long as a wrong
    /// password.
    decoy: String,
}

fn cookie_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(COOKIE)?.strip_prefix('='))
}

fn random_hex() -> Result<String, Error> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}

fn with_cookie(value: Value, cookie: &str) -> Response {
    let mut response = axum::Json(value).into_response();
    if let Ok(v) = HeaderValue::from_str(cookie) {
        response.headers_mut().insert(header::SET_COOKIE, v);
    }
    response
}

#[derive(Deserialize)]
struct Login {
    name: String,
    password: String,
}

/// The fields of a request that name a snapshot (USERS.md §4).
const SNAPSHOT_FIELDS: &[&str] = &["spec", "start", "snapshot", "from", "to"];

fn hidden() -> Error {
    Error::new(ErrorKind::NotFound, "no such snapshot among your backups")
}

impl WebState {
    /// The user named `name` in the users file as it is now.
    fn user(&self, name: &str) -> Option<User> {
        let modified = fs::metadata(&self.users_file)
            .and_then(|m| m.modified())
            .ok();
        let mut loaded = lock(&self.users);
        if modified != loaded.modified {
            // A file being written may not parse; the last good one stays.
            if let Ok(users) = Users::load(&self.users_file) {
                *loaded = Loaded { modified, users };
            }
        }
        loaded.users.get(name).cloned()
    }

    /// The user of the request's live session; using it keeps it alive.
    pub(crate) fn current(&self, headers: &HeaderMap) -> Option<User> {
        let id = cookie_value(headers)?;
        let (name, password) = {
            let mut sessions = lock(&self.sessions);
            let now = Instant::now();
            match sessions.get_mut(id) {
                Some(s) if now - s.used < IDLE && now - s.made < LIFETIME => {
                    s.used = now;
                    (s.user.clone(), s.password.clone())
                }
                Some(_) => {
                    sessions.remove(id);
                    return None;
                }
                None => return None,
            }
        };
        match self.user(&name) {
            Some(user) if user.password == password => Some(user),
            _ => {
                lock(&self.sessions).remove(id);
                None
            }
        }
    }

    pub(crate) async fn login(&self, bytes: Bytes, from: Option<String>) -> Response {
        match self.try_login(bytes, from).await {
            Ok(response) => response,
            Err(f) => f.into_response(),
        }
    }

    async fn try_login(&self, bytes: Bytes, from: Option<String>) -> Result<Response, Failure> {
        let r: Login = serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
        let name = r.name.clone();
        // Only a sign-in that worked names the user; a failed one keeps the
        // name given in `detail` (AUDIT.md §2).
        let record = |user: Option<&str>, outcome: &str| {
            let actor = Actor {
                user: user.map(str::to_owned),
                from: from.clone(),
            };
            self.audit
                .record(&actor, "sign_in", json!({ "name": name }), outcome)
        };
        let mut attempts = self.attempts.lock().await;
        let now = Instant::now();
        if attempts.paused_until.is_some_and(|until| now < until) {
            record(None, "locked")?;
            return Err(Failure(
                StatusCode::TOO_MANY_REQUESTS,
                json!({"code": 7, "kind": "locked", "message": "too many wrong passwords; try again in a minute"}),
            ));
        }
        let user = self.user(&r.name);
        let decoy = self.decoy.clone();
        let user = blocking(move || {
            Ok(match user {
                Some(user) if user.verify(&r.password) => Some(user),
                Some(_) => None,
                None => {
                    let _ =
                        Argon2::default().verify_password(r.password.as_bytes(), decoy.as_str());
                    None
                }
            })
        })
        .await?;
        let Some(user) = user else {
            record(None, "key")?;
            attempts.wrong += 1;
            if attempts.wrong >= TRIES {
                attempts.wrong = 0;
                attempts.paused_until = Some(Instant::now() + PAUSE);
            }
            return Err(Error::new(ErrorKind::Key, "wrong name or password").into());
        };
        record(Some(&user.name), "ok")?;
        *attempts = Attempts::default();
        let id = random_hex()?;
        let mut sessions = lock(&self.sessions);
        sessions.retain(|_, s| now - s.used < IDLE && now - s.made < LIFETIME);
        sessions.insert(
            id.clone(),
            Session {
                user: user.name,
                password: user.password,
                made: now,
                used: now,
            },
        );
        Ok(with_cookie(
            json!({}),
            &format!("{COOKIE}={id}; HttpOnly; Secure; SameSite=Strict; Path=/"),
        ))
    }

    pub(crate) fn logout(&self, headers: &HeaderMap, who: Option<&Who>) -> Response {
        if let Some(who) = who
            && let Err(e) = self
                .audit
                .record(&who.actor(), "sign_out", Value::Null, "ok")
        {
            return Failure::from(e).into_response();
        }
        if let Some(id) = cookie_value(headers) {
            lock(&self.sessions).remove(id);
        }
        with_cookie(
            json!({}),
            &format!("{COOKIE}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"),
        )
    }

    /// The folders `user` may use, canonical (USERS.md §1).
    pub(crate) fn folders(&self, user: &User) -> Vec<PathBuf> {
        if user.folders.is_empty() {
            return match user.role {
                Role::Admin => self.roots.clone(),
                _ => Vec::new(),
            };
        }
        user.folders
            .iter()
            .filter_map(|f| f.canonicalize().ok())
            .filter(|f| self.roots.iter().any(|r| f.starts_with(r)))
            .collect()
    }

    /// Checks an operation and its request for `user` (WEB.md §4, USERS.md
    /// §4). Snapshots and paths are replaced by what they resolve to, so
    /// what was checked is what is used.
    pub(crate) fn check(
        &self,
        repo: &Repository,
        user: &User,
        long: bool,
        operation: &str,
        mut request: Value,
    ) -> Result<Value, Error> {
        let known = if long { STARTS } else { CALLS };
        if !known.contains(&operation) {
            return Err(not_here(operation));
        }
        let allowed = match operation {
            "backup" | "restore" => user.role != Role::Reader,
            "verify" => user.role == Role::Admin,
            _ => true,
        };
        if !allowed {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!(
                    "{operation} is not allowed for the role {}",
                    user.role.name()
                ),
            ));
        }
        let text = |request: &Value, field: &str| {
            request
                .get(field)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        if user.role != Role::Admin {
            for field in SNAPSHOT_FIELDS {
                if let Some(spec) = text(&request, field) {
                    let id = repo.resolve(&spec).map_err(|e| match e.kind {
                        ErrorKind::NotFound => hidden(),
                        _ => e,
                    })?;
                    self.visible(repo, user, id)?;
                    request[*field] = json!(id);
                }
            }
            if operation == "search" && text(&request, "snapshot").is_none() {
                return Err(invalid("name the snapshot to search"));
            }
            if operation == "reflog" && !text(&request, "name").is_some_and(|n| user.owns(&n)) {
                return Err(hidden());
            }
            if operation == "backup" && !text(&request, "ref").is_some_and(|n| user.owns(&n)) {
                return Err(invalid(format!(
                    "you may back up only into {}",
                    user.backups.join(", ")
                )));
            }
        }
        let field = match operation {
            "backup" => "source",
            "restore" => "target",
            _ => return Ok(request),
        };
        let path = text(&request, field).ok_or_else(|| invalid(format!("{field} is missing")))?;
        let resolved = inside(&self.folders(user), Path::new(&path))?;
        request[field] = json!(resolved);
        Ok(request)
    }

    /// Leaves out of a result what `user` may not see.
    pub(crate) fn filter(&self, user: &User, operation: &str, result: Value) -> Value {
        match (operation, result) {
            ("refs", Value::Array(refs)) => Value::Array(
                refs.into_iter()
                    .filter(|r| r["name"].as_str().is_some_and(|n| user.owns(n)))
                    .collect(),
            ),
            (_, result) => result,
        }
    }

    /// Whether a snapshot can be reached from one of `user`'s backups
    /// through its parents (USERS.md §1).
    fn visible(&self, repo: &Repository, user: &User, id: ObjectId) -> Result<(), Error> {
        for r in repo.refs()? {
            if user.owns(&r.name) && self.reachable(repo, r.snapshot)?.contains(&id) {
                return Ok(());
            }
        }
        Err(hidden())
    }

    fn reachable(
        &self,
        repo: &Repository,
        head: ObjectId,
    ) -> Result<Arc<HashSet<ObjectId>>, Error> {
        if let Some(known) = lock(&self.reachable).get(&head) {
            return Ok(known.clone());
        }
        let mut seen = HashSet::new();
        let mut todo = vec![head];
        while let Some(start) = todo.pop() {
            let mut next = Some(start);
            'chain: while let Some(from) = next.take() {
                let page = repo.log(&LogRequest {
                    start: from.to_string(),
                    limit: 256,
                })?;
                for s in page.snapshots {
                    if !seen.insert(s.id) {
                        break 'chain;
                    }
                    todo.extend(s.parents.iter().skip(1).copied());
                }
                next = page.next;
            }
        }
        let seen = Arc::new(seen);
        let mut kept = lock(&self.reachable);
        if kept.len() >= KEPT_HEADS {
            kept.clear();
        }
        kept.insert(head, seen.clone());
        Ok(seen)
    }
}

fn not_here(operation: &str) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        format!("{operation} is not available in the web interface"),
    )
}

/// `path` resolved, if it is inside one of `folders` (canonical paths); a
/// path that does not exist yet is resolved through its nearest existing
/// parent (WEB.md §5).
pub(crate) fn inside(folders: &[PathBuf], path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute() {
        return Err(invalid(format!("{} is not a full path", path.display())));
    }
    let mut existing = path;
    let mut rest = Vec::new();
    let resolved = loop {
        match existing.canonicalize() {
            Ok(p) => break p,
            Err(_) => {
                // A missing `..` has no file name, so nothing is resolved
                // after the check.
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
                    return Err(invalid(format!("{} cannot be resolved", path.display())));
                };
                rest.push(name.to_owned());
                existing = parent;
            }
        }
    };
    let full = rest.iter().rev().fold(resolved, |p, name| p.join(name));
    if folders.iter().any(|f| full.starts_with(f)) {
        Ok(full)
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "{} is outside the folders this server may use",
                path.display()
            ),
        ))
    }
}

/// A running web server.
pub struct Web {
    address: SocketAddr,
    running: Running,
}

impl Web {
    /// Starts a web server for one repository on a thread of its own.
    pub fn start(options: WebOptions) -> Result<Self, Error> {
        let io_error = |e: io::Error| Error::new(ErrorKind::Io, e.to_string());
        let mut roots = Vec::new();
        for folder in &options.folders {
            roots.push(folder.canonicalize().map_err(|e| {
                Error::new(ErrorKind::NotFound, format!("{}: {e}", folder.display()))
            })?);
        }
        Users::load(&options.users)?.need_admin()?;
        let tls = options.tls.as_ref().map(Tls::config).transpose()?;
        let listener = std::net::TcpListener::bind(options.listen).map_err(|e| {
            Error::new(
                ErrorKind::Io,
                format!("cannot listen on {}: {e}", options.listen),
            )
        })?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let address = listener.local_addr().map_err(io_error)?;
        let app = Arc::new(App {
            mode: Mode::Web(Box::new(WebState {
                audit: options.audit,
                users_file: options.users,
                users: Mutex::new(Loaded {
                    modified: None,
                    users: Users::default(),
                }),
                roots,
                sessions: Mutex::new(HashMap::new()),
                attempts: tokio::sync::Mutex::new(Attempts::default()),
                reachable: Mutex::new(HashMap::new()),
                decoy: hash_password("not the password of anyone")?,
            })),
            repo: Mutex::new(Some(options.repo)),
            jobs: Mutex::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            memory: None,
        });
        let running = Running::start(listener, app, tls).map_err(io_error)?;
        Ok(Web { address, running })
    }

    /// The address the server listens on.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Blocks until the server ends.
    pub fn wait(self) {
        self.running.wait();
    }
}
