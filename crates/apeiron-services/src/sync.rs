//! Sync of a folder between devices through a remote (SYNC.md).

use std::fs::{self, DirEntry, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apeiron_core::{
    EntryKind, ObjectId, Repository, Snapshot, Store, Time, TransferReport, Tree, TreeEntry,
};
use serde::{Deserialize, Serialize};

use crate::backup;
use crate::control::{Control, Phase, Warning, WarningKind};
use crate::diff::{Change, ChangeKind};
use crate::error::{Error, IoContext, Result};
use crate::merge::{
    Conflict, ConflictKind, ContentMerger, Merge, PluginMerge, by_name, describe, free_name,
    is_dir, is_file, same, same_content, tree_of, write_items,
};
use crate::name::Name;
use crate::os::{self, Attrs};

/// Temporary files of a sync start with this; backups made by sync leave
/// them out (SYNC.md §5).
pub(crate) const TMP_PREFIX: &str = ".apeiron-sync-";
/// A temporary file older than this was left behind by an interrupted sync.
const TMP_MAX_AGE: Duration = Duration::from_secs(3600);
/// How often a round starts again after losing a push (SYNC.md §3).
const MAX_ATTEMPTS: u32 = 3;

/// Which way a sync goes (SYNC.md §3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncMode {
    #[default]
    TwoWay,
    /// Back up and push; the folder is never changed.
    Send,
    /// Pull and apply; nothing is pushed.
    Receive,
}

/// What to sync, and how.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOptions {
    pub folder: PathBuf,
    /// The ref on this device and on the remote.
    #[serde(rename = "ref")]
    pub ref_name: String,
    #[serde(default)]
    pub mode: SyncMode,
    /// Changes neither the folder, nor a ref, nor the remote; new folder
    /// content is still stored, for garbage collection to remove.
    #[serde(default)]
    pub dry_run: bool,
    /// This device's name, recorded in its snapshots and shown in the names
    /// of conflict copies made on other devices.
    #[serde(default)]
    pub host: Option<String>,
}

/// What a round did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncAction {
    /// The folder, the local ref and the remote were already equal.
    UpToDate,
    /// Only this device had changes; they were pushed.
    Sent,
    /// Only the remote had changes; they were applied to the folder.
    Received,
    /// Both had changes; they were merged, applied and pushed.
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOutcome {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub action: SyncAction,
    /// The snapshot the local ref points to now (would point to, in a dry
    /// run).
    pub snapshot: Option<ObjectId>,
    /// The snapshot made of the folder, if it had changed.
    pub backup: Option<ObjectId>,
    /// What came from the remote.
    pub fetched: Option<TransferReport>,
    pub pushed: Option<TransferReport>,
    /// Changes made to the folder (that would be made, in a dry run).
    pub changes: Vec<Change>,
    pub conflicts: Vec<Conflict>,
    /// Files plugins merged instead of making conflict copies.
    #[serde(default)]
    pub plugin_merges: Vec<PluginMerge>,
    /// Paths changed here while the round ran; left alone for the next
    /// round.
    pub skipped: Vec<Name>,
    pub warnings: Vec<Warning>,
    /// Rounds run; more than one when another device pushed meanwhile.
    pub attempts: u32,
}

/// Runs one sync round of `options.folder` with the ref `options.ref_name`
/// on `store`, starting again when another device pushed meanwhile. Files
/// changed on both devices go to the type drivers of `merger` first
/// (PLUGIN.md §4). The caller holds the repository lock shared.
pub fn sync(
    repo: &mut Repository,
    store: &dyn Store,
    options: &SyncOptions,
    merger: Option<&dyn ContentMerger>,
    ctl: &Control,
) -> Result<SyncOutcome> {
    apeiron_core::validate_ref_name(&folder_ref(&options.ref_name))?;
    let mut attempt = 1;
    loop {
        match round(repo, store, options, merger, ctl) {
            Ok(mut outcome) => {
                outcome.attempts = attempt;
                ctl.set_phase(Phase::Done);
                return Ok(outcome);
            }
            Err(Error::Core(
                apeiron_core::Error::RefConflict { .. } | apeiron_core::Error::RemoteAhead(_),
            )) if attempt < MAX_ATTEMPTS && options.mode != SyncMode::Send => attempt += 1,
            Err(e) => return Err(e),
        }
    }
}

enum Plan {
    UpToDate,
    Send,
    Receive(ObjectId),
    Merge {
        base: Option<ObjectId>,
        remote: ObjectId,
    },
}

