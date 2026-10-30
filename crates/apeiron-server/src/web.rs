//! Web mode (WEB.md): one repository served to the network over TLS, to
//! signed-in users, with only the operations, backups and folders their
//! role and rights allow (USERS.md); or one for each tenant (TENANTS.md).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use apeiron::{EntryInfo, Error, ErrorKind, ListRequest, LogRequest, ObjectId, Repository};
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
use crate::oidc::{Oidc, OidcConfig};
use crate::replicate::Replicator;
use crate::shares::{Share, Shares, gone};
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
    /// for `names` the first time, and again when a name it was not made
    /// for is added, such as the host of a new tenant (WEB.md §2,
    /// TENANTS.md §3).
    pub fn kept_in(dir: &Path, names: Vec<String>) -> Result<Tls, Error> {
        let (cert, key) = (dir.join("web-cert.pem"), dir.join("web-key.pem"));
        // The names the kept certificate was made for, one per line.
        let made_for = dir.join("web-cert.names");
        let io_error = |path: &Path, e: io::Error| {
            Error::new(ErrorKind::Io, format!("{}: {e}", path.display()))
        };
        let listed = names.join("\n") + "\n";
        if cert.exists() && key.exists() {
            let covered = match fs::read_to_string(&made_for) {
                Ok(known) => names.iter().all(|n| known.lines().any(|k| k == n)),
                // Kept before the names were: taken to cover them.
                Err(_) => {
                    let _ = fs::write(&made_for, &listed);
                    true
                }
            };
            if covered {
                return Ok(Tls {
                    cert_pem: fs::read(&cert).map_err(|e| io_error(&cert, e))?,
                    key_pem: fs::read(&key).map_err(|e| io_error(&key, e))?,
                });
            }
        }
        let tls = Tls::self_signed(names)?;
        fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
        write_private(&key, &tls.key_pem).map_err(|e| io_error(&key, e))?;
        fs::write(&cert, &tls.cert_pem).map_err(|e| io_error(&cert, e))?;
        fs::write(&made_for, &listed).map_err(|e| io_error(&made_for, e))?;
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
    /// The share links (SHARE.md).
    pub shares: Shares,
    /// Remotes every ref is pushed to after changes (REPLICATION.md §4).
    pub replicas: Vec<String>,
    /// Serves a standby copy: everything but backups (REPLICATION.md §5).
    pub standby: bool,
}

/// One tenant of a web server (TENANTS.md).
pub struct TenantOptions {
    pub name: String,
    /// The host name browsers reach the tenant at.
    pub host: String,
    pub repo: Repository,
    pub users: PathBuf,
    pub folders: Vec<PathBuf>,
    pub audit: Audit,
    pub shares: Shares,
    pub replicas: Vec<String>,
    pub standby: bool,
}

struct Session {
    user: String,
    /// The user's password hash and identity at sign-in; a change to
    /// either ends the session.
    stamp: String,
    made: Instant,
    used: Instant,
}

#[derive(Default)]
struct Attempts {
    wrong: u32,
    paused_until: Option<Instant>,
}

/// What a session of `user` stays valid with.
fn stamp(user: &User) -> String {
    format!("{}\n{}", user.password, user.oidc.as_deref().unwrap_or(""))
}

/// The cookie that ties a sign-in through the identity provider to the
/// browser that started it (IDENTITY.md §3).
const OIDC_COOKIE: &str = "apeiron_oidc";

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
    pub(crate) shares: Shares,
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
    oidc: Oidc,
    /// Pushes to the replicas, if there are any.
    replicator: Option<Replicator>,
    standby: bool,
}

fn cookie_value(headers: &HeaderMap) -> Option<&str> {
    named_cookie(headers, COOKIE)
}

fn named_cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(name)?.strip_prefix('='))
}

fn no_provider() -> Error {
    Error::new(
        ErrorKind::Unsupported,
        "no identity provider is set up; see `apeiron web oidc set`",
    )
}

/// The kind of an error as its JSON name, such as `key`.
fn kind_name(e: &Error) -> String {
    serde_json::to_value(e.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "internal".to_owned())
}

