//! Services of Apeiron: turning folders into snapshots and back, listing,
//! diff and export. The public `apeiron` crate exposes them.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod backup;
mod control;
mod diff;
mod error;
mod list;
mod name;
mod os;
mod restore;

pub use backup::{BackupOptions, BackupOutcome, backup};
pub use control::{Control, Phase, Progress, Warning, WarningKind};
pub use diff::{Change, ChangeKind, diff};
pub use error::{Error, Result};
pub use list::{EntryInfo, Kind, list};
pub use name::Name;
pub use restore::{RestoreOutcome, restore};
