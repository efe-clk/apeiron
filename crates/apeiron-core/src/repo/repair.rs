//! Repairing damaged packs from another copy (FORMAT.md §8.3).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::gc::remove;
use super::transfer::{pack_file, read_remote};
use super::{INDEX, PACKS, Repository, TMP, list_dirs, list_files};
use crate::error::{Error, Result};
use crate::fsutil;
use crate::id::{ObjectId, ObjectType};
use crate::index::{self, Location};
use crate::lock::RepoLock;
use crate::object::{Blob, EntryKind, Snapshot, Tree};
use crate::pack::{self, FileHash, PackEntry, PackWriter};
use crate::refs::RepairReport;
use crate::remote::Store;

/// What [`Repository::repair_from`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyRepairReport {
    /// What the repair without a copy did first.
    pub repair: RepairReport,
    /// Pack files found damaged or missing.
    pub damaged_packs: u64,
    /// Objects still whole in a damaged pack, moved to a new one.
    pub salvaged_objects: u64,
    /// Objects taken from the copy.
    pub copied_objects: u64,
    /// Damaged packs deleted because everything in them has a whole copy.
    pub removed_packs: Vec<PathBuf>,
    /// Objects neither this repository nor the copy holds whole. Their packs
    /// are kept.
    pub unrecoverable: Vec<ObjectId>,
}

impl CopyRepairReport {
    /// Whether everything damaged has been replaced.
    pub fn is_ok(&self) -> bool {
        self.unrecoverable.is_empty()
    }
}

/// Whether an error says that this source does not hold the object whole, so
/// that the next source can be tried. Anything else, such as a network
/// failure, stops the repair before it deletes anything.
fn not_held<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Error::Corrupt { .. } | Error::ObjectNotFound(_) | Error::InvalidObject { .. }) => {
            Ok(None)
        }
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The objects the repair works on.
struct Sources<'a> {
    repo: &'a Repository,
    store: &'a dyn Store,
    have: &'a HashMap<ObjectId, Location>,
    /// Objects in packs whose whole file checked out.
    healthy: HashMap<ObjectId, Location>,
}

impl Sources<'_> {
    /// The content of an object from wherever it is whole.
    fn read(&self, id: &ObjectId, ty: ObjectType) -> Result<Option<Vec<u8>>> {
        let packs = self.repo.dir(PACKS);
        for location in [self.healthy.get(id), self.repo.index.get(id)]
            .into_iter()
            .flatten()
        {
            let path = pack::pack_path(&packs, &location.pack);
            if location.entry.ty == ty
                && let Some(data) =
                    not_held(pack::read_object(&path, &location.entry, &self.repo.keys))?
            {
                return Ok(Some(data));
            }
        }
        not_held(read_remote(self.store, self.have, &self.repo.keys, id, ty))
    }

    /// The stored bytes of an object that is not in a healthy pack, and
    /// whether they came from the copy.
    fn stored(&self, id: &ObjectId) -> Result<Option<(PackEntry, Vec<u8>, bool)>> {
        if let Some(location) = self.repo.index.get(id) {
            let path = pack::pack_path(&self.repo.dir(PACKS), &location.pack);
            if let Some(stored) =
                not_held(pack::read_stored(&path, &location.entry, &self.repo.keys))?
            {
                return Ok(Some((location.entry, stored, false)));
            }
        }
        let Some(location) = self.have.get(id) else {
            return Ok(None);
        };
        let path = pack_file(&location.pack);
        let read = self
            .store
            .read_range(
                &path,
                location.entry.offset,
                u64::from(location.entry.stored_len),
            )
            .and_then(|stored| {
                let from = self.store.location(&path);
                pack::decode_stored(&from, &location.entry, &self.repo.keys, &stored)?;
                Ok(stored)
            });
        Ok(not_held(read)?.map(|stored| (location.entry, stored, true)))
    }
}

