//! Remotes on an SSH server, reached with SFTP (FORMAT.md §15.2).

use std::collections::HashSet;
use std::fmt::Display;
use std::io::{self, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apeiron_core::{Error, ObjectId, Result, SftpUrl, Store, decode_ref, encode_ref};
use russh::client::{self, Handle};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{self, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::{RawSftpSession, SftpSession};
use russh_sftp::protocol::{OpenFlags, Packet, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::runtime::Runtime;

/// Ref locks older than this are stale (FORMAT.md §9.3).
const LOCK_STALE_AGE: Duration = Duration::from_secs(600);
/// Replaces a file atomically; plain SFTP rename never replaces one.
const POSIX_RENAME: &str = "posix-rename@openssh.com";

/// Which files `ssh` would use to check the server and to log in.
#[derive(Clone, Debug)]
pub struct SshOptions {
    pub known_hosts: PathBuf,
    /// Private keys without a passphrase, tried after the agent.
    pub identity_files: Vec<PathBuf>,
    /// Whether to ask the SSH agent for keys.
    pub use_agent: bool,
}

impl SshOptions {
    /// `~/.ssh/known_hosts`, the SSH agent and the default key files.
    pub fn from_home() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .ok_or_else(|| Error::InvalidRemote("cannot find the home folder".to_owned()))?;
        let ssh = home.join(".ssh");
        Ok(SshOptions {
            known_hosts: ssh.join("known_hosts"),
            identity_files: ["id_ed25519", "id_ecdsa", "id_rsa"]
                .iter()
                .map(|name| ssh.join(name))
                .collect(),
            use_agent: true,
        })
    }
}

/// Checks the server key against `known_hosts` and remembers why a key
/// was refused, for the error message.
struct Client {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    refused: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let (host, file) = (&self.host, self.known_hosts.display());
        let refused = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => {
                match keys::check_known_hosts_path(host, self.port, key, &self.known_hosts) {
                    Ok(true) => return Ok(true),
                    Ok(false) => format!(
                        "{host} is not in {file}; connect once with `ssh` and check the key it shows"
                    ),
                    Err(keys::Error::KeyChanged { line }) => format!(
                        "the key of {host} differs from line {line} of {file}; \
                         someone may be in between, or the server was reinstalled"
                    ),
                    Err(e) => format!("cannot check {host} against {file}: {e}"),
                }
            }
            PublicKeyOrCertificate::Certificate(_) => {
                format!("{host} offered a host certificate, which is not supported")
            }
        };
        *self.refused.lock().unwrap_or_else(PoisonError::into_inner) = Some(refused);
        Ok(false)
    }
}

struct Connection {
    sftp: SftpSession,
    /// A second SFTP channel for the extension `SftpSession` does not offer.
    raw: RawSftpSession,
    _ssh: Handle<Client>,
}

/// A remote in a folder on an SSH server. Every call blocks until the
/// server has answered.
pub struct SftpStore {
    /// Dropped before the runtime it runs on.
    conn: Option<Connection>,
    rt: Runtime,
    /// The repository folder, without a trailing `/`.
    root: String,
    /// `sftp://user@host:port`, for messages.
    origin: String,
    /// Folders known to exist.
    dirs: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for SftpStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SftpStore({}{})", self.origin, self.root)
    }
}

impl Drop for SftpStore {
    fn drop(&mut self) {
        let _runtime = self.rt.enter();
        self.conn.take();
    }
}