fn round(
    repo: &mut Repository,
    store: &dyn Store,
    options: &SyncOptions,
    merger: Option<&dyn ContentMerger>,
    ctl: &Control,
) -> Result<SyncOutcome> {
    let name = options.ref_name.as_str();
    let dry_run = options.dry_run;
    let mut outcome = SyncOutcome {
        ref_name: name.to_owned(),
        action: SyncAction::UpToDate,
        snapshot: None,
        backup: None,
        fetched: None,
        pushed: None,
        changes: Vec::new(),
        conflicts: Vec::new(),
        plugin_merges: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
        attempts: 0,
    };

    // 1. Back up the folder as L.
    let state = folder_ref(name);
    let previous = repo.read_ref(&state)?;
    let previous_tree = previous
        .map(|id| repo.read_snapshot(&id).map(|s| s.tree))
        .transpose()?;
    let mut writer = repo.writer();
    let stored = backup::store_folder(&mut writer, &options.folder, true, ctl)?;
    outcome.warnings = stored.warnings;
    let local_tree = stored.tree;
    let local = if previous_tree != Some(local_tree) && !dry_run {
        let id = writer.put_snapshot(&snapshot(
            local_tree,
            previous.into_iter().collect(),
            options,
            "sync".to_owned(),
        ))?;
        writer.finish()?;
        repo.update_ref(&state, previous, Some(id))?;
        outcome.backup = Some(id);
        Some(id)
    } else {
        writer.finish()?;
        previous
    };
    outcome.snapshot = local;
    // Times alone change, for instance a folder's when a file in it is
    // replaced; such a snapshot is kept here but is not sent anywhere.
    let changed_here = match previous_tree {
        Some(p) => !same_content(repo, p, local_tree)?,
        None => true,
    };

    // 2. Copy what the remote ref reaches, moving no ref.
    ctl.set_phase(Phase::Copying);
    if store.read("config")?.is_some() {
        outcome.fetched = repo.fetch(store, name, &mut |r| copied(ctl, r))?;
    }
    let remote = outcome.fetched.as_ref().map(|r| r.snapshot);

    // 3. Compare by history; `previous` stands for L, whose only parent it is.
    let (remote_behind, local_behind) = match (remote, previous) {
        (Some(r), Some(p)) => (repo.is_ancestor(r, p)?, repo.is_ancestor(p, r)?),
        (Some(_), None) => (false, true),
        _ => (false, false),
    };
    let plan = match remote {
        None if local.is_none() && !changed_here => Plan::UpToDate,
        None => Plan::Send,
        Some(r) if Some(r) == previous => {
            if changed_here {
                Plan::Send
            } else {
                Plan::UpToDate
            }
        }
        Some(r) if remote_behind => {
            let remote_tree = repo.read_snapshot(&r)?.tree;
            if same_content(repo, remote_tree, local_tree)? {
                Plan::UpToDate
            } else {
                Plan::Send
            }
        }
        Some(r) if local_behind => {
            if changed_here {
                Plan::Merge {
                    base: previous,
                    remote: r,
                }
            } else {
                Plan::Receive(r)
            }
        }
        Some(r) => Plan::Merge {
            base: match previous {
                Some(p) => repo.merge_base(p, r)?,
                None => None,
            },
            remote: r,
        },
    };
    let plan = match (options.mode, plan) {
        (SyncMode::Send, Plan::Receive(_) | Plan::Merge { .. }) => {
            return Err(apeiron_core::Error::RemoteAhead(name.to_owned()).into());
        }
        (SyncMode::Receive, Plan::Send) => Plan::UpToDate,
        (_, plan) => plan,
    };

    let push = options.mode != SyncMode::Receive && !dry_run;
    match plan {
        Plan::UpToDate => {
            if !dry_run {
                set_shared(repo, name, local)?;
            }
        }
        Plan::Send => {
            outcome.action = SyncAction::Sent;
            if !dry_run {
                set_shared(repo, name, local)?;
            }
            if push {
                outcome.pushed = Some(repo.push(store, name, &mut |r| copied(ctl, r))?);
            }
        }
        Plan::Receive(r) => receive(repo, options, local_tree, local, r, &mut outcome, ctl)?,
        Plan::Merge { base, remote: r } => {
            let base_tree = base
                .map(|b| repo.read_snapshot(&b).map(|s| s.tree))
                .transpose()?;
            let remote_snapshot = repo.read_snapshot(&r)?;
            let suffix = conflict_suffix(&remote_snapshot);
            let result = Merge::new(repo, &suffix, merger).run(
                base_tree,
                local_tree,
                remote_snapshot.tree,
            )?;
            let conflicts = result.conflicts;
            outcome.plugin_merges = result.plugin_merges;
            outcome.warnings.extend(result.warnings);
            let mut writer = repo.writer();
            let merged = write_items(&mut writer, result.items)?;
            writer.finish()?;
            if same_content(repo, merged, remote_snapshot.tree)? {
                outcome.conflicts = conflicts;
                return receive(repo, options, local_tree, local, r, &mut outcome, ctl)
                    .map(|()| outcome);
            }
            let mut message = "sync: merge".to_owned();
            describe(&mut message, &conflicts, &outcome.plugin_merges);
            let parents = local.into_iter().chain([r]).collect();
            let mut writer = repo.writer();
            let m = writer.put_snapshot(&snapshot(merged, parents, options, message))?;
            writer.finish()?;
            outcome.action = SyncAction::Merged;
            outcome.conflicts = conflicts;
            apply_tree(
                repo,
                options,
                local_tree,
                merged,
                &suffix,
                &mut outcome,
                ctl,
            )?;
            if !dry_run {
                repo.update_ref(&state, local, Some(m))?;
                set_shared(repo, name, Some(m))?;
            }
            outcome.snapshot = Some(m);
            if push {
                outcome.pushed = Some(repo.push(store, name, &mut |r| copied(ctl, r))?);
            }
        }
    }
    Ok(outcome)
}

