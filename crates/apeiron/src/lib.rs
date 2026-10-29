//! The public Rust API of Apeiron (`docs/API.md` §8).
//!
//! Applications use only this crate. Every request and result is plain data
//! that can be written as JSON; long operations run as [`Job`]s.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod error;
mod job;
pub mod json;
mod plugins;
mod repo;
mod spec;
mod types;

/// Version of the API contract in `docs/API.md` §8.
pub const API_VERSION: &str = "0.1";

pub use apeiron_core::{
    GcOptions as GcRequest, GcReport, KeyInfo, KeyKind, ObjectId, RefOp, ReflogEntry, Remote,
    RepairReport, Time, TransferReport, VerifyReport,
};
pub use apeiron_git::{
    ExportReport as GitExportReport, ImportReport as GitImportReport, RefUpdate as GitRefUpdate,
    Skipped as GitSkipped,
};
pub use apeiron_plugin::{Added as PluginAdded, PluginInfo};
pub use apeiron_services::PluginMerge;
pub use apeiron_services::{
    ArchiveFormat, BackupOptions as BackupRequest, BackupOutcome as BackupReport, Change,
    ChangeKind, Conflict, ConflictKind, EntryInfo, ExportOutcome as ExportReport, Kind, Name,
    Phase, Progress, RestoreOutcome as RestoreReport, RestoreProblem,
    RestoreTestOutcome as RestoreTestReport, SyncAction, SyncMode, SyncOutcome as SyncReport,
    Warning, WarningKind,
};
pub use apeiron_services::{MergeAction, MergeOutcome as MergeReport};
pub use apeiron_services::{Query, SearchHit, SearchOutcome as SearchReport, TextLine};
pub use error::{Error, ErrorKind, Result};
pub use job::Job;
pub use plugins::{add_plugin, plugin_dir, plugins, remove_plugin};
pub use repo::Repository;
pub use types::{
    AddedKey, CloneReport, CloneRequest, ClonedRef, DeleteRefRequest, DiffRequest, ExportRequest,
    GitRequest, ListRequest, LogPage, LogRequest, RefInfo, RepoInfo, RestoreRequest,
    RestoreTestRequest, SnapshotInfo, SyncRequest, TransferRequest,
};
pub use types::{CreateRefRequest, MergeRequest, SearchRequest, TextDiff};
