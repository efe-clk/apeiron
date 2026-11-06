//! A working folder that follows a branch (BRANCH.md §6).

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use apeiron_core::{ObjectId, Repository, Snapshot, Tree};
use serde::{Deserialize, Serialize};

use crate::backup;
use crate::control::{Control, Phase, Warning};
use crate::diff::Change;
use crate::error::{Error, IoContext, Result};
use crate::merge::{Conflict, same_content};
use crate::name::Name;
use crate::os;
use crate::sync::{SyncAction, SyncMode, SyncOptions, SyncOutcome, apply_tree, folder_ref};

/// Which folder to turn into which snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchOptions {
    pub folder: PathBuf,
    /// The snapshot the folder is to hold.
    pub target: ObjectId,
    /// How the caller named it, for messages and conflict copies.
    pub target_name: String,
    /// The name under `folder/` of the ref that records the folder.
    pub name: String,
    /// Changes neither the folder nor a ref.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub host: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchOutcome {
    /// The ref that records the folder.
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// The snapshot the folder holds now (would hold, in a dry run).
    pub snapshot: ObjectId,
    /// The snapshot made of the folder before it was changed, when it held
    /// something not recorded yet.
    pub saved: Option<ObjectId>,
    /// Changes made to the folder (that would be made, in a dry run).
    pub changes: Vec<Change>,
    /// Files changed while the switch ran: left alone, with the version of
    /// the snapshot beside them.
    pub conflicts: Vec<Conflict>,
    pub skipped: Vec<Name>,
    pub warnings: Vec<Warning>,
}

/// Makes `options.folder` hold the snapshot `options.target` (BRANCH.md
/// §6): records the folder first, so nothing in it is lost, then changes
/// only what differs. A folder never switched before must be empty or hold
/// what a ref points to. The caller holds the repository lock shared.
pub fn switch(
    repo: &mut Repository,
    options: &SwitchOptions,
    ctl: &Control,
) -> Result<SwitchOutcome> {
    let state = folder_ref(&options.name);
    apeiron_core::validate_ref_name(&state)?;
    let target_tree = repo.read_snapshot(&options.target)?.tree;
    let previous = repo.read_ref(&state)?;

    let mut writer = repo.writer();
    let (folder_tree, warnings) = if options.folder.exists() {
        let stored = backup::store_folder(
            &mut writer,
            &options.folder,
            true,
            &[],
            backup::Previous::default(),
            ctl,
        )?;
        (stored.tree, stored.warnings)
    } else {
        (
            writer.put_tree(&Tree {
                entries: Vec::new(),
            })?,
            Vec::new(),
        )
    };
    writer.finish()?;
    let empty = repo.read_tree(&folder_tree)?.entries.is_empty();

    let recorded = match previous {
        Some(id) => Some(repo.read_snapshot(&id)?.tree),
        None if empty => None,
        None => known_tree(repo, folder_tree)?,
    };
    if previous.is_none() && !empty && recorded.is_none() {
        return Err(Error::InvalidInput(format!(
            "{} holds files that no ref points to; back it up first or switch an empty folder",
            options.folder.display()
        )));
    }
    // Times alone change, for instance a folder's when a file in it is
    // replaced; only other changes need recording, as in sync.
    let unsaved = !empty
        && match recorded {
            Some(tree) => !same_content(repo, tree, folder_tree)?,
            None => true,
        };
    let mut saved = None;
    if unsaved && !options.dry_run {
        let mut writer = repo.writer();
        let id = writer.put_snapshot(&Snapshot {
            tree: folder_tree,
            parents: previous.into_iter().collect(),
            time: os::to_time(SystemTime::now()),
            author: None,
            host: options.host.clone(),
            message: Some(format!(
                "switch: the folder before switching to {}",
                options.target_name
            )),
            meta: Vec::new(),
        })?;
        writer.finish()?;
        repo.update_ref(&state, previous, Some(id))?;
        saved = Some(id);
    }

    let mut outcome = SyncOutcome {
        ref_name: state.clone(),
        action: SyncAction::Received,
        snapshot: None,
        backup: None,
        fetched: None,
        pushed: None,
        changes: Vec::new(),
        conflicts: Vec::new(),
        plugin_merges: Vec::new(),
        skipped: Vec::new(),
        warnings,
        attempts: 1,
    };
    if !options.dry_run {
        fs::create_dir_all(&options.folder).at(&options.folder)?;
    }
    let sync_options = SyncOptions {
        folder: options.folder.clone(),
        ref_name: options.name.clone(),
        mode: SyncMode::Receive,
        dry_run: options.dry_run,
        host: options.host.clone(),
        exclude: Vec::new(),
    };
    let suffix = format!(" (conflict {})", options.target_name.replace('/', "-"));
    apply_tree(
        repo,
        &sync_options,
        folder_tree,
        target_tree,
        &suffix,
        &mut outcome,
        ctl,
    )?;
    if !options.dry_run {
        let current = saved.or(previous);
        if current != Some(options.target) {
            repo.update_ref(&state, current, Some(options.target))?;
        }
    }
    ctl.set_phase(Phase::Done);
    Ok(SwitchOutcome {
        ref_name: state,
        snapshot: options.target,
        saved,
        changes: outcome.changes,
        conflicts: outcome.conflicts,
        skipped: outcome.skipped,
        warnings: outcome.warnings,
    })
}

/// The tree of a ref's snapshot with the same content as `tree`, if any.
fn known_tree(repo: &Repository, tree: ObjectId) -> Result<Option<ObjectId>> {
    for (_, id) in repo.list_refs()? {
        let theirs = repo.read_snapshot(&id)?.tree;
        if same_content(repo, theirs, tree)? {
            return Ok(Some(tree));
        }
    }
    Ok(None)
}
