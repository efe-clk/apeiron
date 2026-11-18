//! `apeiron peer serve`: a device offers its repository to its other
//! devices, so they sync without a server in between (SYNC.md §7, FORMAT.md
//! §15.9).

// A refusal is a whole response, as in `git.rs`.
#![allow(clippy::result_large_err)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use apeiron::{Error, ErrorKind, ObjectId};
use apeiron_core::{FolderStore, Store};
use apeiron_remote::{Swap, WireError};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::web::{Tls, Web};

/// The largest file sent or taken at once: a pack and a little more.
const LIMIT: usize = 128 << 20;
/// How long `--pair` lets a new device read without the key.
pub const PAIRING: Duration = Duration::from_secs(10 * 60);

/// The device's certificate and key, kept so its fingerprint stays the
/// same.
#[derive(Clone, Serialize, Deserialize)]
pub struct PeerIdentity {
    pub cert_pem: String,
    pub key_pem: String,
}

impl PeerIdentity {
    /// The identity of the repository folder `repo`, kept in
    /// `<state>/peer/`, made the first time.
    pub fn for_repo(state: &Path, repo: &Path) -> Result<Self, Error> {
        let key: String = Sha256::digest(repo.to_string_lossy().as_bytes())
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect();
        PeerIdentity::load_or_make(&state.join("peer").join(format!("{key}.json")))
    }

    /// The identity in `file`, made and saved there if there is none.
    pub fn load_or_make(file: &Path) -> Result<Self, Error> {
        let io = |e: std::io::Error| Error::new(ErrorKind::Io, format!("{}: {e}", file.display()));
        match std::fs::read(file) {
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|e| Error::new(ErrorKind::Corrupt, format!("{}: {e}", file.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let made = rcgen::generate_simple_self_signed(vec!["apeiron-peer".to_owned()])
                    .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
                let identity = PeerIdentity {
                    cert_pem: made.cert.pem(),
                    key_pem: made.signing_key.serialize_pem(),
                };
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir).map_err(io)?;
                }
                let data = serde_json::to_vec_pretty(&identity)
                    .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
                use std::io::Write;
                options
                    .open(file)
                    .and_then(|mut f| f.write_all(&data).and_then(|()| f.sync_all()))
                    .map_err(io)?;
                Ok(identity)
            }
            Err(e) => Err(io(e)),
        }
    }

    /// The SHA-256 of the certificate, which the address names.
    pub fn fingerprint(&self) -> Result<String, Error> {
        use rustls::pki_types::pem::PemObject;
        let der = rustls::pki_types::CertificateDer::from_pem_slice(self.cert_pem.as_bytes())
            .map_err(|e| Error::new(ErrorKind::Corrupt, format!("certificate: {e}")))?;
        Ok(apeiron_remote::fingerprint(&der))
    }
}

/// What `apeiron peer serve` needs.
pub struct PeerOptions {
    /// The repository folder offered.
    pub repo: PathBuf,
    pub listen: SocketAddr,
    pub identity: PeerIdentity,
    /// What other copies show: [`apeiron_core::Repository::peer_token`] of
    /// the fingerprint.
    pub token: String,
    /// Lets a device without the key read for [`PAIRING`], to clone.
    pub pair: bool,
}

struct Peer {
    store: FolderStore,
    /// SHA-256 of the token, so it is compared without telling how much
    /// of it matched.
    token_hash: [u8; 32],
    pair_until: Option<Instant>,
}