/// Applies the remote snapshot `remote` to the folder and moves the local
/// ref to it.
fn receive(
    repo: &mut Repository,
    options: &SyncOptions,
    local_tree: ObjectId,
    local: Option<ObjectId>,
    remote: ObjectId,
    outcome: &mut SyncOutcome,
    ctl: &Control,
) -> Result<()> {
    let snapshot = repo.read_snapshot(&remote)?;
    let suffix = conflict_suffix(&snapshot);
    outcome.action = SyncAction::Received;
    apply_tree(
        repo,
        options,
        local_tree,
        snapshot.tree,
        &suffix,
        outcome,
        ctl,
    )?;
    if !options.dry_run {
        repo.update_ref(&folder_ref(&options.ref_name), local, Some(remote))?;
        set_shared(repo, &options.ref_name, Some(remote))?;
    }
    outcome.snapshot = Some(remote);
    Ok(())
}

/// The local ref that points to the snapshot equal to the folder as of the
/// last finished round (SYNC.md §3). Only sync moves it; the shared ref
/// `name` may also come from `clone` or `pull` without the folder having
/// been changed.
pub fn folder_ref(name: &str) -> String {
    format!("folder/{name}")
}

/// Points the shared ref, the one pushed, to `id`.
fn set_shared(repo: &mut Repository, name: &str, id: Option<ObjectId>) -> Result<()> {
    let current = repo.read_ref(name)?;
    if id.is_some() && current != id {
        repo.update_ref(name, current, id)?;
    }
    Ok(())
}

fn apply_tree(
    repo: &Repository,
    options: &SyncOptions,
    from: ObjectId,
    to: ObjectId,
    suffix: &str,
    outcome: &mut SyncOutcome,
    ctl: &Control,
) -> Result<()> {
    ctl.set_phase(Phase::Restoring);
    let mut apply = Apply {
        repo,
        ctl,
        suffix,
        dry_run: options.dry_run,
        outcome,
    };
    let from = repo.read_tree(&from)?;
    let to = repo.read_tree(&to)?;
    apply.dir(&options.folder, &Name::default(), &from, &to)
}

fn snapshot(
    tree: ObjectId,
    parents: Vec<ObjectId>,
    options: &SyncOptions,
    message: String,
) -> Snapshot {
    Snapshot {
        tree,
        parents,
        time: os::to_time(SystemTime::now()),
        author: None,
        host: options.host.clone(),
        message: Some(message),
        meta: Vec::new(),
    }
}

fn copied(ctl: &Control, report: &TransferReport) -> apeiron_core::Result<()> {
    ctl.update(|p| {
        p.items_done = report.objects;
        p.bytes_done = report.bytes;
    });
    if ctl.is_cancelled() {
        return Err(apeiron_core::Error::Cancelled);
    }
    Ok(())
}

