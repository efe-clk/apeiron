//! Services of Apeiron: turning folders into snapshots and back, listing,
//! diff, search and export. The public `apeiron` crate exposes them.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod backup;
mod conflicts;
mod control;
mod diff;
mod error;
mod exclude;
mod export;
mod import;
mod list;
mod merge;
mod name;
mod os;
mod prune;
mod restore;
mod restore_test;
mod search;
mod switch;
mod sync;
mod tags;
mod upload;

pub use backup::{BackupOptions, BackupOutcome, backup};
pub use conflicts::{ConflictCopy, Keep, find_conflicts, original_name, resolve_conflict};
pub use control::{Control, Phase, Progress, Warning, WarningKind};
pub use diff::{Change, ChangeKind, diff};
pub use error::{Error, Result};
pub use exclude::IGNORE_FILE;
pub use export::{ArchiveFormat, ExportOutcome, export, export_zip_to, split_archive};
pub use import::{ImportFormat, ImportOptions, ImportOutcome, import};
pub use list::{EntryInfo, Kind, file_chunks, list, read_path, read_to};
pub use merge::{
    Conflict, ConflictKind, ContentMerger, MergeAction, MergeOptions, MergeOutcome, Merged,
    PLUGIN_SIZE_LIMIT, PluginMerge, cherry_pick, merge, rebase,
};
pub use name::Name;
pub use prune::{PruneOptions, PruneOutcome, PrunedKept, PrunedRemoved, prune};
pub use restore::{RestoreOutcome, restore};
pub use restore_test::{RestoreProblem, RestoreTestOutcome, restore_test};
pub use search::{
    Duplicate, Query, SearchHit, SearchOptions, SearchOutcome, TextExtractor, TextLine, duplicates,
    name_matches, search, search_with,
};
pub use switch::{SwitchOptions, SwitchOutcome, switch};
pub use sync::{SyncAction, SyncMode, SyncOptions, SyncOutcome, folder_ref, sync};
pub use tags::{entry_tags, file_tags, set_file_tags};
pub use upload::{AddFileOptions, AddFileOutcome, add_file, put_files};