impl SftpStore {
    /// Connects, checks the server key and logs in with the agent or a key
    /// file.
    pub fn connect(url: &SftpUrl, options: &SshOptions) -> Result<Self> {
        let user = match &url.user {
            Some(user) => user.clone(),
            None => std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .map_err(|_| Error::InvalidRemote("no user name in the URL".to_owned()))?,
        };
        let origin = format!("sftp://{user}@{}:{}", url.host, url.port);
        let failed = |e: String| Error::Io {
            path: PathBuf::from(&origin),
            source: io::Error::other(e),
        };
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| failed(e.to_string()))?;
        let conn = rt.block_on(connect(url, &user, options)).map_err(failed)?;
        Ok(SftpStore {
            conn: Some(conn),
            rt,
            root: url.path.trim_end_matches('/').to_owned(),
            origin,
            dirs: Mutex::new(HashSet::new()),
        })
    }

    fn conn(&self) -> Result<&Connection> {
        self.conn
            .as_ref()
            .ok_or_else(|| self.error("", "the connection is closed"))
    }

    fn path(&self, rel: &str) -> String {
        match (rel, self.root.as_str()) {
            ("", "") => "/".to_owned(),
            ("", root) => root.to_owned(),
            (rel, root) => format!("{root}/{rel}"),
        }
    }

    fn error(&self, rel: &str, e: impl Display) -> Error {
        Error::Io {
            path: self.location(rel),
            source: io::Error::other(e.to_string()),
        }
    }

    fn sftp_error(&self, rel: &str, e: SftpError) -> Error {
        if is_not_found(&e) {
            return Error::Io {
                path: self.location(rel),
                source: io::Error::new(io::ErrorKind::NotFound, e.to_string()),
            };
        }
        self.error(rel, e)
    }

    async fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>> {
        match self.conn()?.sftp.read(self.path(rel)).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(self.sftp_error(rel, e)),
        }
    }

    async fn exists(&self, rel: &str) -> Result<bool> {
        let sftp = &self.conn()?.sftp;
        sftp.try_exists(self.path(rel))
            .await
            .map_err(|e| self.sftp_error(rel, e))
    }

    /// Creates the folder `rel` unless it exists; its parent must exist.
    async fn ensure_dir(&self, rel: &str) -> Result<()> {
        let known = |dirs: &HashSet<String>| dirs.contains(rel);
        if known(&self.dirs.lock().unwrap_or_else(PoisonError::into_inner)) {
            return Ok(());
        }
        if let Err(e) = self.conn()?.sftp.create_dir(self.path(rel)).await {
            if !self.exists(rel).await? {
                return Err(self.sftp_error(rel, e));
            }
        }
        self.dirs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(rel.to_owned());
        Ok(())
    }

    /// Creates the folders above `rel` below the repository folder.
    async fn ensure_parents(&self, rel: &str) -> Result<()> {
        let mut parent = String::new();
        let parts: Vec<&str> = rel.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            if !parent.is_empty() {
                parent.push('/');
            }
            parent.push_str(part);
            self.ensure_dir(&parent).await?;
        }
        Ok(())
    }

    /// Writes `bytes` into a file that must not exist, and syncs it.
    async fn write_exclusive(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        let sftp = &self.conn()?.sftp;
        let flags = OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE;
        let mut file = sftp
            .open_with_flags(self.path(rel), flags)
            .await
            .map_err(|e| self.sftp_error(rel, e))?;
        let io = |e: io::Error| self.error(rel, e);
        file.write_all(bytes).await.map_err(io)?;
        file.flush().await.map_err(io)?;
        file.sync_all().await.map_err(|e| self.sftp_error(rel, e))?;
        file.close().await.map_err(io)
    }

    async fn write_new_async(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        if self.exists(rel).await? {
            return Ok(());
        }
        self.ensure_parents(rel).await?;
        self.ensure_dir("tmp").await?;
        let tmp = format!("tmp/{}", unique_name());
        let written = self.write_exclusive(&tmp, bytes).await;
        let sftp = &self.conn()?.sftp;
        // A plain SFTP rename never replaces a file, so a file someone else
        // wrote meanwhile stays.
        let moved = match written {
            Ok(()) => sftp
                .rename(self.path(&tmp), self.path(rel))
                .await
                .map_err(|e| self.sftp_error(rel, e)),
            Err(e) => Err(e),
        };
        if moved.is_err() {
            let _ = sftp.remove_file(self.path(&tmp)).await;
            if self.exists(rel).await? {
                return Ok(());
            }
        }
        moved
    }

    async fn read_ref_async(&self, name: &str) -> Result<Option<ObjectId>> {
        apeiron_core::validate_ref_name(name)?;
        let rel = format!("refs/{name}");
        let Some(bytes) = self.read_file(&rel).await? else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&bytes);
        decode_ref(&self.location(&rel), &text).map(Some)
    }

    async fn swap_ref_async(
        &self,
        name: &str,
        expected: Option<ObjectId>,
        new: ObjectId,
    ) -> Result<()> {
        apeiron_core::validate_ref_name(name)?;
        let rel = format!("refs/{name}");
        let lock = format!("{rel}.lock");
        self.ensure_parents(&rel).await?;
        let content = encode_ref(new);
        if let Err(e) = self.write_exclusive(&lock, content.as_bytes()).await {
            if !self.exists(&lock).await? {
                return Err(e);
            }
            if !self.is_stale(&lock).await? {
                return Err(Error::RefLocked {
                    name: name.to_owned(),
                    lock: self.location(&lock),
                });
            }
            self.delete_async(&lock).await?;
            self.write_exclusive(&lock, content.as_bytes())
                .await
                .map_err(|_| Error::RefLocked {
                    name: name.to_owned(),
                    lock: self.location(&lock),
                })?;
        }
        let swapped = async {
            let actual = self.read_ref_async(name).await?;
            if actual != expected {
                return Err(Error::RefConflict {
                    name: name.to_owned(),
                    expected,
                    actual,
                });
            }
            self.posix_rename(&lock, &rel).await
        }
        .await;
        if swapped.is_err() {
            let _ = self.delete_async(&lock).await;
        }
        swapped
    }

    async fn is_stale(&self, rel: &str) -> Result<bool> {
        let sftp = &self.conn()?.sftp;
        let meta = match sftp.metadata(self.path(rel)).await {
            Ok(meta) => meta,
            Err(e) if is_not_found(&e) => return Ok(true),
            Err(e) => return Err(self.sftp_error(rel, e)),
        };
        let modified = UNIX_EPOCH + Duration::from_secs(u64::from(meta.mtime.unwrap_or(0)));
        Ok(SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age > LOCK_STALE_AGE))
    }

    async fn posix_rename(&self, from: &str, to: &str) -> Result<()> {
        let mut data = Vec::new();
        for path in [self.path(from), self.path(to)] {
            data.extend_from_slice(&(path.len() as u32).to_be_bytes());
            data.extend_from_slice(path.as_bytes());
        }
        let reply = self.conn()?.raw.extended(POSIX_RENAME, data).await;
        match reply {
            Ok(Packet::Status(status)) if status.status_code == StatusCode::Ok => Ok(()),
            Ok(Packet::Status(status)) => Err(self.error(to, status.error_message)),
            Ok(_) => Err(self.error(to, "unexpected reply to rename")),
            Err(e) => Err(self.sftp_error(to, e)),
        }
    }

    async fn delete_async(&self, rel: &str) -> Result<()> {
        match self.conn()?.sftp.remove_file(self.path(rel)).await {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(self.sftp_error(rel, e)),
        }
    }
}

