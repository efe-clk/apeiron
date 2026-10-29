//! Requests and results of the short operations and of restore (API.md §8.3).

use std::path::PathBuf;

use apeiron_core::{ObjectId, Snapshot, Time, TransferReport};
use apeiron_services::{ArchiveFormat, ChangeKind, Name, Query, SyncMode};
use serde::{Deserialize, Serialize};

/// Facts about an open repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    pub path: PathBuf,
    pub format_version: u32,
    pub api_version: String,
    pub chunk_min: u32,
    pub chunk_avg: u32,
    pub chunk_max: u32,
    pub encrypted: bool,
    /// Index and pack files found damaged when the repository was opened;
    /// `verify` checks everything and `repair` replaces damaged indexes.
    pub damaged_files: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreRequest {
    /// A snapshot spec (API.md §8.4).
    pub snapshot: String,
    /// A new or empty folder.
    pub target: PathBuf,
    /// A path inside the snapshot; empty or absent for everything.
    #[serde(default)]
    pub path: Name,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportRequest {
    /// A snapshot spec (API.md §8.4).
    pub snapshot: String,
    /// The archive file to create; it must not exist.
    pub output: PathBuf,
    /// `tar`, `zip` or `7z`; taken from the file extension when absent.
    #[serde(default)]
    pub format: Option<ArchiveFormat>,
    /// A path inside the snapshot; empty or absent for everything.
    #[serde(default)]
    pub path: Name,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRequest {
    /// A snapshot spec where the history starts.
    pub start: String,
    /// Maximum number of snapshots in one page.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

/// One page of history, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogPage {
    pub snapshots: Vec<SnapshotInfo>,
    /// Where the next page starts; `None` at the first snapshot.
    pub next: Option<ObjectId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub id: ObjectId,
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub time: Time,
    pub author: Option<String>,
    pub host: Option<String>,
    pub message: Option<String>,
    pub meta: Vec<(String, String)>,
}

impl SnapshotInfo {
    pub(crate) fn new(id: ObjectId, s: Snapshot) -> Self {
        SnapshotInfo {
            id,
            tree: s.tree,
            parents: s.parents,
            time: s.time,
            author: s.author,
            host: s.host,
            message: s.message,
            meta: s.meta,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRequest {
    pub snapshot: String,
    #[serde(default)]
    pub path: Name,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffRequest {
    pub from: String,
    pub to: String,
}

/// A changed path with the difference a plugin shows for it (TEXT.md §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextDiff {
    pub path: Name,
    pub kind: ChangeKind,
    /// The changed lines; absent when no plugin handles the file, it is not
    /// text or it is a folder or a link.
    pub text: Option<String>,
    /// Why the plugin could not show the difference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefInfo {
    pub name: String,
    pub snapshot: ObjectId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteRefRequest {
    pub name: String,
    /// The snapshot the ref must still point to.
    pub expected: ObjectId,
}

/// A new ref and the snapshot it starts at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRefRequest {
    pub name: String,
    /// A snapshot spec (API.md §8.4).
    pub snapshot: String,
}

/// Which snapshot to merge into which ref.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeRequest {
    /// A snapshot spec (API.md §8.4), usually a branch name.
    pub from: String,
    /// The ref that moves; `main` by default.
    #[serde(default = "default_into")]
    pub into: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
}

fn default_into() -> String {
    "main".to_owned()
}

impl MergeRequest {
    pub fn new(from: impl Into<String>, into: impl Into<String>) -> Self {
        MergeRequest {
            from: from.into(),
            into: into.into(),
            message: None,
            author: None,
            dry_run: false,
        }
    }
}

/// A password or recovery key added to an encrypted repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedKey {
    pub id: String,
    /// The new recovery key, shown to the user once; absent for a password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreTestRequest {
    /// A snapshot spec (API.md §8.4).
    pub snapshot: String,
    /// How much to restore; at least one file is always restored.
    #[serde(default = "default_sample_bytes")]
    pub sample_bytes: u64,
}

fn default_sample_bytes() -> u64 {
    100 * 1024 * 1024
}

impl RestoreTestRequest {
    pub fn new(snapshot: impl Into<String>) -> Self {
        RestoreTestRequest {
            snapshot: snapshot.into(),
            sample_bytes: default_sample_bytes(),
        }
    }
}

/// Which ref to push to or pull from which remote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRequest {
    /// The name of a remote.
    pub remote: String,
    #[serde(rename = "ref", default = "default_ref")]
    pub ref_name: String,
}

fn default_ref() -> String {
    "main".to_owned()
}

impl TransferRequest {
    pub fn new(remote: impl Into<String>, ref_name: impl Into<String>) -> Self {
        TransferRequest {
            remote: remote.into(),
            ref_name: ref_name.into(),
        }
    }
}

/// Where to clone a remote copy from and to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneRequest {
    /// A folder path or an `sftp://` or `s3://` URL, as for a remote.
    pub url: String,
    /// The new repository: a folder that does not exist or is empty.
    pub path: PathBuf,
    /// The password or recovery key of an encrypted copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// What `clone` copied, ref by ref.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneReport {
    pub refs: Vec<ClonedRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClonedRef {
    pub name: String,
    #[serde(flatten)]
    pub report: TransferReport,
}

/// Which folder to sync with which remote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRequest {
    pub folder: PathBuf,
    /// The name of a remote.
    pub remote: String,
    /// The ref on both sides; `sync/<folder name>` when absent.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub ref_name: Option<String>,
    #[serde(default)]
    pub mode: SyncMode,
    /// Shows what would change without changing the folder, a ref or the
    /// remote.
    #[serde(default)]
    pub dry_run: bool,
}

impl SyncRequest {
    pub fn new(folder: impl Into<PathBuf>, remote: impl Into<String>) -> Self {
        SyncRequest {
            folder: folder.into(),
            remote: remote.into(),
            ref_name: None,
            mode: SyncMode::TwoWay,
            dry_run: false,
        }
    }
}

/// Which Git repository to import from or export to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRequest {
    pub path: PathBuf,
    /// The folder of refs for the repository; `git` by default.
    #[serde(default = "default_git_prefix")]
    pub prefix: String,
}

fn default_git_prefix() -> String {
    "git".to_owned()
}

impl GitRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        GitRequest {
            path: path.into(),
            prefix: default_git_prefix(),
        }
    }
}

/// What to search for and where (SEARCH.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    /// A snapshot spec (API.md §8.4); when absent, `main`, or with
    /// `history` every ref.
    #[serde(default)]
    pub snapshot: Option<String>,
    /// Also search every snapshot before it.
    #[serde(default)]
    pub history: bool,
    #[serde(flatten)]
    pub query: Query,
    /// Most results returned.
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    1000
}

impl SearchRequest {
    pub fn new(query: Query) -> Self {
        SearchRequest {
            snapshot: None,
            history: false,
            query,
            limit: default_search_limit(),
        }
    }
}