/// Starts offering the repository; runs until the returned server ends.
pub fn serve_peer(options: PeerOptions) -> Result<Web, Error> {
    let store = FolderStore::open(&options.repo)
        .map_err(|e| Error::new(ErrorKind::NotFound, e.to_string()))?;
    let peer = Arc::new(Peer {
        store,
        token_hash: Sha256::digest(options.token.as_bytes()).into(),
        pair_until: options.pair.then(|| Instant::now() + PAIRING),
    });
    let router = Router::new()
        .route("/peer/v1/read", get(read))
        .route("/peer/v1/range", get(range))
        .route("/peer/v1/list", get(list))
        .route("/peer/v1/ref", get(read_ref))
        .route("/peer/v1/refs", get(list_refs))
        .route("/peer/v1/write", post(write))
        .route("/peer/v1/dirs", post(dirs))
        .route("/peer/v1/delete", post(delete))
        .route("/peer/v1/swap", post(swap))
        .layer(DefaultBodyLimit::max(LIMIT))
        .with_state(peer);
    let tls = Tls {
        cert_pem: options.identity.cert_pem.into_bytes(),
        key_pem: options.identity.key_pem.into_bytes(),
        acme: None,
    };
    Web::serve(options.listen, Some(tls), router)
}

fn wire(status: StatusCode, kind: &str, message: impl Into<String>) -> Response {
    wire_with(
        status,
        WireError {
            kind: kind.to_owned(),
            message: message.into(),
            name: None,
            expected: None,
            actual: None,
            path: None,
        },
    )
}