impl Store for SftpStore {
    fn location(&self, path: &str) -> PathBuf {
        PathBuf::from(format!("{}{}", self.origin, self.path(path)))
    }

    fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.rt.block_on(self.read_file(path))
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        self.rt.block_on(async {
            let sftp = &self.conn()?.sftp;
            let mut file = sftp
                .open(self.path(path))
                .await
                .map_err(|e| self.sftp_error(path, e))?;
            let io = |e: io::Error| self.error(path, e);
            file.seek(SeekFrom::Start(offset)).await.map_err(io)?;
            let mut bytes = vec![0u8; len as usize];
            file.read_exact(&mut bytes).await.map_err(io)?;
            Ok(bytes)
        })
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        self.rt.block_on(async {
            let entries = match self.conn()?.sftp.read_dir(self.path(dir)).await {
                Ok(entries) => entries,
                Err(e) if is_not_found(&e) => return Ok(Vec::new()),
                Err(e) => return Err(self.sftp_error(dir, e)),
            };
            let mut names: Vec<String> = entries
                .map(|e| e.file_name())
                .filter(|n| n != "." && n != "..")
                .collect();
            names.sort();
            Ok(names)
        })
    }

    fn write_new(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.rt.block_on(self.write_new_async(path, bytes))
    }

    fn create_dirs(&self, dirs: &[&str]) -> Result<()> {
        self.rt.block_on(async {
            self.ensure_dir("").await?;
            for dir in dirs {
                self.ensure_dir(dir).await?;
            }
            Ok(())
        })
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.rt.block_on(self.delete_async(path))
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        self.rt.block_on(self.read_ref_async(name))
    }

    fn swap_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<()> {
        self.rt.block_on(self.swap_ref_async(name, expected, new))
    }
}