/// ` (conflict HOST YYYY-MM-DD HH-MM)` for the snapshot that brought the
/// other version; the time is UTC.
fn conflict_suffix(snapshot: &Snapshot) -> String {
    let host: String = snapshot
        .host
        .as_deref()
        .unwrap_or("unknown")
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let (y, mo, d, h, mi) = utc(snapshot.time);
    format!(" (conflict {host} {y:04}-{mo:02}-{d:02} {h:02}-{mi:02})")
}

/// Year, month, day, hour and minute of a time, in UTC.
fn utc(t: Time) -> (i64, u32, u32, u32, u32) {
    let days = t.seconds.div_euclid(86_400);
    let secs = t.seconds.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (secs / 3600) as u32, (secs % 3600 / 60) as u32)
}

/// Changes the folder from what the snapshot L recorded to a new tree,
/// touching only what still is as L recorded it (SYNC.md §5).
struct Apply<'a> {
    repo: &'a Repository,
    ctl: &'a Control,
    suffix: &'a str,
    dry_run: bool,
    outcome: &'a mut SyncOutcome,
}

impl Apply<'_> {
    fn change(&mut self, path: Name, kind: ChangeKind) {
        self.outcome.changes.push(Change { path, kind });
    }

    fn warn(&mut self, path: &Name, message: impl ToString) {
        self.outcome.warnings.push(Warning {
            path: path.clone(),
            kind: WarningKind::NotRestored,
            message: message.to_string(),
        });
    }

    fn dir(&mut self, dir: &Path, rel: &Name, from: &Tree, to: &Tree) -> Result<()> {
        for [f, t] in by_name([from, to]) {
            self.ctl.check()?;
            let Some(any) = t.or(f) else { continue };
            let child = rel.join(&any.name);
            let Some(os_name) = os::os_name(&any.name) else {
                self.warn(&child, "the name cannot be used on this system");
                continue;
            };
            let path = dir.join(os_name);
            match (f, t) {
                (Some(f), Some(t)) if same(Some(f), Some(t)) => {}
                (Some(f), Some(t)) if is_dir(Some(f)) && is_dir(Some(t)) => {
                    if !self.dry_run && !path.is_dir() {
                        self.outcome.skipped.push(child);
                        continue;
                    }
                    let (from, to) = (tree_of(self.repo, Some(f))?, tree_of(self.repo, Some(t))?);
                    self.dir(&path, &child, &from, &to)?;
                }
                (Some(f), Some(t)) if is_file(f) && is_file(t) => {
                    // Replaced in one rename, so the file is never missing.
                    if !unchanged(&path, f) {
                        self.outcome.skipped.push(child.clone());
                        self.copy_aside(dir, rel, t, &child)?;
                        continue;
                    }
                    self.create(dir, &path, &child, t)?;
                    self.change(child, ChangeKind::Modified);
                }
                (Some(f), t) => {
                    if !self.remove(&path, &child, f)? {
                        if let Some(t) = t {
                            self.copy_aside(dir, rel, t, &child)?;
                        }
                        continue;
                    }
                    match t {
                        Some(t) => {
                            self.create(dir, &path, &child, t)?;
                            self.change(child, ChangeKind::Modified);
                        }
                        None => self.change(child, ChangeKind::Removed),
                    }
                }
                (None, Some(t)) => {
                    if fs::symlink_metadata(&path).is_ok() {
                        if is_dir(Some(t)) && path.is_dir() {
                            let to = tree_of(self.repo, Some(t))?;
                            let empty = Tree {
                                entries: Vec::new(),
                            };
                            self.dir(&path, &child, &empty, &to)?;
                        } else {
                            self.copy_aside(dir, rel, t, &child)?;
                        }
                        continue;
                    }
                    self.create(dir, &path, &child, t)?;
                    self.change(child, ChangeKind::Added);
                }
                (None, None) => {}
            }
        }
        Ok(())
    }

    /// Removes what L recorded at `path` if it is still there unchanged;
    /// `false` if something else is there now. A folder is removed only
    /// once it is empty.
    fn remove(&mut self, path: &Path, rel: &Name, entry: &TreeEntry) -> Result<bool> {
        if let EntryKind::Directory { .. } = entry.kind {
            if !path.is_dir() {
                self.outcome.skipped.push(rel.clone());
                return Ok(false);
            }
            let from = tree_of(self.repo, Some(entry))?;
            let empty = Tree {
                entries: Vec::new(),
            };
            let skipped = self.outcome.skipped.len();
            self.dir(path, rel, &from, &empty)?;
            if self.dry_run {
                return Ok(self.outcome.skipped.len() == skipped);
            }
            return match fs::remove_dir(path) {
                Ok(()) => Ok(true),
                // Something appeared in it meanwhile.
                Err(_) if path.is_dir() => {
                    if self.outcome.skipped.len() == skipped {
                        self.outcome.skipped.push(rel.clone());
                    }
                    Ok(false)
                }
                Err(e) => Err(e).at(path),
            };
        }
        if !unchanged(path, entry) {
            self.outcome.skipped.push(rel.clone());
            return Ok(false);
        }
        if !self.dry_run {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).at(path),
            }
        }
        Ok(true)
    }

    /// Writes `entry` at `path`, where nothing is.
    fn create(&mut self, dir: &Path, path: &Path, rel: &Name, entry: &TreeEntry) -> Result<()> {
        if self.dry_run {
            return Ok(());
        }
        match &entry.kind {
            EntryKind::File { blob, .. } => {
                self.ctl.update(|p| p.current = Some(rel.clone()));
                let tmp = dir.join(tmp_name());
                let written = self.write_file(&tmp, blob.as_ref(), entry);
                let moved = written.and_then(|()| fs::rename(&tmp, path).at(path));
                if moved.is_err() {
                    let _ = fs::remove_file(&tmp);
                }
                moved?;
                self.ctl.update(|p| p.items_done += 1);
            }
            EntryKind::Directory { .. } => {
                fs::create_dir(path).at(path)?;
                let to = tree_of(self.repo, Some(entry))?;
                let empty = Tree {
                    entries: Vec::new(),
                };
                self.dir(path, rel, &empty, &to)?;
                set_attrs(path, entry, true);
            }
            EntryKind::Symlink { target } => {
                if let Err(e) = os::make_symlink(target, path) {
                    self.warn(rel, format!("the symbolic link could not be created: {e}"));
                }
            }
        }
        Ok(())
    }

    /// Writes the incoming version beside a file that changed while the
    /// round ran, so that neither is lost.
    fn copy_aside(&mut self, dir: &Path, rel: &Name, entry: &TreeEntry, path: &Name) -> Result<()> {
        if let EntryKind::File { .. } = entry.kind {
            let name = free_name(&entry.name, self.suffix, |n| {
                os::os_name(n).is_none_or(|n| fs::symlink_metadata(dir.join(n)).is_ok())
            });
            if let Some(os_name) = os::os_name(&name) {
                let copy = rel.join(&name);
                let mut aside = entry.clone();
                aside.name = name;
                self.create(dir, &dir.join(os_name), &copy, &aside)?;
                self.outcome.conflicts.push(Conflict {
                    path: path.clone(),
                    kind: ConflictKind::ChangedDuringSync,
                    copy: Some(copy),
                });
            }
        }
        Ok(())
    }

    fn write_file(&self, tmp: &Path, blob: Option<&ObjectId>, entry: &TreeEntry) -> Result<()> {
        let mut file = File::create_new(tmp).at(tmp)?;
        if let Some(blob) = blob {
            for (chunk, len) in self.repo.read_blob(blob)?.chunks {
                self.ctl.check()?;
                file.write_all(&self.repo.read_chunk(&chunk, len)?)
                    .at(tmp)?;
                self.ctl.update(|p| p.bytes_done += u64::from(len));
            }
        }
        if let Some(time) = entry.mtime.and_then(os::from_time) {
            file.set_modified(time).at(tmp)?;
        }
        file.sync_all().at(tmp)?;
        drop(file);
        set_attrs(tmp, entry, false);
        Ok(())
    }
}

