//! Share links (SHARE.md): anyone who holds one may read one version of a
//! backup, or a folder or file in it, until the link ends.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use apeiron::{Error, ErrorKind, ObjectId};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Days a link lasts when none are given, and at most (SHARE.md §2).
pub const DEFAULT_DAYS: u32 = 7;
pub const MAX_DAYS: u32 = 30;

/// One link, as kept in the shares file (SHARE.md §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    /// The first 16 characters of `hash`; names the link to its owner.
    pub id: String,
    /// SHA-256 of the link's secret, hex; the secret itself is not kept.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub hash: String,
    pub owner: String,
    pub snapshot: ObjectId,
    /// The shared path inside the snapshot; empty for all of it.
    #[serde(default)]
    pub path: String,
    pub made: String,
    pub expires: String,
}

impl Share {
    /// Whether the link has not ended by `now`.
    fn live(&self, now: Timestamp) -> bool {
        self.expires.parse::<Timestamp>().is_ok_and(|t| now < t)
    }

    /// The share without its hash, for showing.
    pub fn public(&self) -> Share {
        Share {
            hash: String::new(),
            ..self.clone()
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    shares: Vec<Share>,
}

fn io_error(path: &Path, e: io::Error) -> Error {
    Error::new(ErrorKind::Io, format!("{}: {e}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

/// The error for a link that is unknown, has ended or was removed; the
/// same for all, so a link holder learns nothing more.
pub fn gone() -> Error {
    Error::new(
        ErrorKind::NotFound,
        "this link does not exist, has expired or was removed",
    )
}

fn now() -> Timestamp {
    Timestamp::now()
        .round(jiff::Unit::Second)
        .unwrap_or_else(|_| Timestamp::now())
}

/// The shares file.
#[derive(Clone, Debug)]
pub struct Shares {
    file: PathBuf,
}

impl Shares {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Shares { file: file.into() }
    }

    /// `shares.json` in the state folder (SHARE.md §3).
    pub fn default_file() -> Option<PathBuf> {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|d| d.join("apeiron").join("shares.json"))
    }

    fn read(&self) -> Result<Stored, Error> {
        match fs::read(&self.file) {
            Ok(data) => serde_json::from_slice(&data).map_err(|e| {
                Error::new(ErrorKind::Corrupt, format!("{}: {e}", self.file.display()))
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Stored::default()),
            Err(e) => Err(io_error(&self.file, e)),
        }
    }

    /// Changes the file under a lock, so the server and `apeiron web share`
    /// do not undo each other; ended links are dropped on the way. The new
    /// file replaces the old one whole.
    fn change<T>(&self, f: impl FnOnce(&mut Vec<Share>) -> Result<T, Error>) -> Result<T, Error> {
        let path = &self.file;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
        }
        let lock_path = path.with_extension("lock");
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| io_error(&lock_path, e))?;
        lock.lock().map_err(|e| io_error(&lock_path, e))?;
        let mut file = self.read()?;
        let now = now();
        file.shares.retain(|s| s.live(now));
        let out = f(&mut file.shares)?;
        let data = serde_json::to_vec_pretty(&file)
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let new = path.with_extension("new");
        let write = || -> io::Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut f = options.open(&new)?;
            f.write_all(&data)?;
            f.sync_all()?;
            fs::rename(&new, path)
        };
        write().map_err(|e| io_error(path, e))?;
        Ok(out)
    }

    /// Makes a link to `path` in `snapshot` for `owner`, lasting `days`;
    /// returns it with its secret, which is shown only now.
    pub fn add(
        &self,
        owner: &str,
        snapshot: ObjectId,
        path: &str,
        days: u32,
    ) -> Result<(Share, String), Error> {
        if !(1..=MAX_DAYS).contains(&days) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("a link lasts 1 to {MAX_DAYS} days"),
            ));
        }
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let secret = hex(&raw);
        let hash = hash(&secret);
        let made = now();
        let expires = made + SignedDuration::from_hours(24 * i64::from(days));
        let share = Share {
            id: hash[..16].to_owned(),
            hash,
            owner: owner.to_owned(),
            snapshot,
            path: path.trim_matches('/').to_owned(),
            made: made.to_string(),
            expires: expires.to_string(),
        };
        self.change(|shares| {
            shares.push(share.clone());
            Ok(())
        })?;
        Ok((share, secret))
    }

    /// The links that have not ended, oldest first.
    pub fn list(&self) -> Result<Vec<Share>, Error> {
        let now = now();
        let mut shares = self.read()?.shares;
        shares.retain(|s| s.live(now));
        Ok(shares)
    }

    /// The live link whose secret is `secret`.
    pub fn find(&self, secret: &str) -> Result<Share, Error> {
        let hash = hash(secret);
        self.list()?
            .into_iter()
            .find(|s| s.hash == hash)
            .ok_or_else(gone)
    }

    /// Removes the link `id` if `may` allows it; returns it.
    pub fn remove(&self, id: &str, may: impl FnOnce(&Share) -> bool) -> Result<Share, Error> {
        let missing = || Error::new(ErrorKind::NotFound, format!("no link {id}"));
        self.change(|shares| {
            let index = shares.iter().position(|s| s.id == id).ok_or_else(missing)?;
            if !may(&shares[index]) {
                return Err(missing());
            }
            Ok(shares.remove(index))
        })
    }
}
