//! Web mode (WEB.md): one repository served to the network over TLS, to a
//! browser signed in with a password, with only the operations and folders
//! the pages need.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apeiron::{Error, ErrorKind, Repository};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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
fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    io::Write::write_all(&mut options.open(path)?, data)
}

/// Hashes a sign-in password with Argon2id, as a PHC string.
pub fn hash_password(password: &str) -> Result<String, Error> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| Error::new(ErrorKind::Internal, format!("password hash: {e}")))
}

#[derive(Serialize, Deserialize)]
struct PasswordFile {
    password: String,
}

/// The stored hash of the sign-in password, if `file` exists (WEB.md §2).
pub fn load_password(file: &Path) -> Result<Option<String>, Error> {
    match fs::read(file) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::new(
            ErrorKind::Io,
            format!("{}: {e}", file.display()),
        )),
        Ok(data) => serde_json::from_slice::<PasswordFile>(&data)
            .map(|f| Some(f.password))
            .map_err(|e| invalid(format!("{}: {e}", file.display()))),
    }
}

/// Stores the hash of the sign-in password in `file`, readable only by its
/// owner.
pub fn save_password(file: &Path, hash: &str) -> Result<(), Error> {
    let data = serde_json::to_vec_pretty(&PasswordFile {
        password: hash.to_owned(),
    })
    .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
    file.parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| write_private(file, &data))
        .map_err(|e| Error::new(ErrorKind::Io, format!("{}: {e}", file.display())))
}

/// What a web server is started with (WEB.md §2).
pub struct WebOptions {
    /// The repository served.
    pub repo: Repository,
    pub listen: SocketAddr,
    /// `None` serves plain HTTP, for use behind a TLS proxy.
    pub tls: Option<Tls>,
    /// The Argon2id hash of the sign-in password.
    pub password_hash: String,
    /// The folders that may be backed up or restored into.
    pub folders: Vec<PathBuf>,
}

struct Session {
    made: Instant,
    used: Instant,
}

#[derive(Default)]
struct Attempts {
    wrong: u32,
    paused_until: Option<Instant>,
}

pub(crate) struct WebState {
    password: String,
    /// Canonical paths.
    pub(crate) folders: Vec<PathBuf>,
    sessions: Mutex<HashMap<String, Session>>,
    /// Held while a password is checked, so guesses go one at a time.
    attempts: tokio::sync::Mutex<Attempts>,
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
    password: String,
}

impl WebState {
    /// Whether the request carries a live session; using it keeps it alive.
    pub(crate) fn signed_in(&self, headers: &HeaderMap) -> bool {
        let Some(id) = cookie_value(headers) else {
            return false;
        };
        let mut sessions = lock(&self.sessions);
        let now = Instant::now();
        match sessions.get_mut(id) {
            Some(s) if now - s.used < IDLE && now - s.made < LIFETIME => {
                s.used = now;
                true
            }
            Some(_) => {
                sessions.remove(id);
                false
            }
            None => false,
        }
    }

    pub(crate) async fn login(&self, bytes: Bytes) -> Response {
        match self.try_login(bytes).await {
            Ok(response) => response,
            Err(f) => f.into_response(),
        }
    }

    async fn try_login(&self, bytes: Bytes) -> Result<Response, Failure> {
        let r: Login = serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
        let mut attempts = self.attempts.lock().await;
        let now = Instant::now();
        if attempts.paused_until.is_some_and(|until| now < until) {
            return Err(Failure(
                StatusCode::TOO_MANY_REQUESTS,
                json!({"code": 7, "kind": "locked", "message": "too many wrong passwords; try again in a minute"}),
            ));
        }
        let hash = self.password.clone();
        let right = blocking(move || {
            Ok(Argon2::default()
                .verify_password(r.password.as_bytes(), hash.as_str())
                .is_ok())
        })
        .await?;
        if !right {
            attempts.wrong += 1;
            if attempts.wrong >= TRIES {
                attempts.wrong = 0;
                attempts.paused_until = Some(Instant::now() + PAUSE);
            }
            return Err(Error::new(ErrorKind::Key, "wrong password").into());
        }
        *attempts = Attempts::default();
        let id = random_hex()?;
        let mut sessions = lock(&self.sessions);
        sessions.retain(|_, s| now - s.used < IDLE && now - s.made < LIFETIME);
        sessions.insert(
            id.clone(),
            Session {
                made: now,
                used: now,
            },
        );
        Ok(with_cookie(
            json!({}),
            &format!("{COOKIE}={id}; HttpOnly; Secure; SameSite=Strict; Path=/"),
        ))
    }

    pub(crate) fn logout(&self, headers: &HeaderMap) -> Response {
        if let Some(id) = cookie_value(headers) {
            lock(&self.sessions).remove(id);
        }
        with_cookie(
            json!({}),
            &format!("{COOKIE}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"),
        )
    }

    pub(crate) fn allow_call(&self, operation: &str) -> Result<(), Error> {
        if CALLS.contains(&operation) {
            Ok(())
        } else {
            Err(not_here(operation))
        }
    }
}

fn not_here(operation: &str) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        format!("{operation} is not available in the web interface"),
    )
}

/// Checks a long operation and the paths in its request (WEB.md §4); the
/// paths are replaced by their resolved form, so what was checked is what
/// is used.
pub(crate) fn allow_start(
    folders: &[PathBuf],
    operation: &str,
    mut request: Value,
) -> Result<Value, Error> {
    if !STARTS.contains(&operation) {
        return Err(not_here(operation));
    }
    let field = match operation {
        "backup" => "source",
        "restore" => "target",
        _ => return Ok(request),
    };
    let path = request
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{field} is missing")))?;
    let resolved = inside(folders, Path::new(path))?;
    request[field] = json!(resolved);
    Ok(request)
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
        let mut folders = Vec::new();
        for folder in &options.folders {
            folders.push(folder.canonicalize().map_err(|e| {
                Error::new(ErrorKind::NotFound, format!("{}: {e}", folder.display()))
            })?);
        }
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
                password: options.password_hash,
                folders,
                sessions: Mutex::new(HashMap::new()),
                attempts: tokio::sync::Mutex::new(Attempts::default()),
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
