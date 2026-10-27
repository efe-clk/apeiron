use std::io;
use std::path::{Path, PathBuf};

use crate::id::ObjectId;

/// Errors returned by `apeiron-core`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read input: {0}")]
    Input(#[source] io::Error),
    #[error("corrupt data in {path}: {reason}")]
    Corrupt { path: PathBuf, reason: String },
    #[error("invalid repository at {path}: {reason}")]
    InvalidRepository { path: PathBuf, reason: String },
    #[error("unsupported format version {version} in {path}")]
    UnsupportedVersion { path: PathBuf, version: u32 },
    #[error("object {0} not found")]
    ObjectNotFound(ObjectId),
    #[error("object {0} is not a snapshot")]
    NotASnapshot(ObjectId),
    #[error("object too large: {0} bytes")]
    ObjectTooLarge(usize),
    #[error("invalid chunker parameters: {0}")]
    InvalidChunkerParams(String),
    #[error("invalid ref name {0:?}")]
    InvalidRefName(String),
    #[error("ref {0:?} conflicts with an existing ref of a different depth")]
    RefNameConflict(String),
    #[error("ref {name:?} is locked by another writer ({lock})")]
    RefLocked { name: String, lock: PathBuf },
    #[error("ref {name:?} changed concurrently: expected {expected:?}, found {actual:?}")]
    RefConflict {
        name: String,
        expected: Option<ObjectId>,
        actual: Option<ObjectId>,
    },
    #[error("random number generator failed: {0}")]
    Random(getrandom::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn corrupt(path: &Path, reason: impl Into<String>) -> Self {
        Error::Corrupt {
            path: path.to_path_buf(),
            reason: reason.into(),
        }
    }
}

/// Attaches the path an I/O error happened at.
pub(crate) trait IoContext<T> {
    fn at(self, path: &Path) -> Result<T>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn at(self, path: &Path) -> Result<T> {
        self.map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}
