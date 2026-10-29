//! Services of Apeiron: turning folders into snapshots and back, listing,
//! diff, search and export. The public `apeiron` crate exposes them.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod backup;
mod control;
mod diff;
mod error;
mod export;
mod list;
mod merge;
mod name;
mod os;
mod restore;
mod restore_test;
mod search;
mod sync;

pub use backup::{BackupOptions, BackupOutcome, backup};
pub use control::{Control, Phase, Progress, Warning, WarningKind};
pub use diff::{Change, ChangeKind, diff};
pub use error::{Error, Result};
pub use export::{ArchiveFormat, ExportOutcome, export};
pub use list::{EntryInfo, Kind, list, read_path};
pub use merge::{
    Conflict, ConflictKind, ContentMerger, MergeAction, MergeOptions, MergeOutcome, Merged,
    PLUGIN_SIZE_LIMIT, PluginMerge, merge,
};
pub use name::Name;
pub use restore::{RestoreOutcome, restore};
pub use restore_test::{RestoreProblem, RestoreTestOutcome, restore_test};
pub use search::{Query, SearchHit, SearchOptions, SearchOutcome, TextLine, name_matches, search};
pub use sync::{SyncAction, SyncMode, SyncOptions, SyncOutcome, folder_ref, sync};
