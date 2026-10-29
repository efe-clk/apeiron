//! Importing a Git repository with its history and exporting it back, with
//! the same object ids (GIT.md, ADR-021).

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod export;
mod import;
mod names;
mod raw;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use export::export;
pub use import::import;

/// Called after each commit or tag with the number done so far; an error,
/// such as [`Error::Cancelled`], stops the operation.
pub type OnObject<'a> = &'a mut dyn FnMut(u64) -> Result<()>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Core(#[from] apeiron_core::Error),
    #[error("Git repository {path}: {message}")]
    Git { path: PathBuf, message: String },
    #[error("{0}")]
    Unsupported(String),
    #[error("Git object {id} cannot be read: {reason}")]
    InvalidObject { id: String, reason: String },
    #[error("cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn git_error(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Git {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// A ref that was moved, with its Git and Apeiron names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefUpdate {
    /// The full Git name, such as `refs/heads/main`.
    pub git: String,
    /// The Apeiron ref, such as `git/heads/main`.
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// The Git object id.
    pub id: String,
    pub snapshot: apeiron_core::ObjectId,
    /// What the ref pointed to before; `None` if it is new.
    pub previous: Option<apeiron_core::ObjectId>,
}

/// A ref or object that was left out, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    /// Commits stored as new snapshots.
    pub commits: u64,
    /// Annotated tags stored as new snapshots.
    pub tags: u64,
    /// Refs created or moved.
    pub refs: Vec<RefUpdate>,
    pub skipped: Vec<Skipped>,
    /// Refs under the prefix whose branch or tag is gone from Git; they are
    /// kept.
    pub stale: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    /// Commits written.
    pub commits: u64,
    /// Annotated tags written.
    pub tags: u64,
    /// Branches and tags pointed at their objects.
    pub refs: Vec<RefUpdate>,
    pub skipped: Vec<Skipped>,
    /// Git ids that a rebuilt object did not get again (GIT.md §5).
    pub inexact: Vec<String>,
}

/// `meta` keys of snapshots made from Git (GIT.md §2.2).
pub(crate) mod keys {
    pub(crate) const ID: &str = "git.id";
    pub(crate) const KIND: &str = "git.kind";
    pub(crate) const HEADERS: &str = "git.headers";
    pub(crate) const HEADERS_HEX: &str = "git.headers-hex";
    pub(crate) const MESSAGE_HEX: &str = "git.message-hex";
    pub(crate) const NO_MESSAGE: &str = "git.no-message";
    pub(crate) const GITLINKS: &str = "git.gitlinks";
}

/// Opens a Git repository and checks it can be used.
pub(crate) fn open_git(path: &Path) -> Result<gix::Repository> {
    let repo = gix::open(path).map_err(|e| git_error(path, e))?;
    check_git(path, &repo)?;
    Ok(repo)
}

pub(crate) fn check_git(path: &Path, repo: &gix::Repository) -> Result<()> {
    if repo.object_hash() != gix::hash::Kind::Sha1 {
        return Err(Error::Unsupported(format!(
            "{} uses SHA-256; only SHA-1 repositories are supported",
            path.display()
        )));
    }
    if repo.is_shallow() {
        return Err(Error::Unsupported(format!(
            "{} is a shallow clone; its oldest commits are missing",
            path.display()
        )));
    }
    Ok(())
}

/// The Git ref folders that are imported (GIT.md §3).
const KINDS: [&str; 3] = ["heads", "tags", "remotes"];

/// `<prefix>/heads/…`, `<prefix>/tags/…` or `<prefix>/remotes/…` for a Git
/// ref name, if Apeiron takes it.
pub(crate) fn apeiron_name(
    prefix: &str,
    git: &[u8],
) -> Option<std::result::Result<String, String>> {
    let (kind, rest) = KINDS.iter().find_map(|kind| {
        let rest = git.strip_prefix(format!("refs/{kind}/").as_bytes())?;
        Some((*kind, rest))
    })?;
    let name = format!("{prefix}/{kind}/{}", names::escape(rest));
    Some(match apeiron_core::validate_ref_name(&name) {
        Ok(()) => Ok(name),
        Err(_) => Err(format!("{name:?} is not a valid ref name")),
    })
}

/// The Git name for an Apeiron ref under `prefix`, if it is a branch, tag
/// or remote-tracking branch.
pub(crate) fn git_name(prefix: &str, name: &str) -> Option<String> {
    let rest = name.strip_prefix(prefix)?.strip_prefix('/')?;
    let (kind, rest) = rest.split_once('/')?;
    if !KINDS.contains(&kind) {
        return None;
    }
    let bytes = names::unescape(rest)?;
    let rest = String::from_utf8(bytes).ok()?;
    Some(format!("refs/{kind}/{rest}"))
}