fn is_not_found(e: &SftpError) -> bool {
    matches!(e, SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile)
}

/// A name for a temporary file that no other writer uses.
fn unique_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{:x}-{n}", std::process::id())
}

async fn connect(
    url: &SftpUrl,
    user: &str,
    options: &SshOptions,
) -> std::result::Result<Connection, String> {
    let refused = Arc::new(Mutex::new(None));
    let client = Client {
        host: url.host.clone(),
        port: url.port,
        known_hosts: options.known_hosts.clone(),
        refused: refused.clone(),
    };
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(600)),
        keepalive_interval: Some(Duration::from_secs(30)),
        ..client::Config::default()
    });
    let mut ssh = match client::connect(config, (url.host.as_str(), url.port), client).await {
        Ok(ssh) => ssh,
        Err(e) => {
            let refused = refused
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            return Err(refused.unwrap_or_else(|| format!("cannot connect: {e}")));
        }
    };
    authenticate(&mut ssh, user, options).await?;

    let sftp_config = russh_sftp::client::Config {
        request_timeout_secs: 60,
        ..russh_sftp::client::Config::default()
    };
    let sftp = SftpSession::new_with_config(subsystem(&ssh).await?, sftp_config)
        .await
        .map_err(|e| format!("cannot start SFTP: {e}"))?;
    let raw = RawSftpSession::new(subsystem(&ssh).await?);
    let version = raw
        .init()
        .await
        .map_err(|e| format!("cannot start SFTP: {e}"))?;
    if !version.extensions.contains_key(POSIX_RENAME) {
        return Err(format!(
            "the server cannot replace files atomically ({POSIX_RENAME}); \
             use an OpenSSH server"
        ));
    }
    Ok(Connection {
        sftp,
        raw,
        _ssh: ssh,
    })
}

async fn subsystem(
    ssh: &Handle<Client>,
) -> std::result::Result<russh::ChannelStream<client::Msg>, String> {
    let channel = ssh
        .channel_open_session()
        .await
        .map_err(|e| format!("cannot open an SSH channel: {e}"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("the server offers no SFTP: {e}"))?;
    Ok(channel.into_stream())
}

/// Logs in with the keys of the SSH agent, then with the key files.
async fn authenticate(
    ssh: &mut Handle<Client>,
    user: &str,
    options: &SshOptions,
) -> std::result::Result<(), String> {
    let rsa_hash = ssh.best_supported_rsa_hash().await.ok().flatten().flatten();
    if options.use_agent {
        if let Some(mut agent) = agent().await {
            let identities = agent.request_identities().await.unwrap_or_default();
            for identity in identities {
                let AgentIdentity::PublicKey { key, .. } = identity else {
                    continue;
                };
                let result = ssh
                    .authenticate_publickey_with(user, key, rsa_hash, &mut agent)
                    .await;
                if result.is_ok_and(|r| r.success()) {
                    return Ok(());
                }
            }
        }
    }
    for path in &options.identity_files {
        // Missing files and keys with a passphrase are skipped.
        let Ok(key) = keys::load_secret_key(path, None) else {
            continue;
        };
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash);
        let result = ssh
            .authenticate_publickey(user, key)
            .await
            .map_err(|e| format!("cannot log in: {e}"))?;
        if result.success() {
            return Ok(());
        }
    }
    Err(format!(
        "the server accepted no SSH key for {user}; add a key to the SSH agent \
         or use a key file without a passphrase"
    ))
}

#[cfg(unix)]
async fn agent() -> Option<AgentClient<Box<dyn keys::agent::client::AgentStream + Send + Unpin>>> {
    AgentClient::connect_env()
        .await
        .ok()
        .map(AgentClient::dynamic)
}

#[cfg(windows)]
async fn agent() -> Option<AgentClient<Box<dyn keys::agent::client::AgentStream + Send + Unpin>>> {
    AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
        .await
        .ok()
        .map(AgentClient::dynamic)
}
