//! Requests and results of the short operations and of restore (API.md §8.3).

use std::path::PathBuf;

use apeiron_core::{ObjectId, Snapshot, Time};
use apeiron_services::{ArchiveFormat, Name};
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
