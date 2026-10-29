//! Remote copies of a repository: the `remotes` file and the stores they
//! live in (FORMAT.md §15.1, §15.2). Push and pull are in `repo::transfer`.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::ObjectId;
use crate::lock::RepoLock;
use crate::refs::{RefStore, validate_ref_name};

/// A named remote copy of the repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRemotes {
    #[serde(default)]
    remote: BTreeMap<String, RawRemote>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRemote {
    url: String,
}

/// Remote names follow the ref name rules, without `/`.
pub(crate) fn validate_remote_name(name: &str) -> Result<()> {
    if name.contains('/') || validate_ref_name(name).is_err() {
        return Err(Error::InvalidRemote(format!(
            "invalid remote name {name:?}"
        )));
    }
    Ok(())
}

/// Checks a remote URL. Only folders are supported so far; a folder must be
/// given as an absolute path.
pub(crate) fn validate_url(url: &str) -> Result<()> {
    if url.contains("://") {
        return Err(Error::UnsupportedRemote(url.to_owned()));
    }
    if !Path::new(url).is_absolute() {
        return Err(Error::InvalidRemote(format!(
            "the folder {url:?} is not an absolute path"
        )));
    }
    Ok(())
}

/// Reads the `remotes` file; a missing file has no remotes.
pub(crate) fn read_remotes(path: &Path) -> Result<Vec<Remote>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).at(path),
    };
    let raw: RawRemotes = toml::from_str(&text).map_err(|e| Error::InvalidRepository {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    Ok(raw
        .remote
        .into_iter()
        .map(|(name, r)| Remote { name, url: r.url })
        .collect())
}

/// Replaces the `remotes` file durably.
pub(crate) fn write_remotes(tmp_dir: &Path, path: &Path, remotes: &[Remote]) -> Result<()> {
    let raw = RawRemotes {
        remote: remotes
            .iter()
            .map(|r| (r.name.clone(), RawRemote { url: r.url.clone() }))
            .collect(),
    };
    // Serializing plain strings cannot fail.
    let text = toml::to_string(&raw).unwrap_or_default();
    fsutil::write_durable(tmp_dir, path, text.as_bytes())
}

/// The operations every kind of remote offers (FORMAT.md §15.2). Paths are
/// relative to the repository root and separated by `/`.
pub trait Store {
    /// Where `path` is, for messages.
    fn location(&self, path: &str) -> PathBuf;

    /// Reads a whole file; `None` if it does not exist.
    fn read(&self, path: &str) -> Result<Option<Vec<u8>>>;

    /// Reads `len` bytes of a file starting at `offset`.
    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>>;

    /// The names of the entries directly in a folder; empty if it does not
    /// exist.
    fn list(&self, dir: &str) -> Result<Vec<String>>;

    /// Writes a new file atomically. A file already there is left as it is:
    /// every file written this way is named by its content or id.
    fn write_new(&self, path: &str, bytes: &[u8]) -> Result<()>;

    /// Creates the folders of a new copy; a store without folders does
    /// nothing.
    fn create_dirs(&self, dirs: &[&str]) -> Result<()>;

    /// Deletes a file; one that does not exist is not an error.
    fn delete(&self, path: &str) -> Result<()>;

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>>;

    /// Points the ref `name` at `new` only if it points at `expected`.
    fn swap_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<()>;
}

/// A remote in a folder, such as one on an external disk. It is a complete
/// repository that also keeps reflogs.
#[derive(Debug)]
pub struct FolderStore {
    root: PathBuf,
    refs: RefStore,
    /// Held shared while the folder already holds a repository, so that
    /// garbage collection run on it directly waits (FORMAT.md §12).
    _lock: Option<RepoLock>,
}

impl FolderStore {
    pub fn open(root: &Path) -> Result<Self> {
        let lock = if root.join("config").is_file() {
            Some(RepoLock::shared(root)?)
        } else {
            None
        };
        Ok(FolderStore {
            root: root.to_path_buf(),
            refs: RefStore::new(root.join("refs"), root.join("logs"), root.join("tmp")),
            _lock: lock,
        })
    }
}

impl Store for FolderStore {
    fn location(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let path = self.location(path);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).at(&path),
        }
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        let path = self.location(path);
        let mut file = File::open(&path).at(&path)?;
        file.seek(SeekFrom::Start(offset)).at(&path)?;
        let mut bytes = vec![0u8; len as usize];
        file.read_exact(&mut bytes).at(&path)?;
        Ok(bytes)
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        let dir = self.location(dir);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).at(&dir),
        };
        let mut names = Vec::new();
        for entry in entries {
            names.push(entry.at(&dir)?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn write_new(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let tmp = self.root.join("tmp");
        fsutil::ensure_dir(&tmp)?;
        fsutil::write_durable_new(&tmp, &self.location(path), bytes)
    }

    fn create_dirs(&self, dirs: &[&str]) -> Result<()> {
        dirs.iter()
            .try_for_each(|dir| fsutil::ensure_dir(&self.location(dir)))
    }

    fn delete(&self, path: &str) -> Result<()> {
        let path = self.location(path);
        match fsutil::checkpoint().and_then(|()| fs::remove_file(&path)) {
            Ok(()) => fsutil::sync_dir(fsutil::parent_of(&path)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).at(&path),
        }
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        self.refs.read(name)
    }

    fn swap_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<()> {
        self.refs.update(name, expected, Some(new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_file_round_trips_and_names_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remotes");
        assert!(read_remotes(&path).unwrap().is_empty());
        let remotes = vec![
            Remote {
                name: "disk".into(),
                url: "/mnt/disk/apeiron".into(),
            },
            Remote {
                name: "nas".into(),
                url: "/srv/apeiron".into(),
            },
        ];
        let absolute = dir.path().to_str().unwrap();
        assert!(validate_url(absolute).is_ok());
        write_remotes(dir.path(), &path, &remotes).unwrap();
        assert_eq!(read_remotes(&path).unwrap(), remotes);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("[remote.disk]\nurl = \"/mnt/disk/apeiron\"")
        );

        assert!(validate_remote_name("disk-2").is_ok());
        for bad in ["", "a/b", ".x", "a b", "Disk"] {
            assert!(validate_remote_name(bad).is_err(), "{bad:?}");
        }
        assert!(matches!(
            validate_url("sftp://nas/srv"),
            Err(Error::UnsupportedRemote(_))
        ));
        assert!(matches!(
            validate_url("relative/dir"),
            Err(Error::InvalidRemote(_))
        ));
    }
}
