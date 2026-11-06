//! The public Rust API of Apeiron (`docs/API.md` §8).
//!
//! Applications use only this crate. Every request and result is plain data
//! that can be written as JSON; long operations run as [`Job`]s.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod conflicts;
mod error;
mod job;
pub mod json;
mod plugins;
mod repo;
mod report;
mod saved;
mod spec;
mod types;

/// Version of the API contract in `docs/API.md` §8.
pub const API_VERSION: &str = "0.1";

pub use apeiron_core::{
    CopyRepairReport, GcOptions as GcRequest, GcReport, Hold, KeyInfo, KeyKind, LegalHold, Lift,
    MAX_RETENTION_DAYS, ObjectId, RefOp, ReflogEntry, Remote, RepairReport, RetentionRule, Time,
    TransferReport, VerifyReport,
};
pub use apeiron_git::{
    ExportReport as GitExportReport, ImportReport as GitImportReport, PushResult as GitPushResult,
    PushedRef as GitPushedRef, RefUpdate as GitRefUpdate, ServedRef as GitServedRef,
    Skipped as GitSkipped,
};
pub use apeiron_plugin::{Added as PluginAdded, PluginInfo};
pub use apeiron_services::PluginMerge;
pub use apeiron_services::{
    AddFileOptions as AddFileRequest, AddFileOutcome as AddFileReport, ArchiveFormat,
    BackupOptions as BackupRequest, BackupOutcome as BackupReport, Change, ChangeKind, Conflict,
    ConflictCopy, ConflictKind, EntryInfo, ExportOutcome as ExportReport, Keep, Kind, Name, Phase,
    Progress, PruneOptions as PruneRequest, PruneOutcome as PruneReport, PrunedKept, PrunedRemoved,
    RestoreOutcome as RestoreReport, RestoreProblem, RestoreTestOutcome as RestoreTestReport,
    SwitchOutcome as SwitchReport, SyncAction, SyncMode, SyncOutcome as SyncReport, Warning,
    WarningKind,
};
pub use apeiron_services::{Duplicate, Query, SearchHit, SearchOutcome as SearchReport, TextLine};

/// The tags of the file at `path` on disk (FORMAT.md §6.3); Linux only.
pub fn file_tags(path: impl AsRef<std::path::Path>) -> Result<Vec<String>> {
    Ok(apeiron_services::file_tags(path.as_ref())?)
}

/// Sets the tags of the file at `path` on disk; none removes them. The next
/// backup keeps them (FORMAT.md §6.3); Linux only.
pub fn set_file_tags(path: impl AsRef<std::path::Path>, tags: &[String]) -> Result<()> {
    Ok(apeiron_services::set_file_tags(path.as_ref(), tags)?)
}
pub use apeiron_services::{MergeAction, MergeOutcome as MergeReport};
pub use conflicts::{conflicts, resolve_conflict};
pub use error::{Error, ErrorKind, Result};
pub use job::Job;
pub use plugins::{add_plugin, plugin_dir, plugins, remove_plugin};
pub use repo::Repository;
pub use report::{
    ComplianceReport, CopyKind, CopyState, Finding, FindingKind, Level, RefFacts, RemoteFacts,
    ReportRequest, RepositoryFacts,
};
pub use saved::{
    SavedSearch, forget_search, save_search, saved_search, saved_searches, searches_file,
};
pub use types::{
    AddedKey, CloneReport, CloneRequest, ClonedRef, DeleteRefRequest, DiffRequest, ExportRequest,
    GitRequest, ListRequest, LogPage, LogRequest, MirrorReport, MirroredRef, RefInfo,
    RemoteRequest, RepoInfo, RestoreRequest, RestoreTestRequest, SnapshotInfo, SyncRequest,
    TransferRequest,
};
pub use types::{
    CreateRefRequest, DuplicatesRequest, MergeRequest, RemoteGcRequest, SearchRequest,
    SwitchRequest, TextDiff,
};