fn wire_with(status: StatusCode, error: WireError) -> Response {
    let body = serde_json::to_vec(&error).unwrap_or_default();
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// An error of the folder store, as the client turns it back.
fn store_error(e: apeiron_core::Error) -> Response {
    use apeiron_core::Error as E;
    let message = e.to_string();
    let mut error = WireError {
        kind: "failed".to_owned(),
        message,
        name: None,
        expected: None,
        actual: None,
        path: None,
    };
    let status = match e {
        E::RefConflict {
            name,
            expected,
            actual,
        } => {
            error.kind = "ref_conflict".to_owned();
            error.name = Some(name);
            error.expected = expected;
            error.actual = actual;
            StatusCode::CONFLICT
        }
        E::RefLocked { name, .. } => {
            error.kind = "ref_locked".to_owned();
            error.name = Some(name);
            StatusCode::CONFLICT
        }
        E::RefNameConflict(name) => {
            error.kind = "ref_name_conflict".to_owned();
            error.name = Some(name);
            StatusCode::CONFLICT
        }
        E::InvalidRefName(name) => {
            error.kind = "invalid_ref_name".to_owned();
            error.name = Some(name);
            StatusCode::BAD_REQUEST
        }
        E::RemoteCleaning(_) => {
            error.kind = "remote_cleaning".to_owned();
            StatusCode::CONFLICT
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    wire_with(status, error)
}

impl Peer {
    /// Whether the request may go on: with the token, or, for a read,
    /// during the pairing time.
    fn allowed(&self, headers: &HeaderMap, read: bool) -> Result<(), Response> {
        let shown = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if let Some(token) = shown {
            let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
            if hash == self.token_hash {
                return Ok(());
            }
        }
        if read && self.pair_until.is_some_and(|until| Instant::now() < until) {
            return Ok(());
        }
        Err(wire(
            StatusCode::UNAUTHORIZED,
            "refused",
            "this device needs proof of the repository's key",
        ))
    }
}

/// A path of a repository file: plain names separated by `/`, nothing that
/// climbs out.
fn checked(path: &str, may_be_empty: bool) -> Result<&str, Response> {
    let ok = (may_be_empty && path.is_empty())
        || (!path.is_empty()
            && path.len() <= 512
            && path.split('/').count() <= 16
            && path.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            }));
    if ok {
        Ok(path)
    } else {
        Err(wire(
            StatusCode::BAD_REQUEST,
            "failed",
            format!("invalid path {path:?}"),
        ))
    }
}

#[derive(Deserialize)]
struct PathQuery {
    #[serde(default)]
    path: String,
    #[serde(default)]
    dir: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    len: u64,
}

type Shared = State<Arc<Peer>>;

/// Runs blocking work of the store off the server's threads.
async fn blocking(f: impl FnOnce() -> Response + Send + 'static) -> Response {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| wire(StatusCode::INTERNAL_SERVER_ERROR, "failed", e.to_string()))
}

fn json(value: &impl Serialize) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => ([(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(e) => wire(StatusCode::INTERNAL_SERVER_ERROR, "failed", e.to_string()),
    }
}

async fn read(State(peer): Shared, headers: HeaderMap, Query(q): Query<PathQuery>) -> Response {
    if let Err(r) = peer
        .allowed(&headers, true)
        .and_then(|()| checked(&q.path, false).map(drop))
    {
        return r;
    }
    blocking(move || match peer.store.read(&q.path) {
        Ok(Some(data)) => data.into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => store_error(e),
    })
    .await
}

async fn range(State(peer): Shared, headers: HeaderMap, Query(q): Query<PathQuery>) -> Response {
    if let Err(r) = peer
        .allowed(&headers, true)
        .and_then(|()| checked(&q.path, false).map(drop))
    {
        return r;
    }
    if q.len > LIMIT as u64 {
        return wire(StatusCode::BAD_REQUEST, "failed", "the range is too long");
    }
    blocking(
        move || match peer.store.read_range(&q.path, q.offset, q.len) {
            Ok(data) => data.into_response(),
            Err(e) => store_error(e),
        },
    )
    .await
}

async fn list(State(peer): Shared, headers: HeaderMap, Query(q): Query<PathQuery>) -> Response {
    if let Err(r) = peer
        .allowed(&headers, true)
        .and_then(|()| checked(&q.dir, true).map(drop))
    {
        return r;
    }
    blocking(move || match peer.store.list(&q.dir) {
        Ok(names) => json(&names),
        Err(e) => store_error(e),
    })
    .await
}

async fn read_ref(State(peer): Shared, headers: HeaderMap, Query(q): Query<PathQuery>) -> Response {
    if let Err(r) = peer.allowed(&headers, true) {
        return r;
    }
    if let Err(e) = apeiron_core::validate_ref_name(&q.name) {
        return store_error(e);
    }
    blocking(move || match peer.store.read_ref(&q.name) {
        Ok(Some(id)) => json(&id),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => store_error(e),
    })
    .await
}

async fn list_refs(State(peer): Shared, headers: HeaderMap) -> Response {
    if let Err(r) = peer.allowed(&headers, true) {
        return r;
    }
    blocking(move || match peer.store.list_refs() {
        Ok(names) => json(&names),
        Err(e) => store_error(e),
    })
    .await
}

async fn write(
    State(peer): Shared,
    headers: HeaderMap,
    Query(q): Query<PathQuery>,
    body: Bytes,
) -> Response {
    if let Err(r) = peer
        .allowed(&headers, false)
        .and_then(|()| checked(&q.path, false).map(drop))
    {
        return r;
    }
    blocking(move || match peer.store.write_new(&q.path, &body) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    })
    .await
}

async fn dirs(State(peer): Shared, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = peer.allowed(&headers, false) {
        return r;
    }
    let names: Vec<String> = match serde_json::from_slice(&body) {
        Ok(names) => names,
        Err(e) => return wire(StatusCode::BAD_REQUEST, "failed", e.to_string()),
    };
    for name in &names {
        if let Err(r) = checked(name, false) {
            return r;
        }
    }
    blocking(move || {
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        match peer.store.create_dirs(&names) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => store_error(e),
        }
    })
    .await
}

async fn delete(State(peer): Shared, headers: HeaderMap, Query(q): Query<PathQuery>) -> Response {
    if let Err(r) = peer
        .allowed(&headers, false)
        .and_then(|()| checked(&q.path, false).map(drop))
    {
        return r;
    }
    blocking(move || match peer.store.delete(&q.path) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error(e),
    })
    .await
}

async fn swap(State(peer): Shared, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = peer.allowed(&headers, false) {
        return r;
    }
    let swap: Swap = match serde_json::from_slice(&body) {
        Ok(swap) => swap,
        Err(e) => return wire(StatusCode::BAD_REQUEST, "failed", e.to_string()),
    };
    if let Err(e) = apeiron_core::validate_ref_name(&swap.name) {
        return store_error(e);
    }
    blocking(move || {
        let new: ObjectId = swap.new;
        match peer.store.swap_ref(&swap.name, swap.expected, new) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => store_error(e),
        }
    })
    .await
}
