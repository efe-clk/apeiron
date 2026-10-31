//! The conflict copies sync and merge leave in a folder, and keeping one
//! version of each (SYNC.md §4). They need no repository.

use std::path::Path;

use crate::error::Result;
use crate::{ConflictCopy, Keep};

/// The conflict copies in `folder` and the folders inside it, sorted by
/// path.
pub fn conflicts(folder: &Path) -> Result<Vec<ConflictCopy>> {
    Ok(apeiron_services::find_conflicts(folder)?)
}

/// Keeps one version of the conflict copy `copy`, a path relative to
/// `folder`: [`Keep::This`] deletes the copy, [`Keep::Copy`] puts it in
/// place of the file it stands beside.
pub fn resolve_conflict(folder: &Path, copy: &str, keep: Keep) -> Result<ConflictCopy> {
    Ok(apeiron_services::resolve_conflict(folder, copy, keep)?)
}
