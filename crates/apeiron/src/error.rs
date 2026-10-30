//! Errors with stable codes (API.md §8.6).

use std::fmt;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Kind of an error. The numbers are shared with the C ABI and every SDK
/// and never change meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
#[repr(u16)]
pub enum ErrorKind {
    Io = 1,
    NotFound = 2,
    AlreadyExists = 3,
    InvalidInput = 4,
    Ambiguous = 5,
    Corrupt = 6,
    Locked = 7,
    Conflict = 8,
    Cancelled = 9,
    Unsupported = 10,
    Internal = 11,
    /// The repository is encrypted and the password or recovery key is
    /// missing or wrong.
    Key = 12,
}

impl ErrorKind {
    pub fn code(self) -> u16 {
        self as u16
    }
}

/// An error: a kind with a stable code, a message and, when it applies, a path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Error {
    pub code: u16,
    pub kind: ErrorKind,
    pub message: String,
    pub path: Option<PathBuf>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            code: kind.code(),
            kind,
            message: message.into(),
            path: None,
        }
    }

    fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    fn io(path: PathBuf, source: &io::Error) -> Self {
        let kind = match source.kind() {
            io::ErrorKind::NotFound => ErrorKind::NotFound,
            io::ErrorKind::AlreadyExists => ErrorKind::AlreadyExists,
            _ => ErrorKind::Io,
        };
        Error::new(kind, format!("{}: {source}", path.display())).with_path(path)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<apeiron_core::Error> for Error {
    fn from(e: apeiron_core::Error) -> Self {
        use apeiron_core::Error as E;
        let message = e.to_string();
        match e {
            E::Io { path, source } => Error::io(path, &source),
            E::Input(_) | E::Output(_) => Error::new(ErrorKind::Io, message),
            E::Corrupt { path, .. } => Error::new(ErrorKind::Corrupt, message).with_path(path),
            E::InvalidRepository { path, .. } => {
                Error::new(ErrorKind::InvalidInput, message).with_path(path)
            }
            E::UnsupportedVersion { path, .. } => {
                Error::new(ErrorKind::Unsupported, message).with_path(path)
            }
            E::ObjectNotFound(_) | E::KeyNotFound(_) | E::RefNotFound(_) | E::RemoteNotFound(_) => {
                Error::new(ErrorKind::NotFound, message)
            }
            E::RemoteExists(_) => Error::new(ErrorKind::AlreadyExists, message),
            E::Cancelled => Error::new(ErrorKind::Cancelled, message),
            E::UnsupportedRemote(_) => Error::new(ErrorKind::Unsupported, message),
            E::OtherRepository(path) => {
                Error::new(ErrorKind::InvalidInput, message).with_path(path)
            }
            E::KeyRequired(path) => Error::new(ErrorKind::Key, message).with_path(path),
            E::WrongKey => Error::new(ErrorKind::Key, message),
            E::RefLocked { lock, .. } | E::RepoLocked(lock) => {
                Error::new(ErrorKind::Locked, message).with_path(lock)
            }
            E::RefConflict { .. }
            | E::RefNameConflict(_)
            | E::RemoteAhead(_)
            | E::LocalAhead(_)
            | E::Retained { .. } => Error::new(ErrorKind::Conflict, message),
            E::NotASnapshot(_)
            | E::WrongObjectType { .. }
            | E::ObjectTooLarge(_)
            | E::InvalidChunkerParams(_)
            | E::InvalidRefName(_)
            | E::InvalidObject { .. }
            | E::NotEncrypted
            | E::LastKey
            | E::InvalidRemote(_)
            | E::InvalidRetention(_) => Error::new(ErrorKind::InvalidInput, message),
            _ => Error::new(ErrorKind::Internal, message),
        }
    }
}

impl From<apeiron_services::Error> for Error {
    fn from(e: apeiron_services::Error) -> Self {
        use apeiron_services::Error as E;
        let message = e.to_string();
        match e {
            E::Core(e) => e.into(),
            E::Io { path, source } => Error::io(path, &source),
            E::Cancelled => Error::new(ErrorKind::Cancelled, message),
            E::NotFound(_) => Error::new(ErrorKind::NotFound, message),
            E::AlreadyExists(path) => Error::new(ErrorKind::AlreadyExists, message).with_path(path),
            E::InvalidInput(_) => Error::new(ErrorKind::InvalidInput, message),
            _ => Error::new(ErrorKind::Internal, message),
        }
    }
}

impl From<apeiron_plugin::Error> for Error {
    fn from(e: apeiron_plugin::Error) -> Self {
        use apeiron_plugin::Error as E;
        let message = e.to_string();
        match e {
            E::Io { path, source } => Error::io(path, &source),
            E::NotFound(_) | E::NoFolder => Error::new(ErrorKind::NotFound, message),
            E::Invalid(_) | E::Failed { .. } => Error::new(ErrorKind::InvalidInput, message),
        }
    }
}

impl From<apeiron_git::Error> for Error {
    fn from(e: apeiron_git::Error) -> Self {
        use apeiron_git::Error as E;
        let message = e.to_string();
        match e {
            E::Core(e) => e.into(),
            E::Git { path, .. } => Error::new(ErrorKind::Io, message).with_path(path),
            E::Unsupported(_) => Error::new(ErrorKind::Unsupported, message),
            E::InvalidObject { .. } => Error::new(ErrorKind::Corrupt, message),
            E::Cancelled => Error::new(ErrorKind::Cancelled, message),
            _ => Error::new(ErrorKind::Internal, message),
        }
    }
}
