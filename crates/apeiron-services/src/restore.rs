//! Writing a snapshot back into a folder.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use apeiron_core::{EntryKind, ObjectId, Repository, Tree, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::control::{Control, Phase, Warning, WarningKind};
use crate::error::{Error, IoContext, Result};
use crate::list::{Node, lookup};
use crate::name::Name;
use crate::os::{self, Attrs};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreOutcome {
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    pub bytes: u64,
    pub warnings: Vec<Warning>,
}

/// Writes `path` of `snapshot` (everything for an empty path) into `target`,
/// which must not exist or be an empty folder. A file is written as
/// `target/<name>`. Nothing outside `target` is touched.
///
/// If the operation fails or is cancelled, the files written so far stay;
/// a partly written file is removed.
pub fn restore(
    repo: &Repository,
    snapshot: &ObjectId,
    path: &[u8],
    target: &Path,
    ctl: &Control,
) -> Result<RestoreOutcome> {
    let root = repo.read_snapshot(snapshot)?.tree;
    let node = lookup(repo, &root, path)?;
    prepare_target(target)?;

    ctl.set_phase(Phase::Scanning);
    let mut totals = (0, 0);
    match &node {
        Node::Tree(tree) => count_tree(repo, tree, ctl, &mut totals)?,
        Node::Entry(entry) => count_entry(repo, entry, ctl, &mut totals)?,
    }
    ctl.update(|p| {
        p.phase = Phase::Restoring;
        p.items_total = Some(totals.0);
        p.bytes_total = Some(totals.1);
    });

    let mut run = Run {
        repo,
        ctl,
        out: RestoreOutcome::default(),
    };
    match node {
        Node::Tree(tree) => run.write_tree(&tree, target, &Name::default())?,
        Node::Entry(entry) => run.write_entry(&entry, target, &Name::default())?,
    }
    ctl.set_phase(Phase::Done);
    Ok(run.out)
}

fn prepare_target(target: &Path) -> Result<()> {
    match fs::read_dir(target) {
        Ok(mut listing) => {
            if listing.next().is_some() {
                return Err(Error::AlreadyExists(target.to_path_buf()));
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir_all(target).at(target),
        Err(_) if target.exists() => Err(Error::AlreadyExists(target.to_path_buf())),
        Err(e) => Err(e).at(target),
    }
}

fn count_tree(
    repo: &Repository,
    tree: &Tree,
    ctl: &Control,
    totals: &mut (u64, u64),
) -> Result<()> {
    for entry in &tree.entries {
        count_entry(repo, entry, ctl, totals)?;
    }
    Ok(())
}

fn count_entry(
    repo: &Repository,
    entry: &TreeEntry,
    ctl: &Control,
    totals: &mut (u64, u64),
) -> Result<()> {
    ctl.check()?;
    match &entry.kind {
        EntryKind::File { size, .. } => {
            totals.0 += 1;
            totals.1 += size;
        }
        EntryKind::Directory { tree } => count_tree(repo, &repo.read_tree(tree)?, ctl, totals)?,
        EntryKind::Symlink { .. } => {}
    }
    Ok(())
}

fn attrs_of(entry: &TreeEntry) -> Attrs {
    Attrs {
        mode: entry.mode,
        mtime: entry.mtime,
        uid: entry.uid,
        gid: entry.gid,
    }
}

struct Run<'a> {
    repo: &'a Repository,
    ctl: &'a Control,
    out: RestoreOutcome,
}

impl Run<'_> {
    fn warn(&mut self, path: &Name, message: impl ToString) {
        self.out.warnings.push(Warning {
            path: path.clone(),
            kind: WarningKind::NotRestored,
            message: message.to_string(),
        });
    }

    fn write_tree(&mut self, tree: &Tree, dir: &Path, rel: &Name) -> Result<()> {
        for entry in &tree.entries {
            self.write_entry(entry, dir, rel)?;
        }
        Ok(())
    }

    fn write_entry(&mut self, entry: &TreeEntry, dir: &Path, rel: &Name) -> Result<()> {
        self.ctl.check()?;
        let child = rel.join(&entry.name);
        let Some(name) = os::os_name(&entry.name) else {
            self.warn(&child, "the name cannot be used on this system");
            return Ok(());
        };
        let path = dir.join(name);
        match &entry.kind {
            EntryKind::File { size, blob } => {
                self.write_file(&path, &child, blob.as_ref())?;
                self.out.files += 1;
                self.out.bytes += size;
                self.ctl.update(|p| p.items_done += 1);
                self.finish_attrs(&path, &child, entry, false);
            }
            EntryKind::Directory { tree } => {
                fs::create_dir(&path).at(&path)?;
                self.write_tree(&self.repo.read_tree(tree)?, &path, &child)?;
                self.out.dirs += 1;
                // Applied after the content, so a read-only folder can still be filled.
                self.finish_attrs(&path, &child, entry, true);
            }
            EntryKind::Symlink { target } => match os::make_symlink(target, &path) {
                Ok(()) => self.out.symlinks += 1,
                Err(e) => self.warn(
                    &child,
                    format!("the symbolic link could not be created: {e}"),
                ),
            },
        }
        Ok(())
    }

    fn write_file(&mut self, path: &Path, rel: &Name, blob: Option<&ObjectId>) -> Result<()> {
        self.ctl.update(|p| p.current = Some(rel.clone()));
        let mut file = File::create_new(path).at(path)?;
        let written = self.copy_blob(&mut file, path, blob);
        drop(file);
        if written.is_err() {
            let _ = fs::remove_file(path);
        }
        written
    }

    fn copy_blob(&self, file: &mut File, path: &Path, blob: Option<&ObjectId>) -> Result<()> {
        let Some(blob) = blob else {
            return Ok(());
        };
        for (chunk, len) in self.repo.read_blob(blob)?.chunks {
            self.ctl.check()?;
            let data = self.repo.read_chunk(&chunk, len)?;
            file.write_all(&data).at(path)?;
            self.ctl.update(|p| p.bytes_done += u64::from(len));
        }
        file.sync_all().at(path)
    }

    fn finish_attrs(&mut self, path: &Path, rel: &Name, entry: &TreeEntry, is_dir: bool) {
        let attrs = attrs_of(entry);
        if let Some(time) = attrs.mtime.and_then(os::from_time) {
            let set = if is_dir {
                os::set_dir_mtime(path, time)
            } else {
                File::options()
                    .write(true)
                    .open(path)
                    .and_then(|f| f.set_modified(time))
            };
            if let Err(e) = set {
                self.warn(rel, format!("the modification time could not be set: {e}"));
            }
        }
        if let Err(e) = os::apply_attrs(path, &attrs) {
            self.warn(rel, format!("the permissions could not be set: {e}"));
        }
    }
}
