//! Plugins that look at a backup before its ref moves (PLUGIN.md §2.2), and
//! plugins that read archive formats for import (§2.1).

use apeiron_core::{ObjectId, Repository};

use crate::control::{Warning, WarningKind};
use crate::diff::{ChangeKind, diff_from};
use crate::error::{Error, Result};
use crate::list::read_path;
use crate::name::Name;

/// The largest file whose content a hook is shown.
pub const HOOK_CONTENT_LIMIT: u64 = 1 << 20;
/// The most file content a hook is shown in all.
pub const HOOK_TOTAL_LIMIT: u64 = 64 << 20;

/// A path a backup changes, as a hook sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookChange {
    pub path: Name,
    pub kind: ChangeKind,
    /// The new content of an added or changed file of at most
    /// [`HOOK_CONTENT_LIMIT`] bytes, while the total stays within
    /// [`HOOK_TOTAL_LIMIT`].
    pub content: Option<Vec<u8>>,
}

/// What a hook plugin says about a backup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Go on, with warnings to show: the plugin and its message.
    Allow(Vec<(String, String)>),
    /// Stop: the plugin and why.
    Refuse(String, String),
}

/// Runs the hooks of an event, such as `backup` (PLUGIN.md §2.2).
pub trait Hooks {
    fn has_hooks(&self, event: &str) -> bool;
    fn check(&self, event: &str, ref_name: &str, changes: &[HookChange]) -> Verdict;
}

/// The files an importer read: path and content.
pub type ReadFiles = Vec<(Vec<u8>, Vec<u8>)>;

/// Reads archive formats Apeiron does not know (PLUGIN.md §2.1).
pub trait ArchiveReader {
    fn reads(&self, name: &[u8]) -> bool;
    /// The plugin that read it and the files inside: path and content.
    fn read(&self, name: &[u8], content: &[u8])
    -> std::result::Result<(String, ReadFiles), String>;
}

/// Shows the changes from `parent` to `snapshot` to the hooks of `event`;
/// their warnings, or [`Error::Refused`] when one says no. The snapshot is
/// stored, but its ref has not moved yet.
pub(crate) fn run_hooks(
    repo: &Repository,
    hooks: &dyn Hooks,
    event: &str,
    ref_name: &str,
    parent: Option<&ObjectId>,
    snapshot: &ObjectId,
) -> Result<Vec<Warning>> {
    if !hooks.has_hooks(event) {
        return Ok(Vec::new());
    }
    let mut budget = HOOK_TOTAL_LIMIT;
    let mut changes = Vec::new();
    for c in diff_from(repo, parent, snapshot)? {
        let content = match c.kind {
            ChangeKind::Removed => None,
            _ => match read_path(repo, snapshot, c.path.as_bytes(), HOOK_CONTENT_LIMIT) {
                Ok(Some(data)) if (data.len() as u64) <= budget => {
                    budget -= data.len() as u64;
                    Some(data)
                }
                // A folder, a link or a file too large has none to show.
                Ok(_) => None,
                Err(e) => return Err(e),
            },
        };
        changes.push(HookChange {
            path: c.path,
            kind: c.kind,
            content,
        });
    }
    match hooks.check(event, ref_name, &changes) {
        Verdict::Allow(warnings) => Ok(warnings
            .into_iter()
            .map(|(plugin, message)| Warning {
                path: Name::default(),
                kind: WarningKind::Plugin,
                message: format!("{plugin}: {message}"),
            })
            .collect()),
        Verdict::Refuse(plugin, message) => Err(Error::Refused(format!(
            "the plugin {plugin} stopped the backup: {message}"
        ))),
    }
}
