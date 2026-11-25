//! A remote on any storage rclone reaches (FORMAT.md §15.1): `rclone:<remote>:<path>`.
//! rclone serves the place over SFTP on this computer only, with a host key
//! and a client key made for this one connection, and the SFTP store of this
//! crate speaks to it, so refs are replaced as on any SFTP server.

use std::fs;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use apeiron_core::{Error, ObjectId, Result, SftpUrl, Store};
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::{Ed25519Keypair, PrivateKey};

use crate::{SftpStore, SshOptions};

/// A remote served by `rclone serve sftp` while it is open.
pub struct RcloneStore {
    inner: SftpStore,
    rclone: Child,
    keys: PathBuf,
    target: String,
}

fn failed(target: &str, message: impl std::fmt::Display) -> Error {
    Error::InvalidRemote(format!("rclone:{target}: {message}"))
}

fn new_key() -> Result<PrivateKey> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| Error::InvalidRemote(format!("random bytes: {e}")))?;
    Ok(PrivateKey::from(Ed25519Keypair::from_seed(&seed)))
}

fn write_private(path: &Path, key: &PrivateKey) -> Result<()> {
    let text = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| Error::InvalidRemote(e.to_string()))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(path)
        .map_err(|e| Error::InvalidRemote(format!("{}: {e}", path.display())))?;
    file.write_all(text.as_bytes())
        .map_err(|e| Error::InvalidRemote(format!("{}: {e}", path.display())))
}

fn public(key: &PrivateKey) -> Result<String> {
    key.public_key()
        .to_openssh()
        .map_err(|e| Error::InvalidRemote(e.to_string()))
}

impl RcloneStore {
    /// Starts rclone on `target`, `<remote>:<path>` as rclone names it, and
    /// connects to it. `work` is a folder of this computer for the keys of
    /// the connection, removed again when the store is dropped.
    pub fn connect(target: &str, work: &Path) -> Result<Self> {
        let keys = work.join(format!(".apeiron-rclone-{}", std::process::id()));
        let _ = fs::remove_dir_all(&keys);
        fs::create_dir_all(&keys).map_err(|e| failed(target, e))?;
        let (host, client) = (new_key()?, new_key()?);
        let host_file = keys.join("host");
        let client_file = keys.join("client");
        write_private(&host_file, &host)?;
        write_private(&client_file, &client)?;
        let authorized = keys.join("authorized_keys");
        fs::write(&authorized, public(&client)?).map_err(|e| failed(target, e))?;

        // A free port of this computer, for rclone to listen on.
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| failed(target, e))?
            .port();
        fs::write(
            keys.join("known_hosts"),
            format!("[127.0.0.1]:{port} {}\n", public(&host)?),
        )
        .map_err(|e| failed(target, e))?;
        let rclone = Command::new("rclone")
            .args(["serve", "sftp", target, "--addr"])
            .arg(format!("127.0.0.1:{port}"))
            .arg("--key")
            .arg(&host_file)
            .arg("--authorized-keys")
            .arg(&authorized)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| failed(target, format!("cannot run rclone: {e}")))?;
        let mut rclone = rclone;
        let cleanup = |rclone: &mut Child| {
            let _ = rclone.kill();
            let _ = rclone.wait();
            let _ = fs::remove_dir_all(&keys);
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if Instant::now() > deadline || rclone.try_wait().ok().flatten().is_some() {
                cleanup(&mut rclone);
                return Err(failed(
                    target,
                    "rclone did not start; is the remote in `rclone config`?",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let url = SftpUrl {
            user: Some("apeiron".to_owned()),
            host: "127.0.0.1".to_owned(),
            port,
            path: "/".to_owned(),
        };
        let options = SshOptions {
            known_hosts: keys.join("known_hosts"),
            identity_files: vec![client_file],
            use_agent: false,
        };
        match SftpStore::connect(&url, &options) {
            Ok(inner) => Ok(RcloneStore {
                inner,
                rclone,
                keys,
                target: target.to_owned(),
            }),
            Err(e) => {
                cleanup(&mut rclone);
                Err(e)
            }
        }
    }
}

impl Drop for RcloneStore {
    fn drop(&mut self) {
        let _ = self.rclone.kill();
        let _ = self.rclone.wait();
        let _ = fs::remove_dir_all(&self.keys);
    }
}

impl Store for RcloneStore {
    fn location(&self, path: &str) -> PathBuf {
        PathBuf::from(format!(
            "rclone:{}/{path}",
            self.target.trim_end_matches('/')
        ))
    }

    fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.inner.read(path)
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        self.inner.read_range(path, offset, len)
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        self.inner.list(dir)
    }

    fn write_new(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.inner.write_new(path, bytes)
    }

    fn create_dirs(&self, dirs: &[&str]) -> Result<()> {
        self.inner.create_dirs(dirs)
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.inner.delete(path)
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        self.inner.read_ref(name)
    }

    fn list_refs(&self) -> Result<Vec<String>> {
        self.inner.list_refs()
    }

    fn swap_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<()> {
        self.inner.swap_ref(name, expected, new)
    }
}
