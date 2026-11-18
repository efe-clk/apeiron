//! A remote that is another device running `apeiron peer serve` (SYNC.md
//! §7, FORMAT.md §15.9): its repository, over HTTPS, with its certificate
//! checked by the fingerprint in the address.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use apeiron_core::{Error, ObjectId, PeerUrl, Result, Store};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::runtime::Runtime;

/// What the device answers with when something failed.
#[derive(Serialize, Deserialize)]
pub struct WireError {
    /// `ref_conflict`, `ref_locked`, `ref_name_conflict`, `invalid_ref_name`,
    /// `remote_cleaning`, `not_found`, `refused` or `failed`.
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub expected: Option<ObjectId>,
    #[serde(default)]
    pub actual: Option<ObjectId>,
    #[serde(default)]
    pub path: Option<String>,
}

/// A conditional ref update.
#[derive(Serialize, Deserialize)]
pub struct Swap {
    pub name: String,
    pub expected: Option<ObjectId>,
    pub new: ObjectId,
}

/// The SHA-256 of a certificate, as the address names it.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Accepts exactly the certificate whose SHA-256 the address names: the
/// device's own, self-signed one.
#[derive(Debug)]
struct Pinned {
    fingerprint: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if fingerprint(end_entity) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "the device's certificate is not the one its address names".to_owned(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The repository of another device.
pub struct PeerStore {
    url: PeerUrl,
    base: String,
    token: Option<String>,
    http: reqwest::Client,
    runtime: Runtime,
}

impl PeerStore {
    /// Connects to the device at `url`. `token` proves this copy holds the
    /// repository's data key; without it only a device in its pairing time
    /// answers, and only to reads.
    pub fn connect(url: &PeerUrl, token: Option<String>) -> Result<Self> {
        let failed = |e: String| Error::InvalidRemote(format!("peer {}: {e}", url.host));
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let tls = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| failed(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Pinned {
                fingerprint: url.fingerprint.clone(),
                provider,
            }))
            .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| failed(e.to_string()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| failed(e.to_string()))?;
        let host = if url.host.contains(':') {
            format!("[{}]", url.host)
        } else {
            url.host.clone()
        };
        let store = PeerStore {
            url: url.clone(),
            base: format!("https://{host}:{}/peer/v1", url.port),
            token,
            http,
            runtime,
        };
        // Reading the config checks the address, the certificate and access
        // at once, so a wrong one fails before anything is copied.
        store.read("config")?;
        Ok(store)
    }

    fn error(&self, path: &str, message: impl Into<String>) -> Error {
        Error::Io {
            path: self.location(path),
            source: std::io::Error::other(message.into()),
        }
    }

    /// Sends a request; `Ok(None)` for "not found".
    fn send(
        &self,
        method: reqwest::Method,
        op: &str,
        query: &[(&str, String)],
        body: Option<Vec<u8>>,
        path: &str,
    ) -> Result<Option<Vec<u8>>> {
        self.runtime.block_on(async {
            let address = reqwest::Url::parse_with_params(&format!("{}/{op}", self.base), query)
                .map_err(|e| self.error(path, e.to_string()))?;
            let mut request = self.http.request(method, address);
            if let Some(token) = &self.token {
                request = request.bearer_auth(token);
            }
            if let Some(body) = body {
                request = request.body(body);
            }
            let response = request.send().await.map_err(|e| {
                let mut message = e.to_string();
                let mut source = std::error::Error::source(&e);
                while let Some(s) = source {
                    message = format!("{message}: {s}");
                    source = s.source();
                }
                self.error(path, message)
            })?;
            let status = response.status();
            let bytes = response
                .bytes()
                .await
                .map_err(|e| self.error(path, e.to_string()))?;
            if status.is_success() {
                return Ok(Some(bytes.to_vec()));
            }
            if status == reqwest::StatusCode::NOT_FOUND && bytes.is_empty() {
                return Ok(None);
            }
            let wire: WireError = serde_json::from_slice(&bytes).map_err(|_| {
                self.error(path, format!("the device answered {status}"))
            })?;
            Err(match wire.kind.as_str() {
                "ref_conflict" => Error::RefConflict {
                    name: wire.name.unwrap_or_default(),
                    expected: wire.expected,
                    actual: wire.actual,
                },
                "ref_locked" => Error::RefLocked {
                    name: wire.name.unwrap_or_default(),
                    lock: self.location(wire.path.as_deref().unwrap_or(path)),
                },
                "ref_name_conflict" => Error::RefNameConflict(wire.name.unwrap_or_default()),
                "invalid_ref_name" => Error::InvalidRefName(wire.name.unwrap_or_default()),
                "remote_cleaning" => {
                    Error::RemoteCleaning(self.location(wire.path.as_deref().unwrap_or(path)))
                }
                "refused" => Error::InvalidRemote(format!(
                    "peer {}: {}; this copy cannot show it the repository's key (pair with `apeiron peer serve --pair`, or use an encrypted repository)",
                    self.url.host, wire.message
                )),
                _ => self.error(path, wire.message),
            })
        })
    }

    fn get(&self, op: &str, query: &[(&str, String)], path: &str) -> Result<Option<Vec<u8>>> {
        self.send(reqwest::Method::GET, op, query, None, path)
    }

    fn post(&self, op: &str, query: &[(&str, String)], body: Vec<u8>, path: &str) -> Result<()> {
        self.send(reqwest::Method::POST, op, query, Some(body), path)
            .map(drop)
    }
}

fn json<T: serde::de::DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| Error::Io {
        path: PathBuf::from(what),
        source: std::io::Error::other(format!("the device sent an invalid answer: {e}")),
    })
}

impl Store for PeerStore {
    fn location(&self, path: &str) -> PathBuf {
        PathBuf::from(format!("peer://{}:{}/{path}", self.url.host, self.url.port))
    }

    fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.get("read", &[("path", path.to_owned())], path)
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        let query = [
            ("path", path.to_owned()),
            ("offset", offset.to_string()),
            ("len", len.to_string()),
        ];
        let bytes = self
            .get("range", &query, path)?
            .ok_or_else(|| self.error(path, "not found"))?;
        if bytes.len() as u64 != len {
            return Err(self.error(path, "the device sent fewer bytes than asked"));
        }
        Ok(bytes)
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        match self.get("list", &[("dir", dir.to_owned())], dir)? {
            Some(bytes) => json(&bytes, dir),
            None => Ok(Vec::new()),
        }
    }

    fn write_new(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.post("write", &[("path", path.to_owned())], bytes.to_vec(), path)
    }

    fn create_dirs(&self, dirs: &[&str]) -> Result<()> {
        let body = serde_json::to_vec(dirs).map_err(|e| self.error("", e.to_string()))?;
        self.post("dirs", &[], body, "")
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.post("delete", &[("path", path.to_owned())], Vec::new(), path)
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        match self.get("ref", &[("name", name.to_owned())], name)? {
            Some(bytes) => json(&bytes, name),
            None => Ok(None),
        }
    }

    fn list_refs(&self) -> Result<Vec<String>> {
        let bytes = self.get("refs", &[], "refs")?.unwrap_or_default();
        json(&bytes, "refs")
    }

    fn swap_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<()> {
        let body = serde_json::to_vec(&Swap {
            name: name.to_owned(),
            expected,
            new,
        })
        .map_err(|e| self.error(name, e.to_string()))?;
        self.post("swap", &[], body, name)
    }
}
