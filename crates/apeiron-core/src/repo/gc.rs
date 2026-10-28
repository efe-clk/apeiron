//! Garbage collection (FORMAT.md §13).

use std::collections::HashSet;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::{INDEX, PACKS, Repository, TMP, list_dirs, list_files};
use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::{ObjectId, ObjectType};
use crate::index;
use crate::lock::RepoLock;
use crate::object::EntryKind;
use crate::pack::{self, FileHash, PackEntry, PackWriter};

const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcOptions {
    /// Reflog lines younger than this many days keep their snapshots alive;
    /// 0 keeps only what refs point to.
    pub keep_reflog_days: u32,
    /// Only report what would be removed.
    pub dry_run: bool,
}

impl Default for GcOptions {
    fn default() -> Self {
        GcOptions {
            keep_reflog_days: 90,
            dry_run: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub live_objects: u64,
    /// Dead objects and duplicate copies removed (or that would be removed).
    pub removed_objects: u64,
    /// Their stored (compressed) size.
    pub removed_bytes: u64,
    pub removed_packs: u64,
    pub written_packs: u64,
}

impl Repository {
    /// Removes objects that no ref and no recent reflog line can reach and
    /// rewrites partly dead packs. Takes the repository lock exclusive.
    pub fn gc(&mut self, options: &GcOptions) -> Result<GcReport> {
        let _lock = RepoLock::exclusive(&self.root)?;
        self.reload_index()?;
        let live = self.live_objects(options.keep_reflog_days)?;

        let mut packs = Vec::new();
        for shard in list_dirs(&self.dir(PACKS))? {
            for path in list_files(&shard)? {
                if let Some(name) = index::hash_from_file_name(&path, ".pack") {
                    let entries = pack::read_table(&path)?;
                    packs.push((name, path, entries));
                }
            }
        }
        packs.sort_by_key(|(name, ..)| *name);

        let mut report = GcReport {
            dry_run: options.dry_run,
            live_objects: live.len() as u64,
            ..GcReport::default()
        };
        let mut covered: HashSet<ObjectId> = HashSet::new();
        let mut kept: Vec<(FileHash, Vec<PackEntry>)> = Vec::new();
        let mut dead = Vec::new();
        for (name, path, entries) in packs {
            if !entries.is_empty() && entries.iter().all(|e| live.contains(&e.id)) {
                covered.extend(entries.iter().map(|e| e.id));
                kept.push((name, entries));
            } else {
                dead.push((path, entries));
            }
        }
        let mut copies = Vec::new();
        for (path, entries) in &dead {
            for entry in entries {
                if live.contains(&entry.id) && covered.insert(entry.id) {
                    copies.push((path.clone(), *entry));
                } else {
                    report.removed_objects += 1;
                    report.removed_bytes += u64::from(entry.stored_len);
                }
            }
        }
        if let Some(missing) = live.iter().find(|id| !covered.contains(id)) {
            return Err(Error::ObjectNotFound(*missing));
        }
        report.removed_packs = dead.len() as u64;
        if options.dry_run {
            return Ok(report);
        }

        let index_files = list_files(&self.dir(INDEX))?;
        if dead.is_empty() && (index_files.len() == 1 || kept.is_empty()) {
            self.clean_all_tmp()?;
            return Ok(report);
        }

        let mut written = Vec::new();
        let mut writer: Option<PackWriter> = None;
        for (path, entry) in &copies {
            let stored = pack::read_stored(path, entry, &self.config.id_key)?;
            let w = match &mut writer {
                Some(w) => w,
                None => writer.insert(PackWriter::new(&self.dir(TMP))?),
            };
            w.add_stored(entry, &stored)?;
            if w.len() >= pack::TARGET_SIZE {
                if let Some(w) = writer.take() {
                    written.push(w.finish(&self.dir(PACKS))?);
                }
            }
        }
        if let Some(w) = writer.take() {
            written.push(w.finish(&self.dir(PACKS))?);
        }
        report.written_packs = written.len() as u64;

        kept.extend(written);
        let new_index = index::write_index(&self.dir(TMP), &self.dir(INDEX), &kept)?;
        for path in index_files {
            if path != new_index {
                remove(&path)?;
            }
        }
        fsutil::sync_dir(&self.dir(INDEX))?;
        for (path, _) in &dead {
            remove(path)?;
            fsutil::sync_dir(fsutil::parent_of(path))?;
        }
        self.clean_all_tmp()?;
        self.reload_index()?;
        Ok(report)
    }

    /// Everything reachable from refs and from reflog lines younger than
    /// `keep_days`. A missing or damaged reachable object is an error.
    fn live_objects(&self, keep_days: u32) -> Result<HashSet<ObjectId>> {
        let mut stack: Vec<(ObjectId, ObjectType)> = Vec::new();
        for (_, target) in self.refs.list()? {
            stack.push((target, ObjectType::Snapshot));
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let cutoff = now.saturating_sub(u64::from(keep_days) * SECONDS_PER_DAY);
        if keep_days > 0 {
            for name in self.refs.log_names()? {
                for entry in self.refs.reflog(&name)? {
                    if entry.time < cutoff {
                        continue;
                    }
                    // Snapshots of old lines may already have been collected.
                    for id in [entry.old, entry.new].into_iter().flatten() {
                        if self.contains(&id) {
                            stack.push((id, ObjectType::Snapshot));
                        }
                    }
                }
            }
        }

        let mut live = HashSet::new();
        while let Some((id, ty)) = stack.pop() {
            if !live.insert(id) {
                continue;
            }
            match ty {
                ObjectType::Snapshot => {
                    let snapshot = self.read_snapshot(&id)?;
                    stack.push((snapshot.tree, ObjectType::Tree));
                    stack.extend(snapshot.parents.iter().map(|p| (*p, ObjectType::Snapshot)));
                }
                ObjectType::Tree => {
                    for entry in self.read_tree(&id)?.entries {
                        match entry.kind {
                            EntryKind::File { blob: Some(b), .. } => {
                                stack.push((b, ObjectType::Blob))
                            }
                            EntryKind::Directory { tree } => stack.push((tree, ObjectType::Tree)),
                            EntryKind::File { blob: None, .. } | EntryKind::Symlink { .. } => {}
                        }
                    }
                }
                ObjectType::Blob => {
                    for (chunk, _) in self.read_blob(&id)?.chunks {
                        stack.push((chunk, ObjectType::Chunk));
                    }
                }
                ObjectType::Chunk => match self.index.get(&id) {
                    Some(l) if l.entry.ty == ObjectType::Chunk => {}
                    Some(l) => {
                        return Err(Error::WrongObjectType {
                            id,
                            expected: ObjectType::Chunk,
                            found: l.entry.ty,
                        });
                    }
                    None => return Err(Error::ObjectNotFound(id)),
                },
            }
        }
        Ok(live)
    }

    /// With the exclusive lock held, nothing else is writing temporary files.
    fn clean_all_tmp(&self) -> Result<()> {
        for path in list_files(&self.dir(TMP))? {
            remove(&path)?;
        }
        Ok(())
    }
}

fn remove(path: &std::path::Path) -> Result<()> {
    fsutil::checkpoint()
        .and_then(|()| std::fs::remove_file(path))
        .at(path)
}