impl Repository {
    /// Replaces what is damaged or missing in the packs of this repository
    /// with whole copies, from the damaged pack itself or from the copy in
    /// `store`, then deletes the packs that are no longer needed (FORMAT.md
    /// §8.3). `on_object` gets the objects replaced so far; an error, such
    /// as [`Error::Cancelled`], stops the repair before it deletes anything.
    /// Takes the repository lock exclusive.
    pub fn repair_from(
        &mut self,
        store: &dyn Store,
        on_object: &mut dyn FnMut(u64) -> Result<()>,
    ) -> Result<CopyRepairReport> {
        let _lock = RepoLock::exclusive(&self.root)?;
        self.reload_index()?;
        let mut report = CopyRepairReport {
            repair: self.repair()?,
            ..CopyRepairReport::default()
        };
        let have = self.open_remote(store)?;

        // Step 1: sort the pack files by their whole-file hash.
        let mut healthy_packs: Vec<(FileHash, Vec<PackEntry>)> = Vec::new();
        let mut damaged: HashMap<FileHash, PathBuf> = HashMap::new();
        for shard in list_dirs(&self.dir(PACKS))? {
            for path in list_files(&shard)? {
                let Some(name) = index::hash_from_file_name(&path, ".pack") else {
                    continue;
                };
                let checked = pack::verify_file_hash(&path, &name)
                    .and_then(|()| pack::read_table(&path, &self.keys));
                match checked {
                    Ok(entries) => healthy_packs.push((name, entries)),
                    Err(Error::Corrupt { .. }) => {
                        damaged.insert(name, path);
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        let mut healthy: HashMap<ObjectId, Location> = HashMap::new();
        for (name, entries) in &healthy_packs {
            for entry in entries {
                healthy.entry(entry.id).or_insert(Location {
                    pack: *name,
                    entry: *entry,
                });
            }
        }

        // Step 2: what to replace, and whether its loss must stop a pack from
        // being deleted. An object an index places outside the healthy packs
        // is replaced if it can be; it is required when a ref or a reflog
        // line reaches it, as nothing else can ever use it. An object only a
        // reflog line reaches and no index lists was removed on purpose by
        // garbage collection, so it is not required.
        let mut needed: HashMap<ObjectId, bool> = self
            .index
            .keys()
            .filter(|id| !healthy.contains_key(id))
            .map(|id| (*id, false))
            .collect();
        let lost = self
            .index
            .values()
            .filter(|l| !healthy_packs.iter().any(|(name, _)| *name == l.pack))
            .map(|l| l.pack)
            .filter(|name| !damaged.contains_key(name))
            .collect::<HashSet<_>>();
        report.damaged_packs = (damaged.len() + lost.len()) as u64;

        let sources = Sources {
            repo: self,
            store,
            have: &have,
            healthy,
        };
        let mut seen = HashSet::new();
        let mut roots: Vec<(ObjectId, bool)> = Vec::new();
        for (name, target) in self.list_refs()? {
            roots.push((target, true));
            for line in self.reflog(&name)? {
                roots.extend(line.old.into_iter().chain(line.new).map(|id| (id, false)));
            }
        }
        // Required roots first, so that an object both kinds reach counts as
        // required.
        roots.sort_by_key(|(_, required)| !*required);
        for (root, required) in roots {
            let mut stack = vec![(root, ObjectType::Snapshot)];
            while let Some((id, ty)) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                if !sources.healthy.contains_key(&id) {
                    let required = required || self.contains(&id);
                    needed
                        .entry(id)
                        .and_modify(|r| *r |= required)
                        .or_insert(required);
                }
                if ty == ObjectType::Chunk {
                    continue;
                }
                let Some(data) = sources.read(&id, ty)? else {
                    continue;
                };
                let children = not_held(children(ty, &data))?;
                stack.extend(children.into_iter().flatten());
            }
        }

        if needed.is_empty() && report.damaged_packs == 0 {
            return Ok(report);
        }

        // Step 3: write whole copies into new packs.
        let mut ids: Vec<(ObjectId, bool)> = needed.into_iter().collect();
        ids.sort();
        let mut written: Vec<(FileHash, Vec<PackEntry>)> = Vec::new();
        let mut writer: Option<PackWriter> = None;
        let mut unrecoverable = Vec::new();
        let (mut salvaged, mut copied) = (0, 0);
        for (id, required) in ids {
            let Some((entry, stored, from_copy)) = sources.stored(&id)? else {
                if required {
                    unrecoverable.push(id);
                }
                continue;
            };
            let w = match &mut writer {
                Some(w) => w,
                None => writer.insert(PackWriter::new(&self.dir(TMP), &self.keys)?),
            };
            w.add_stored(&entry, &stored)?;
            if from_copy {
                copied += 1;
            } else {
                salvaged += 1;
            }
            on_object(salvaged + copied)?;
            if w.len() >= pack::TARGET_SIZE
                && let Some(w) = writer.take()
            {
                written.push(w.finish_replacing(&self.dir(PACKS))?);
            }
        }
        if let Some(w) = writer.take() {
            written.push(w.finish_replacing(&self.dir(PACKS))?);
        }
        report.salvaged_objects = salvaged;
        report.copied_objects = copied;

        // Step 4: a damaged pack stays listed only for the objects nothing
        // else holds. One whose content no index knows may hold such an
        // object too, so it stays whenever any object is unrecoverable.
        let mut kept: HashMap<FileHash, Vec<PackEntry>> = HashMap::new();
        for id in &unrecoverable {
            if let Some(location) = self.index.get(id) {
                kept.entry(location.pack).or_default().push(location.entry);
            }
        }
        // A new pack can have the very bytes the damaged one had before it
        // was damaged, and so its name: then it has just replaced that file,
        // which must not be deleted now.
        let rewritten: HashSet<FileHash> = written.iter().map(|(name, _)| *name).collect();
        kept.retain(|name, _| !rewritten.contains(name));
        let known: HashSet<FileHash> = self.index.values().map(|l| l.pack).collect();
        let delete: Vec<PathBuf> = damaged
            .iter()
            .filter(|(name, _)| {
                !rewritten.contains(*name)
                    && !kept.contains_key(*name)
                    && (unrecoverable.is_empty() || known.contains(*name))
            })
            .map(|(_, path)| path.clone())
            .collect();

        // Step 5: one index for every healthy and new pack, then the rest
        // goes, in the order garbage collection uses (FORMAT.md §13.2).
        let mut listed = healthy_packs;
        listed.extend(written);
        listed.extend(kept);
        let index_files = list_files(&self.dir(INDEX))?;
        let new_index = index::write_index(&self.dir(TMP), &self.dir(INDEX), &listed, &self.keys)?;
        for path in index_files {
            if path != new_index {
                remove(&path)?;
            }
        }
        fsutil::sync_dir(&self.dir(INDEX))?;
        for path in &delete {
            remove(path)?;
            fsutil::sync_dir(fsutil::parent_of(path))?;
        }
        report.removed_packs = delete;
        report.removed_packs.sort();
        unrecoverable.sort();
        report.unrecoverable = unrecoverable;
        self.reload_index()?;
        Ok(report)
    }
}

/// The objects a snapshot, tree or blob refers to.
fn children(ty: ObjectType, data: &[u8]) -> Result<Vec<(ObjectId, ObjectType)>> {
    let mut out = Vec::new();
    match ty {
        ObjectType::Snapshot => {
            let snapshot = Snapshot::decode(data)?;
            out.push((snapshot.tree, ObjectType::Tree));
            out.extend(snapshot.parents.iter().map(|p| (*p, ObjectType::Snapshot)));
        }
        ObjectType::Tree => {
            for entry in Tree::decode(data)?.entries {
                match entry.kind {
                    EntryKind::File { blob: Some(b), .. } => out.push((b, ObjectType::Blob)),
                    EntryKind::Directory { tree } => out.push((tree, ObjectType::Tree)),
                    EntryKind::File { blob: None, .. } | EntryKind::Symlink { .. } => {}
                }
            }
        }
        ObjectType::Blob => {
            let blob = Blob::decode(data)?;
            out.extend(blob.chunks.iter().map(|(id, _)| (*id, ObjectType::Chunk)));
        }
        ObjectType::Chunk => {}
    }
    Ok(out)
}