/// Whether the file or link at `path` is still what `entry` recorded.
fn unchanged(path: &Path, entry: &TreeEntry) -> bool {
    let Ok(md) = fs::symlink_metadata(path) else {
        return false;
    };
    match &entry.kind {
        EntryKind::File { size, .. } => {
            md.is_file() && md.len() == *size && md.modified().ok().map(os::to_time) == entry.mtime
        }
        EntryKind::Symlink { target } => {
            md.file_type().is_symlink()
                && os::link_target(path).ok().flatten().as_ref() == Some(target)
        }
        EntryKind::Directory { .. } => md.is_dir(),
    }
}

/// Sets permissions and, for a folder, its time; failures are left alone,
/// as the content is what matters.
fn set_attrs(path: &Path, entry: &TreeEntry, is_dir: bool) {
    let attrs = Attrs {
        mode: entry.mode,
        mtime: entry.mtime,
        uid: None,
        gid: None,
    };
    if is_dir && let Some(time) = attrs.mtime.and_then(os::from_time) {
        let _ = os::set_dir_mtime(path, time);
    }
    let _ = os::apply_attrs(path, &attrs);
}

fn tmp_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{TMP_PREFIX}{nanos:x}-{:x}-{n}.tmp", std::process::id())
}

/// Whether `entry` is a temporary file of a sync. One older than an hour
/// was left by an interrupted sync and is removed.
pub(crate) fn is_tmp(entry: &DirEntry) -> bool {
    if !entry
        .file_name()
        .to_str()
        .is_some_and(|n| n.starts_with(TMP_PREFIX))
    {
        return false;
    }
    let old = entry
        .metadata()
        .and_then(|md| md.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > TMP_MAX_AGE);
    if old {
        let _ = fs::remove_file(entry.path());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(repo: &mut Repository, folder: &Path) -> ObjectId {
        let mut writer = repo.writer();
        let tree = backup::store_folder(&mut writer, folder, true, &Control::new())
            .unwrap()
            .tree;
        writer.finish().unwrap();
        tree
    }

    #[test]
    fn files_changed_during_the_round_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::init(&dir.path().join("repo")).unwrap();
        let (folder, next) = (dir.path().join("folder"), dir.path().join("next"));
        for (base, x, z) in [(&folder, "old", true), (&next, "new", false)] {
            fs::create_dir_all(base).unwrap();
            fs::write(base.join("x.txt"), x).unwrap();
            if z {
                fs::write(base.join("z.txt"), "z").unwrap();
            }
        }
        fs::write(next.join("y.txt"), "added").unwrap();
        let from = store(&mut repo, &folder);
        let to = store(&mut repo, &next);
        // A stale temporary file of an interrupted sync is not stored.
        fs::write(folder.join(tmp_name()), "half").unwrap();
        assert_eq!(store(&mut repo, &folder), from);

        // The user edits x and z after the backup.
        fs::write(folder.join("x.txt"), "edited now").unwrap();
        fs::write(folder.join("z.txt"), "edited too").unwrap();
        let options = SyncOptions {
            folder: folder.clone(),
            ref_name: "sync/test".to_owned(),
            mode: SyncMode::TwoWay,
            dry_run: false,
            host: None,
        };
        let mut outcome = SyncOutcome {
            ref_name: "sync/test".to_owned(),
            action: SyncAction::Received,
            snapshot: None,
            backup: None,
            fetched: None,
            pushed: None,
            changes: Vec::new(),
            conflicts: Vec::new(),
            plugin_merges: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            attempts: 1,
        };
        let suffix = " (conflict b 2026-10-29 10-00)";
        apply_tree(
            &repo,
            &options,
            from,
            to,
            suffix,
            &mut outcome,
            &Control::new(),
        )
        .unwrap();
        assert_eq!(outcome.skipped, [Name::from("x.txt"), Name::from("z.txt")]);
        assert_eq!(outcome.conflicts.len(), 1);
        assert_eq!(outcome.conflicts[0].kind, ConflictKind::ChangedDuringSync);
        let read = |n: &str| fs::read_to_string(folder.join(n)).unwrap();
        assert_eq!(read("x.txt"), "edited now");
        assert_eq!(read("x (conflict b 2026-10-29 10-00).txt"), "new");
        assert_eq!(read("z.txt"), "edited too");
        assert_eq!(read("y.txt"), "added");
        assert!(
            outcome
                .changes
                .iter()
                .any(|c| c.path == Name::from("y.txt") && c.kind == ChangeKind::Added)
        );
    }

    #[test]
    fn conflict_names_keep_the_extension_and_stay_free() {
        let suffix = " (conflict laptop 2026-10-29 14-05)";
        let none = |_: &[u8]| false;
        assert_eq!(
            free_name(b"report.pdf", suffix, none),
            b"report (conflict laptop 2026-10-29 14-05).pdf"
        );
        assert_eq!(
            free_name(b".bashrc", suffix, none),
            b".bashrc (conflict laptop 2026-10-29 14-05)"
        );
        let first = free_name(b"a.txt", suffix, none);
        assert_eq!(
            free_name(b"a.txt", suffix, |n| n == first.as_slice()),
            b"a (conflict laptop 2026-10-29 14-05 2).txt"
        );
        let time = Time {
            seconds: 1_761_746_700,
            nanos: 0,
        };
        assert_eq!(utc(time), (2025, 10, 29, 14, 5));
        assert_eq!(
            utc(Time {
                seconds: -1,
                nanos: 0
            }),
            (1969, 12, 31, 23, 59)
        );
    }
}