/// A redirect to `to` that sets `cookies`.
fn redirect(to: &str, cookies: &[String]) -> Response {
    let mut response = (StatusCode::FOUND, [(header::LOCATION, to.to_owned())]).into_response();
    for cookie in cookies {
        if let Ok(v) = HeaderValue::from_str(cookie) {
            response.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    response
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
    /// Pushes to the replicas now, as after a backup.
    pub(crate) fn replicate_now(&self) {
        if let Some(r) = &self.replicator {
            r.nudge();
        }
    }

    /// The users file.
    pub(crate) fn users_path(&self) -> &Path {
        &self.users_file
    }

    /// The canonical `--folder` paths users may be given folders inside.
    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether this server serves a standby copy (REPLICATION.md §5).
    pub(crate) fn standby(&self) -> bool {
        self.standby
    }

    /// The users file as it is now.
    fn users(&self) -> std::sync::MutexGuard<'_, Loaded> {
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
        loaded
    }

    /// The user named `name` in the users file as it is now.
    fn user(&self, name: &str) -> Option<User> {
        self.users().users.get(name).cloned()
    }

    /// The identity provider's settings, if sign-ins through one are on.
    pub(crate) fn oidc_config(&self) -> Option<OidcConfig> {
        self.users().users.oidc.clone()
    }

    /// The user of the request's live session; using it keeps it alive.
    pub(crate) fn current(&self, headers: &HeaderMap) -> Option<User> {
        let id = cookie_value(headers)?;
        let (name, kept) = {
            let mut sessions = lock(&self.sessions);
            let now = Instant::now();
            match sessions.get_mut(id) {
                Some(s) if now - s.used < IDLE && now - s.made < LIFETIME => {
                    s.used = now;
                    (s.user.clone(), s.stamp.clone())
                }
                Some(_) => {
                    sessions.remove(id);
                    return None;
                }
                None => return None,
            }
        };
        match self.user(&name) {
            Some(user) if stamp(&user) == kept => Some(user),
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
                // A user without a password takes as long as anyone.
                Some(user) if !user.password.is_empty() => None,
                _ => {
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
        Ok(with_cookie(json!({}), &self.new_session(&user)?))
    }

    /// Starts a session of `user`; returns its cookie.
    fn new_session(&self, user: &User) -> Result<String, Error> {
        let id = random_hex()?;
        let now = Instant::now();
        let mut sessions = lock(&self.sessions);
        sessions.retain(|_, s| now - s.used < IDLE && now - s.made < LIFETIME);
        sessions.insert(
            id.clone(),
            Session {
                user: user.name.clone(),
                stamp: stamp(user),
                made: now,
                used: now,
            },
        );
        Ok(format!(
            "{COOKIE}={id}; HttpOnly; Secure; SameSite=Strict; Path=/"
        ))
    }

    /// Sends the browser to the identity provider (IDENTITY.md §3).
    pub(crate) async fn oidc_start(&self, from: Option<String>) -> Response {
        let result = async {
            let config = self.oidc_config().ok_or_else(no_provider)?;
            self.oidc.start(&config).await
        }
        .await;
        match result {
            Ok((url, state)) => redirect(
                &url,
                &[format!(
                    "{OIDC_COOKIE}={state}; HttpOnly; Secure; SameSite=Lax; Path=/api/oidc; Max-Age=600"
                )],
            ),
            Err(e) => {
                // A provider that cannot be reached is recorded like any
                // failed sign-in (AUDIT.md §1).
                let actor = Actor { user: None, from };
                let detail = json!({"via": "oidc", "error": e.message});
                if let Err(e) = self.audit.record(&actor, "sign_in", detail, &kind_name(&e)) {
                    return Failure::from(e).into_response();
                }
                redirect(&format!("/#oidc_error={}", kind_name(&e)), &[])
            }
        }
    }

    /// Where the identity provider sends the browser back: checks the sign-in,
    /// starts a session and goes to the pages (IDENTITY.md §3).
    pub(crate) async fn oidc_callback(
        &self,
        headers: &HeaderMap,
        query: &HashMap<String, String>,
        from: Option<String>,
    ) -> Response {
        let clear =
            format!("{OIDC_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/api/oidc; Max-Age=0");
        let result = async {
            let config = self.oidc_config().ok_or_else(|| (None, no_provider()))?;
            if let Some(error) = query.get("error") {
                return Err((
                    None,
                    Error::new(
                        ErrorKind::Key,
                        format!("the identity provider said {error}"),
                    ),
                ));
            }
            let state = query.get("state").map(String::as_str).unwrap_or("");
            // The state must come back to the browser that started the sign-in.
            if state.is_empty() || named_cookie(headers, OIDC_COOKIE) != Some(state) {
                return Err((
                    None,
                    Error::new(
                        ErrorKind::Key,
                        "this sign-in was started in another browser",
                    ),
                ));
            }
            let code = query.get("code").map(String::as_str).unwrap_or("");
            let value = self
                .oidc
                .finish(&config, code, state)
                .await
                .map_err(|e| (None, e))?;
            let user = self
                .users()
                .users
                .by_identity(&value)
                .cloned()
                .ok_or_else(|| {
                    (
                        Some(value.clone()),
                        Error::new(ErrorKind::Key, format!("no user of this server is {value}")),
                    )
                })?;
            Ok((value, user))
        }
        .await;
        let actor = |user: Option<&str>| Actor {
            user: user.map(str::to_owned),
            from: from.clone(),
        };
        match result {
            Ok((value, user)) => {
                let recorded = self.audit.record(
                    &actor(Some(&user.name)),
                    "sign_in",
                    json!({"via": "oidc", "identity": value}),
                    "ok",
                );
                match recorded.and_then(|()| self.new_session(&user)) {
                    Ok(cookie) => redirect("/", &[cookie, clear]),
                    Err(e) => Failure::from(e).into_response(),
                }
            }
            Err((value, e)) => {
                let detail = json!({"via": "oidc", "identity": value, "error": e.message});
                if let Err(e) = self
                    .audit
                    .record(&actor(None), "sign_in", detail, &kind_name(&e))
                {
                    return Failure::from(e).into_response();
                }
                redirect(&format!("/#oidc_error={}", kind_name(&e)), &[clear])
            }
        }
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
        if self.standby && operation == "backup" {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "this server is a standby and takes no backups; back up to the primary",
            ));
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

/// The entry at `path` in `snapshot`; `None` for the root.
fn entry_at(repo: &Repository, snapshot: ObjectId, path: &str) -> Result<Option<EntryInfo>, Error> {
    if path.is_empty() {
        return Ok(None);
    }
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    repo.list(&ListRequest {
        snapshot: snapshot.to_string(),
        path: parent.into(),
    })?
    .into_iter()
    .find(|e| e.name.to_string() == name)
    .map(Some)
    .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("path {path}")))
}

/// A link's path joined with `sub`, a path below it; `.` and `..` are
/// refused, so nothing outside the shared path is reached (SHARE.md §4).
pub(crate) fn below(share: &Share, sub: &str) -> Result<String, Error> {
    let mut parts: Vec<&str> = share.path.split('/').filter(|p| !p.is_empty()).collect();
    for part in sub.split('/').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." {
            return Err(gone());
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

impl WebState {
    /// Makes a link to `path` in the snapshot `spec` for `user` (SHARE.md
    /// §1); the answer holds the secret, shown only now.
    pub(crate) fn share(
        &self,
        repo: &Repository,
        user: &User,
        spec: &str,
        path: &str,
        days: u32,
    ) -> Result<(Share, String), Error> {
        if user.role == Role::Reader {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "sharing is not allowed for the role reader",
            ));
        }
        let snapshot = repo.resolve(spec).map_err(|e| match e.kind {
            ErrorKind::NotFound if user.role != Role::Admin => hidden(),
            _ => e,
        })?;
        if user.role != Role::Admin {
            self.visible(repo, user, snapshot)?;
        }
        let path = path.trim_matches('/');
        if path.split('/').any(|p| p == "." || p == "..") {
            return Err(invalid(format!("{path} is not a path inside a backup")));
        }
        entry_at(repo, snapshot, path)?;
        self.shares.add(&user.name, snapshot, path, days)
    }

    /// The link whose secret is `secret`, while its owner may still share
    /// what it names (SHARE.md §2).
    pub(crate) fn shared(&self, repo: &Repository, secret: &str) -> Result<Share, Error> {
        let share = self.shares.find(secret)?;
        let owner = self.user(&share.owner).ok_or_else(gone)?;
        match owner.role {
            Role::Admin => {}
            Role::Member => self
                .visible(repo, &owner, share.snapshot)
                .map_err(|_| gone())?,
            Role::Reader => return Err(gone()),
        }
        Ok(share)
    }

    /// What a link shows at `sub` below its path: the entry there and, for
    /// a folder, what it holds (SHARE.md §4).
    pub(crate) fn shared_list(
        &self,
        repo: &Repository,
        share: &Share,
        sub: &str,
    ) -> Result<Value, Error> {
        let path = below(share, sub)?;
        let entry = entry_at(repo, share.snapshot, &path).map_err(|_| gone())?;
        let entries = match &entry {
            Some(e) if e.kind != apeiron::Kind::Dir => Vec::new(),
            _ => repo.list(&ListRequest {
                snapshot: share.snapshot.to_string(),
                path: path.as_str().into(),
            })?,
        };
        let time = repo
            .log(&LogRequest {
                start: share.snapshot.to_string(),
                limit: 1,
            })?
            .snapshots
            .first()
            .map(|s| s.time);
        Ok(json!({
            "name": share.path.rsplit('/').next().unwrap_or_default(),
            "path": sub.trim_matches('/'),
            "entry": entry,
            "entries": entries,
            "time": time,
            "expires": share.expires,
        }))
    }

    /// The file at `sub` below a link's path and its size, before it is
    /// read (SHARE.md §4).
    pub(crate) fn shared_file(
        &self,
        repo: &Repository,
        share: &Share,
        sub: &str,
    ) -> Result<(String, u64), Error> {
        let path = below(share, sub)?;
        match entry_at(repo, share.snapshot, &path).map_err(|_| gone())? {
            Some(e) if e.kind == apeiron::Kind::File => Ok((path, e.size.unwrap_or(0))),
            _ => Err(invalid("only files can be downloaded")),
        }
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

/// What one site of a web server is made of: a single server's options, or
/// a tenant's.
struct Site {
    repo: Repository,
    users: PathBuf,
    folders: Vec<PathBuf>,
    audit: Audit,
    shares: Shares,
    replicas: Vec<String>,
    standby: bool,
}

/// The server of one repository and its users, files and folders.
fn site(site: Site) -> Result<Arc<App>, Error> {
    let mut roots = Vec::new();
    for folder in &site.folders {
        roots.push(
            folder.canonicalize().map_err(|e| {
                Error::new(ErrorKind::NotFound, format!("{}: {e}", folder.display()))
            })?,
        );
    }
    Users::load(&site.users)?.need_admin()?;
    let remotes = site.repo.remotes()?;
    if let Some(missing) = site
        .replicas
        .iter()
        .find(|r| !remotes.iter().any(|known| &known.name == *r))
    {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("no remote named {missing}; add it with `apeiron remote add`"),
        ));
    }
    let decoy = hash_password("not the password of anyone")?;
    let oidc = Oidc::new()?;
    let app = Arc::new_cyclic(|app| App {
        mode: Mode::Web(Box::new(WebState {
            audit: site.audit,
            shares: site.shares,
            users_file: site.users,
            users: Mutex::new(Loaded {
                modified: None,
                users: Users::default(),
            }),
            roots,
            sessions: Mutex::new(HashMap::new()),
            attempts: tokio::sync::Mutex::new(Attempts::default()),
            reachable: Mutex::new(HashMap::new()),
            decoy,
            oidc,
            replicator: Replicator::start(site.replicas, app.clone()),
            standby: site.standby,
        })),
        repo: Mutex::new(Some(site.repo)),
        jobs: Mutex::new(HashMap::new()),
        next_job: AtomicU64::new(1),
        memory: None,
    });
    if let Mode::Web(web) = &app.mode {
        web.replicate_now();
    }
    Ok(app)
}

/// A running web server.
pub struct Web {
    address: SocketAddr,
    running: Running,
}

impl Web {
    /// Starts a web server for one repository on a thread of its own.
    pub fn start(options: WebOptions) -> Result<Self, Error> {
        let app = site(Site {
            repo: options.repo,
            users: options.users,
            folders: options.folders,
            audit: options.audit,
            shares: options.shares,
            replicas: options.replicas,
            standby: options.standby,
        })?;
        Web::serve(options.listen, options.tls, crate::router(app))
    }

    /// Starts a web server for several tenants, each answering at its own
    /// host (TENANTS.md §3).
    pub fn start_tenants(
        listen: SocketAddr,
        tls: Option<Tls>,
        tenants: Vec<TenantOptions>,
    ) -> Result<Self, Error> {
        if tenants.is_empty() {
            return Err(invalid(
                "there are no tenants; add one with `apeiron web tenant add`",
            ));
        }
        let mut sites = HashMap::new();
        for t in tenants {
            let app = site(Site {
                repo: t.repo,
                users: t.users,
                folders: t.folders,
                audit: t.audit,
                shares: t.shares,
                replicas: t.replicas,
                standby: t.standby,
            })
            .map_err(|e| Error::new(e.kind, format!("tenant {}: {}", t.name, e.message)))?;
            if sites
                .insert(crate::host_name(&t.host), crate::router(app))
                .is_some()
            {
                return Err(invalid(format!("two tenants have the host {}", t.host)));
            }
        }
        Web::serve(listen, tls, crate::by_host(sites))
    }

    fn serve(listen: SocketAddr, tls: Option<Tls>, router: axum::Router) -> Result<Self, Error> {
        let io_error = |e: io::Error| Error::new(ErrorKind::Io, e.to_string());
        let tls = tls.as_ref().map(Tls::config).transpose()?;
        let listener = std::net::TcpListener::bind(listen)
            .map_err(|e| Error::new(ErrorKind::Io, format!("cannot listen on {listen}: {e}")))?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let address = listener.local_addr().map_err(io_error)?;
        let running = Running::start(listener, router, tls).map_err(io_error)?;
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
