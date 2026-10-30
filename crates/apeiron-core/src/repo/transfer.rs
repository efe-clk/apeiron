//! Remotes, push and pull (FORMAT.md §15).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{
    CONFIG, INDEX, KEYS, LOCKS, LOGS, PACKS, REFS, REMOTES, RETENTION, Repository, TMP, Unlock,
    create_layout,
};
use crate::config::Config;
use crate::crypto::Keys;
use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::{ObjectId, ObjectType, to_hex};
use crate::index::{self, Location};
use crate::keyfile;
use crate::object::{Blob, EntryKind, Snapshot, Tree};
use crate::pack::{self, FileHash, PackEntry, PackWriter};
use crate::refs::validate_ref_name;
use crate::remote::{self, Remote, Store};

/// Called after each copied object with the totals so far; an error, such
/// as [`Error::Cancelled`], stops the push or pull.
pub type OnCopy<'a> = &'a mut dyn FnMut(&TransferReport) -> Result<()>;

/// What a push or pull did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferReport {
    /// The snapshot the ref points to on the target now.
    pub snapshot: ObjectId,
    /// The snapshot it pointed to before; `None` if the ref was new.
    pub previous: Option<ObjectId>,
    /// Objects copied.
    pub objects: u64,
    /// Their stored (compressed, encrypted) size.
    pub bytes: u64,
}

/// What happened to one ref when every ref was pushed or pulled: the
/// report, or the conflict that kept it where it was.
pub type RefOutcome = (String, std::result::Result<TransferReport, Error>);

/// `report` with the objects and bytes of the refs before it added.
fn summed(report: &TransferReport, (objects, bytes): (u64, u64)) -> TransferReport {
    let mut sum = report.clone();
    sum.objects += objects;
    sum.bytes += bytes;
    sum
}

/// The outcome of one ref of a push or pull of every ref: a conflict is
/// kept with the ref, any other error stops them all.
fn outcome(
    name: String,
    result: Result<TransferReport>,
    totals: &mut (u64, u64),
) -> Result<RefOutcome> {
    match result {
        Ok(report) => {
            totals.0 += report.objects;
            totals.1 += report.bytes;
            Ok((name, Ok(report)))
        }
        Err(e @ (Error::RemoteAhead(_) | Error::LocalAhead(_))) => Ok((name, Err(e))),
        Err(e) => Err(e),
    }
}

impl Repository {
    /// Makes a new repository at `path`, which must not exist or be empty,
    /// from the copy in `store`, then pulls every ref of it (FORMAT.md
    /// §15.6). `secret` opens an encrypted copy. A failure before the
    /// config is written removes what was created; a failure while pulling
    /// leaves a repository whose refs are pulled again with `pull`.
    /// `on_copy` gets the totals over all refs.
    pub fn clone_from(
        store: &dyn Store,
        path: &Path,
        secret: Option<&str>,
        on_copy: OnCopy<'_>,
    ) -> Result<(Repository, Vec<(String, TransferReport)>)> {
        let bytes = store
            .read(CONFIG)?
            .ok_or_else(|| Error::InvalidRepository {
                path: store.location(""),
                reason: "no config file".to_owned(),
            })?;
        let config_path = store.location(CONFIG);
        let text = String::from_utf8(bytes.clone()).map_err(|_| Error::InvalidRepository {
            path: config_path.clone(),
            reason: "config is not UTF-8".to_owned(),
        })?;
        let config = Config::from_toml(&config_path, &text)?;
        let encrypted = config.id_key.is_none();
        if encrypted && secret.is_none() {
            return Err(Error::KeyRequired(store.location("")));
        }
        let existed = path.exists();
        create_layout(path, encrypted)?;
        let unlock = match prepare_clone(store, path, &config, &bytes, secret) {
            Ok(unlock) => unlock,
            Err(e) => {
                // The folder was empty or missing before.
                let _ = fs::remove_dir_all(path);
                if existed {
                    let _ = fs::create_dir(path);
                }
                return Err(e);
            }
        };
        let mut repo = Repository::open_with(path, unlock)?;
        let mut reports = Vec::new();
        let (mut objects, mut bytes) = (0, 0);
        for name in store.list_refs()? {
            let report = repo.pull(store, &name, &mut |r| {
                let mut sum = r.clone();
                sum.objects += objects;
                sum.bytes += bytes;
                on_copy(&sum)
            })?;
            objects += report.objects;
            bytes += report.bytes;
            reports.push((name, report));
        }
        Ok((repo, reports))
    }

    /// Whether the snapshot `ancestor` is `id` or reachable from it through
    /// parents.
    pub fn is_ancestor(&self, ancestor: ObjectId, id: ObjectId) -> Result<bool> {
        is_ancestor(ancestor, id, |s| self.read_snapshot(s))
    }

    /// The nearest snapshot from `b` that is also an ancestor of `a`; `None`
    /// if their histories never meet.
    pub fn merge_base(&self, a: ObjectId, b: ObjectId) -> Result<Option<ObjectId>> {
        let mut of_a = HashSet::new();
        let mut stack = vec![a];
        while let Some(id) = stack.pop() {
            if of_a.insert(id) {
                stack.extend(self.read_snapshot(&id)?.parents);
            }
        }
        let mut seen = HashSet::new();
        let mut queue = std::collections::VecDeque::from([b]);
        while let Some(id) = queue.pop_front() {
            if of_a.contains(&id) {
                return Ok(Some(id));
            }
            if seen.insert(id) {
                queue.extend(self.read_snapshot(&id)?.parents);
            }
        }
        Ok(None)
    }

    /// The remotes of this copy, sorted by name.
    pub fn remotes(&self) -> Result<Vec<Remote>> {
        remote::read_remotes(&self.dir(REMOTES))
    }

    pub fn remote(&self, name: &str) -> Result<Remote> {
        self.remotes()?
            .into_iter()
            .find(|r| r.name == name)
            .ok_or_else(|| Error::RemoteNotFound(name.to_owned()))
    }

    pub fn add_remote(&self, name: &str, url: &str) -> Result<()> {
        remote::validate_remote_name(name)?;
        remote::validate_url(url)?;
        let mut remotes = self.remotes()?;
        if remotes.iter().any(|r| r.name == name) {
            return Err(Error::RemoteExists(name.to_owned()));
        }
        remotes.push(Remote {
            name: name.to_owned(),
            url: url.to_owned(),
        });
        remote::write_remotes(&self.dir(TMP), &self.dir(REMOTES), &remotes)
    }

    /// Forgets a remote; the copy it names is not touched.
    pub fn remove_remote(&self, name: &str) -> Result<()> {
        let mut remotes = self.remotes()?;
        let before = remotes.len();
        remotes.retain(|r| r.name != name);
        if remotes.len() == before {
            return Err(Error::RemoteNotFound(name.to_owned()));
        }
        remote::write_remotes(&self.dir(TMP), &self.dir(REMOTES), &remotes)
    }

    /// Copies the objects the ref `name` reaches to `store` and moves the
    /// ref there (FORMAT.md §15.3). The caller holds the repository lock.
    pub fn push(
        &self,
        store: &dyn Store,
        name: &str,
        on_copy: OnCopy<'_>,
    ) -> Result<TransferReport> {
        validate_ref_name(name)?;
        if self.read_ref(name)?.is_none() {
            return Err(Error::RefNotFound(name.to_owned()));
        }
        self.prepare_remote(store)?;
        let mut have = remote_index(store, &self.keys)?.into_keys().collect();
        self.push_ref(store, name, &mut have, on_copy)
    }

    /// Pushes every ref to `store`, reading its index once (FORMAT.md
    /// §15.7). A ref the remote has moved past is reported and the others
    /// are still pushed; any other error stops the push. `on_copy` gets the
    /// totals over all refs.
    pub fn push_all(&self, store: &dyn Store, on_copy: OnCopy<'_>) -> Result<Vec<RefOutcome>> {
        self.prepare_remote(store)?;
        let mut have = remote_index(store, &self.keys)?.into_keys().collect();
        let mut done = Vec::new();
        let mut totals = (0, 0);
        for (name, _) in self.list_refs()? {
            let result = self.push_ref(store, &name, &mut have, &mut |r| {
                on_copy(&summed(r, totals))
            });
            done.push(outcome(name, result, &mut totals)?);
        }
        Ok(done)
    }

    /// Pushes one ref; `have` holds the objects the remote has and gets
    /// the ones copied.
    fn push_ref(
        &self,
        store: &dyn Store,
        name: &str,
        have: &mut HashSet<ObjectId>,
        on_copy: OnCopy<'_>,
    ) -> Result<TransferReport> {
        let local = self
            .read_ref(name)?
            .ok_or_else(|| Error::RefNotFound(name.to_owned()))?;
        let previous = store.read_ref(name)?;
        let mut report = TransferReport {
            snapshot: local,
            previous,
            objects: 0,
            bytes: 0,
        };
        if previous == Some(local) {
            return Ok(report);
        }
        if let Some(remote) = previous
            && !is_ancestor(remote, local, |id| self.read_snapshot(id))?
        {
            return Err(Error::RemoteAhead(name.to_owned()));
        }

        let missing = missing_objects(
            local,
            |id| have.contains(id),
            |id, ty| self.read_typed(id, ty),
        )?;
        let copied: Vec<ObjectId> = missing.iter().map(|(id, _)| *id).collect();
        let mut packs = Vec::new();
        let mut writer: Option<PackWriter> = None;
        for (id, ty) in missing {
            let location = self.index.get(&id).ok_or(Error::ObjectNotFound(id))?;
            check_type(id, ty, location)?;
            let path = pack::pack_path(&self.dir(PACKS), &location.pack);
            let stored = pack::read_stored(&path, &location.entry, &self.keys)?;
            let w = match &mut writer {
                Some(w) => w,
                None => writer.insert(PackWriter::new(&self.dir(TMP), &self.keys)?),
            };
            w.add_stored(&location.entry, &stored)?;
            report.objects += 1;
            report.bytes += stored.len() as u64;
            on_copy(&report)?;
            if w.len() >= pack::TARGET_SIZE
                && let Some(w) = writer.take()
            {
                packs.push(write_pack(store, w)?);
            }
        }
        if let Some(w) = writer.take() {
            packs.push(write_pack(store, w)?);
        }
        if !packs.is_empty() {
            let (file, bytes) = index::encode_index(&packs, &self.keys)?;
            store.write_new(&format!("{INDEX}/{file}"), &bytes)?;
        }
        // Listed on the remote now, so the next ref need not copy them.
        have.extend(copied);
        store.swap_ref(name, previous, local)?;
        Ok(report)
    }

    /// Copies the objects the ref `name` reaches on `store` into this
    /// repository and moves the local ref forward (FORMAT.md §15.4).
    pub fn pull(
        &mut self,
        store: &dyn Store,
        name: &str,
        on_copy: OnCopy<'_>,
    ) -> Result<TransferReport> {
        validate_ref_name(name)?;
        let have = self.open_remote(store)?;
        self.pull_ref(store, &have, name, on_copy)
    }

    /// Pulls every ref of `store`, reading its index once (FORMAT.md
    /// §15.7). A ref this copy has moved past is reported and the others
    /// are still pulled; any other error stops the pull. `on_copy` gets the
    /// totals over all refs.
    pub fn pull_all(&mut self, store: &dyn Store, on_copy: OnCopy<'_>) -> Result<Vec<RefOutcome>> {
        let have = self.open_remote(store)?;
        let mut done = Vec::new();
        let mut totals = (0, 0);
        for name in store.list_refs()? {
            let result = self.pull_ref(store, &have, &name, &mut |r| on_copy(&summed(r, totals)));
            done.push(outcome(name, result, &mut totals)?);
        }
        Ok(done)
    }

    /// Pulls one ref; `have` is the remote's index.
    fn pull_ref(
        &mut self,
        store: &dyn Store,
        have: &HashMap<ObjectId, Location>,
        name: &str,
        on_copy: OnCopy<'_>,
    ) -> Result<TransferReport> {
        validate_ref_name(name)?;
        let remote = store
            .read_ref(name)?
            .ok_or_else(|| Error::RefNotFound(name.to_owned()))?;
        let previous = self.read_ref(name)?;
        let mut report = TransferReport {
            snapshot: remote,
            previous,
            objects: 0,
            bytes: 0,
        };
        if previous == Some(remote) {
            return Ok(report);
        }
        if let Some(local) = previous {
            let read_snapshot = |id: &ObjectId| {
                if self.contains(id) {
                    self.read_snapshot(id)
                } else {
                    Snapshot::decode(&read_remote(
                        store,
                        have,
                        &self.keys,
                        id,
                        ObjectType::Snapshot,
                    )?)
                }
            };
            if !is_ancestor(local, remote, read_snapshot)? {
                if self.contains(&remote) && is_ancestor(remote, local, read_snapshot)? {
                    // The local ref is already ahead.
                    report.snapshot = local;
                    return Ok(report);
                }
                return Err(Error::LocalAhead(name.to_owned()));
            }
        }
        self.copy_objects(store, have, &mut report, on_copy)?;
        self.update_ref(name, previous, Some(remote))?;
        Ok(report)
    }

    /// Copies the objects the ref `name` reaches on `store` into this
    /// repository without moving any local ref; `None` if the remote has no
    /// such ref. The objects stay unreachable until a ref points to them.
    pub fn fetch(
        &mut self,
        store: &dyn Store,
        name: &str,
        on_copy: OnCopy<'_>,
    ) -> Result<Option<TransferReport>> {
        validate_ref_name(name)?;
        let have = self.open_remote(store)?;
        let Some(remote) = store.read_ref(name)? else {
            return Ok(None);
        };
        let mut report = TransferReport {
            snapshot: remote,
            previous: None,
            objects: 0,
            bytes: 0,
        };
        self.copy_objects(store, &have, &mut report, on_copy)?;
        Ok(Some(report))
    }

    /// Checks that `store` holds a copy of this repository and reads its
    /// index files.
    fn open_remote(&self, store: &dyn Store) -> Result<HashMap<ObjectId, Location>> {
        match store.read(CONFIG)? {
            Some(bytes) => self.check_remote_config(store, &bytes)?,
            None => {
                return Err(Error::InvalidRepository {
                    path: store.location(""),
                    reason: "no config file".to_owned(),
                });
            }
        }
        self.take_rules(store)?;
        remote_index(store, &self.keys)
    }

    /// Copies what `report.snapshot` reaches and this repository lacks.
    fn copy_objects(
        &mut self,
        store: &dyn Store,
        have: &HashMap<ObjectId, Location>,
        report: &mut TransferReport,
        on_copy: OnCopy<'_>,
    ) -> Result<()> {
        let missing = missing_objects(
            report.snapshot,
            |id| self.contains(id),
            |id, ty| read_remote(store, have, &self.keys, id, ty),
        )?;

        // Read in pack order so that adjacent objects come in one read.
        let mut wanted: Vec<Location> = Vec::with_capacity(missing.len());
        for (id, ty) in missing {
            let location = have.get(&id).ok_or(Error::ObjectNotFound(id))?;
            check_type(id, ty, location)?;
            wanted.push(*location);
        }
        wanted.sort_by_key(|l| (l.pack, l.entry.offset));
        let mut writer = self.writer();
        let mut start = 0;
        while start < wanted.len() {
            let first = wanted[start];
            let mut end = start + 1;
            let mut len = u64::from(first.entry.stored_len);
            while let Some(next) = wanted.get(end) {
                if next.pack != first.pack
                    || next.entry.offset != first.entry.offset + len
                    || len >= pack::TARGET_SIZE
                {
                    break;
                }
                len += u64::from(next.entry.stored_len);
                end += 1;
            }
            let path = pack_file(&first.pack);
            let bytes = store.read_range(&path, first.entry.offset, len)?;
            let from = store.location(&path);
            for location in &wanted[start..end] {
                let at = (location.entry.offset - first.entry.offset) as usize;
                let stored = &bytes[at..at + location.entry.stored_len as usize];
                writer.put_stored(&location.entry, stored, &from)?;
                report.objects += 1;
                report.bytes += stored.len() as u64;
                on_copy(report)?;
            }
            start = end;
        }
        writer.finish()
    }

    /// Checks the remote `config`, or makes `store` a new copy of this
    /// repository; then makes its key files equal to the local ones.
    fn prepare_remote(&self, store: &dyn Store) -> Result<()> {
        match store.read(CONFIG)? {
            Some(bytes) => self.check_remote_config(store, &bytes)?,
            None => {
                let layout = [PACKS, INDEX, REFS, LOGS, LOCKS, KEYS, RETENTION, TMP];
                if store
                    .list("")?
                    .iter()
                    .any(|n| !layout.contains(&n.as_str()))
                {
                    return Err(Error::InvalidRepository {
                        path: store.location(""),
                        reason: "the folder is not empty and holds no repository".to_owned(),
                    });
                }
                store.create_dirs(&[PACKS, INDEX, REFS, LOGS, LOCKS, TMP])?;
                self.copy_keys(store)?;
                self.send_rules(store)?;
                let config = self.dir(CONFIG);
                return store.write_new(CONFIG, &fs::read(&config).at(&config)?);
            }
        }
        self.copy_keys(store)?;
        self.send_rules(store)
    }

    /// Writes the retention rule files the remote lacks; none is ever
    /// deleted (FORMAT.md §16.3).
    fn send_rules(&self, store: &dyn Store) -> Result<()> {
        let local = self.retention_files()?;
        if local.is_empty() {
            return Ok(());
        }
        let remote = store.list(RETENTION)?;
        store.create_dirs(&[RETENTION])?;
        for (name, bytes) in local {
            if !remote.contains(&name) {
                store.write_new(&format!("{RETENTION}/{name}"), &bytes)?;
            }
        }
        Ok(())
    }

    /// Writes the retention rule files of the remote this copy lacks
    /// (FORMAT.md §16.3).
    fn take_rules(&self, store: &dyn Store) -> Result<()> {
        let local = self.retention_files()?;
        for name in store.list(RETENTION)? {
            if !name.ends_with(".toml") || local.iter().any(|(n, _)| *n == name) {
                continue;
            }
            if let Some(bytes) = store.read(&format!("{RETENTION}/{name}"))? {
                self.write_retention_file(&(name, bytes))?;
            }
        }
        Ok(())
    }

    fn check_remote_config(&self, store: &dyn Store, bytes: &[u8]) -> Result<()> {
        let path = store.location(CONFIG);
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| Error::InvalidRepository {
            path: path.clone(),
            reason: "config is not UTF-8".to_owned(),
        })?;
        if Config::from_toml(&path, &text)?.id != self.config.id {
            return Err(Error::OtherRepository(store.location("")));
        }
        Ok(())
    }

    /// Writes missing key files first, then deletes the remote ones that
    /// were removed here (FORMAT.md §15.3 step 2).
    fn copy_keys(&self, store: &dyn Store) -> Result<()> {
        let local: Vec<(String, std::path::PathBuf)> = if self.is_encrypted() {
            keyfile::key_files(&self.dir(KEYS))?
                .into_iter()
                .map(|(id, path)| (format!("{id}.key"), path))
                .collect()
        } else {
            Vec::new()
        };
        let remote = store.list(KEYS)?;
        for (name, path) in &local {
            if !remote.contains(name) {
                store.write_new(&format!("{KEYS}/{name}"), &fs::read(path).at(path)?)?;
            }
        }
        for name in remote {
            if name.ends_with(".key") && !local.iter().any(|(n, _)| *n == name) {
                store.delete(&format!("{KEYS}/{name}"))?;
            }
        }
        Ok(())
    }
}

/// Copies the key files of `store` into the new repository at `path`,
/// opens them with `secret` and writes the config last.
fn prepare_clone(
    store: &dyn Store,
    path: &Path,
    config: &Config,
    config_bytes: &[u8],
    secret: Option<&str>,
) -> Result<Unlock<'static>> {
    let tmp = path.join(TMP);
    let unlock = match (&config.id_key, secret) {
        (Some(_), _) => Unlock::None,
        (None, secret) => {
            let keys_dir = path.join(KEYS);
            for name in store.list(KEYS)? {
                if !name.ends_with(".key") {
                    continue;
                }
                if let Some(bytes) = store.read(&format!("{KEYS}/{name}"))? {
                    fsutil::write_durable_new(&tmp, &keys_dir.join(&name), &bytes)?;
                }
            }
            fsutil::sync_dir(&keys_dir)?;
            let (keys, key_id) = keyfile::unlock(&keys_dir, &config.id, secret.unwrap_or(""))?;
            Unlock::Keys(keys, Some(key_id))
        }
    };
    fsutil::write_durable_new(&tmp, &path.join(CONFIG), config_bytes)?;
    Ok(unlock)
}

/// The objects listed by the healthy index files of a remote. Damaged
/// index files are skipped; their objects are copied again.
fn remote_index(store: &dyn Store, keys: &Keys) -> Result<HashMap<ObjectId, Location>> {
    let mut have = HashMap::new();
    for name in store.list(INDEX)? {
        if !name.ends_with(".idx") {
            continue;
        }
        let path = format!("{INDEX}/{name}");
        let Some(bytes) = store.read(&path)? else {
            continue;
        };
        match index::decode_index(&store.location(&path), &bytes, keys) {
            Ok(locations) => {
                for location in locations {
                    have.entry(location.entry.id).or_insert(location);
                }
            }
            Err(Error::Corrupt { .. }) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(have)
}

/// Reads a snapshot, tree or blob from a remote.
fn read_remote(
    store: &dyn Store,
    have: &HashMap<ObjectId, Location>,
    keys: &Keys,
    id: &ObjectId,
    ty: ObjectType,
) -> Result<Vec<u8>> {
    let location = have.get(id).ok_or(Error::ObjectNotFound(*id))?;
    check_type(*id, ty, location)?;
    let path = pack_file(&location.pack);
    let entry = &location.entry;
    let stored = store.read_range(&path, entry.offset, u64::from(entry.stored_len))?;
    pack::decode_stored(&store.location(&path), entry, keys, &stored)
}

fn pack_file(name: &FileHash) -> String {
    let hex = to_hex(name);
    format!("{PACKS}/{}/{hex}.pack", &hex[..2])
}

fn write_pack(store: &dyn Store, writer: PackWriter) -> Result<(FileHash, Vec<PackEntry>)> {
    let (name, entries, bytes) = writer.finish_to_bytes()?;
    store.write_new(&pack_file(&name), &bytes)?;
    Ok((name, entries))
}

fn check_type(id: ObjectId, expected: ObjectType, location: &Location) -> Result<()> {
    if location.entry.ty != expected {
        return Err(Error::WrongObjectType {
            id,
            expected,
            found: location.entry.ty,
        });
    }
    Ok(())
}

/// Whether `ancestor` is `start` or reachable from it through parents.
fn is_ancestor(
    ancestor: ObjectId,
    start: ObjectId,
    read: impl Fn(&ObjectId) -> Result<Snapshot>,
) -> Result<bool> {
    let mut seen = HashSet::new();
    let mut stack = vec![start];
    while let Some(id) = stack.pop() {
        if id == ancestor {
            return Ok(true);
        }
        if seen.insert(id) {
            stack.extend(read(&id)?.parents);
        }
    }
    Ok(false)
}

/// The objects reachable from the snapshot `start` that the target does not
/// have, with their expected types. An object the target has is not
/// entered: everything it reaches is there too (FORMAT.md §15.5). `read`
/// returns the content of a snapshot, tree or blob.
fn missing_objects(
    start: ObjectId,
    has: impl Fn(&ObjectId) -> bool,
    read: impl Fn(&ObjectId, ObjectType) -> Result<Vec<u8>>,
) -> Result<Vec<(ObjectId, ObjectType)>> {
    let mut seen = HashSet::new();
    let mut missing = Vec::new();
    let mut stack = vec![(start, ObjectType::Snapshot)];
    while let Some((id, ty)) = stack.pop() {
        if has(&id) || !seen.insert(id) {
            continue;
        }
        missing.push((id, ty));
        match ty {
            ObjectType::Snapshot => {
                let snapshot = Snapshot::decode(&read(&id, ty)?)?;
                stack.push((snapshot.tree, ObjectType::Tree));
                stack.extend(snapshot.parents.iter().map(|p| (*p, ObjectType::Snapshot)));
            }
            ObjectType::Tree => {
                for entry in Tree::decode(&read(&id, ty)?)?.entries {
                    match entry.kind {
                        EntryKind::File { blob: Some(b), .. } => stack.push((b, ObjectType::Blob)),
                        EntryKind::Directory { tree } => stack.push((tree, ObjectType::Tree)),
                        EntryKind::File { blob: None, .. } | EntryKind::Symlink { .. } => {}
                    }
                }
            }
            ObjectType::Blob => {
                for (chunk, _) in Blob::decode(&read(&id, ty)?)?.chunks {
                    stack.push((chunk, ObjectType::Chunk));
                }
            }
            ObjectType::Chunk => {}
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::chunker::ChunkerParams;
    use crate::chunker::tests::noise;
    use crate::object::{Time, TreeEntry};
    use crate::remote::FolderStore;

    const PASSWORD: &str = "pw";

    fn init(path: &Path, encrypted: bool) -> Repository {
        let params = ChunkerParams::new(64 * 1024).unwrap();
        if encrypted {
            Repository::init_encrypted(path, params, PASSWORD)
                .unwrap()
                .0
        } else {
            Repository::init_with(path, params).unwrap()
        }
    }

    /// Stores a snapshot of one file and moves `main` to it.
    fn commit(repo: &mut Repository, data: &[u8], parent: Option<ObjectId>) -> ObjectId {
        let mut w = repo.writer();
        let (size, blob) = w.put_file(data).unwrap();
        let tree = w
            .put_tree(&Tree {
                entries: vec![TreeEntry {
                    name: b"file".to_vec(),
                    kind: EntryKind::File { size, blob },
                    mode: None,
                    mtime: None,
                    uid: None,
                    gid: None,
                    xattrs: vec![],
                }],
            })
            .unwrap();
        let snapshot = w
            .put_snapshot(&Snapshot {
                tree,
                parents: parent.into_iter().collect(),
                time: Time {
                    seconds: 1_700_000_000,
                    nanos: 0,
                },
                author: None,
                host: None,
                message: None,
                meta: vec![],
            })
            .unwrap();
        w.finish().unwrap();
        repo.update_ref("main", parent, Some(snapshot)).unwrap();
        snapshot
    }

    /// Moves `name` from `parent` to a new snapshot of `main`'s content.
    fn commit_on(repo: &mut Repository, name: &str, parent: ObjectId) -> ObjectId {
        let parent_snapshot = repo.read_snapshot(&parent).unwrap();
        let mut w = repo.writer();
        let snapshot = w
            .put_snapshot(&Snapshot {
                parents: vec![parent],
                message: Some(name.to_owned()),
                ..parent_snapshot
            })
            .unwrap();
        w.finish().unwrap();
        repo.update_ref(name, Some(parent), Some(snapshot)).unwrap();
        snapshot
    }

    fn content(repo: &Repository, snapshot: &ObjectId) -> Vec<u8> {
        let tree = repo
            .read_tree(&repo.read_snapshot(snapshot).unwrap().tree)
            .unwrap();
        let EntryKind::File {
            blob: Some(blob), ..
        } = tree.entries[0].kind
        else {
            panic!("expected a file");
        };
        let mut out = Vec::new();
        repo.read_file(&blob, &mut out).unwrap();
        out
    }

    fn copy_dir(src: &Path, dest: &Path) {
        fs::create_dir_all(dest).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &dest.join(entry.file_name()));
            } else {
                fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
            }
        }
    }

    fn push(repo: &Repository, remote: &Path) -> Result<TransferReport> {
        repo.push(&FolderStore::open(remote)?, "main", &mut |_| Ok(()))
    }

    fn pull(repo: &mut Repository, remote: &Path) -> Result<TransferReport> {
        repo.pull(&FolderStore::open(remote)?, "main", &mut |_| Ok(()))
    }

    struct Setup {
        _dir: tempfile::TempDir,
        root: PathBuf,
        a: Repository,
        first: ObjectId,
    }

    /// Repository `a` with one snapshot on `main`.
    fn setup(encrypted: bool) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let mut a = init(&root.join("a"), encrypted);
        let first = commit(&mut a, &noise(300 * 1024, 1), None);
        Setup {
            _dir: dir,
            root,
            a,
            first,
        }
    }

    #[test]
    fn push_and_pull_move_snapshots_between_copies() {
        for encrypted in [false, true] {
            push_and_pull_move_snapshots_between_copies_with(encrypted);
        }
    }

    fn push_and_pull_move_snapshots_between_copies_with(encrypted: bool) {
        let Setup {
            _dir,
            root,
            mut a,
            first,
        } = setup(encrypted);
        let remote = root.join("remote");
        let report = push(&a, &remote).unwrap();
        assert_eq!((report.snapshot, report.previous), (first, None));
        assert!(report.objects > 5 && report.bytes > 0);
        assert_eq!(push(&a, &remote).unwrap().objects, 0);

        // The remote is a complete repository with the same keys.
        let copy = Repository::unlock(&remote, PASSWORD).unwrap();
        assert_eq!(copy.read_ref("main").unwrap(), Some(first));
        assert_eq!(content(&copy, &first), noise(300 * 1024, 1));
        assert!(copy.verify().unwrap().is_ok());
        copy_dir(&remote, &root.join("b"));
        let mut b = Repository::unlock(&root.join("b"), PASSWORD).unwrap();

        // Only the new objects travel; the unchanged chunks stay behind.
        let mut data = noise(300 * 1024, 1);
        data.extend(noise(100 * 1024, 2));
        let second = commit(&mut a, &data, Some(first));
        let report = push(&a, &remote).unwrap();
        assert_eq!(report.previous, Some(first));
        assert!(report.bytes < 200 * 1024, "{report:?}");

        let report = pull(&mut b, &remote).unwrap();
        assert_eq!((report.snapshot, report.previous), (second, Some(first)));
        assert!(report.objects > 0);
        assert_eq!(content(&b, &second), data);
        assert!(b.verify().unwrap().is_ok());
        let b = b.reopen().unwrap();
        assert_eq!(b.read_ref("main").unwrap(), Some(second));
        assert_eq!(b.reflog("main").unwrap().last().unwrap().new, Some(second));
        assert_eq!(pull(&mut a, &remote).unwrap().objects, 0);
    }

    #[test]
    fn clone_copies_keys_config_and_every_ref() {
        for encrypted in [false, true] {
            let Setup {
                _dir,
                root,
                mut a,
                first,
            } = setup(encrypted);
            let remote = root.join("remote");
            push(&a, &remote).unwrap();
            let other = commit(&mut a, b"other", Some(first));
            a.update_ref("sync/docs", None, Some(other)).unwrap();
            let store = FolderStore::open(&remote).unwrap();
            a.push(&store, "sync/docs", &mut |_| Ok(())).unwrap();

            let path = root.join("b");
            fs::create_dir(&path).unwrap();
            let (b, reports) =
                Repository::clone_from(&store, &path, Some(PASSWORD), &mut |_| Ok(())).unwrap();
            let names: Vec<&str> = reports.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, ["main", "sync/docs"]);
            assert_eq!(b.read_ref("main").unwrap(), Some(first));
            assert_eq!(content(&b, &other), b"other");
            assert!(b.verify().unwrap().is_ok());
            assert_eq!(b.is_encrypted(), encrypted);
            Repository::unlock(&path, PASSWORD).unwrap();
            // The clone is the same repository: push and pull work both ways.
            assert_eq!(pull(&mut b.reopen().unwrap(), &remote).unwrap().objects, 0);

            // A folder that is not empty is refused and left alone.
            assert!(matches!(
                Repository::clone_from(&store, &path, Some(PASSWORD), &mut |_| Ok(())),
                Err(Error::InvalidRepository { .. })
            ));
            assert!(path.join("config").exists());
        }
    }

    #[test]
    fn clone_with_a_wrong_password_leaves_nothing() {
        let s = setup(true);
        let remote = s.root.join("remote");
        push(&s.a, &remote).unwrap();
        let store = FolderStore::open(&remote).unwrap();
        let path = s.root.join("b");
        assert!(matches!(
            Repository::clone_from(&store, &path, Some("wrong"), &mut |_| Ok(())),
            Err(Error::WrongKey)
        ));
        assert!(!path.exists());
        assert!(matches!(
            Repository::clone_from(&store, &path, None, &mut |_| Ok(())),
            Err(Error::KeyRequired(_))
        ));
        fs::create_dir(&path).unwrap();
        assert!(Repository::clone_from(&store, &path, Some("wrong"), &mut |_| Ok(())).is_err());
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        assert!(matches!(
            Repository::clone_from(
                &FolderStore::open(&s.root.join("none")).unwrap(),
                &s.root.join("c"),
                None,
                &mut |_| Ok(())
            ),
            Err(Error::InvalidRepository { .. })
        ));
    }

    #[test]
    fn every_ref_is_pushed_and_pulled_past_a_conflict() {
        let Setup {
            _dir,
            root,
            mut a,
            first,
        } = setup(true);
        a.update_ref("backup/docs", None, Some(first)).unwrap();
        let remote = root.join("remote");
        let store = FolderStore::open(&remote).unwrap();
        let mut seen = 0;
        let pushed = a
            .push_all(&store, &mut |r| {
                assert!(r.objects > seen);
                seen = r.objects;
                Ok(())
            })
            .unwrap();
        let names: Vec<&str> = pushed.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["backup/docs", "main"]);
        // The second ref reaches only objects the first one copied.
        let copied: Vec<u64> = pushed
            .iter()
            .map(|(_, r)| r.as_ref().unwrap().objects)
            .collect();
        assert!(copied[0] > 0);
        assert_eq!(copied[1], 0);

        // A copy that went its own way on `main` still gets `backup/docs`.
        drop(store);
        copy_dir(&remote, &root.join("b"));
        let mut b = Repository::unlock(&root.join("b"), PASSWORD).unwrap();
        let on_b = commit(&mut b, b"b", Some(first));
        let on_a = commit(&mut a, b"a", Some(first));
        let docs = commit_on(&mut a, "backup/docs", first);
        let store = FolderStore::open(&remote).unwrap();
        a.push_all(&store, &mut |_| Ok(())).unwrap();
        let pulled = b.pull_all(&store, &mut |_| Ok(())).unwrap();
        assert_eq!(pulled[0].0, "backup/docs");
        assert_eq!(pulled[0].1.as_ref().unwrap().snapshot, docs);
        assert!(matches!(pulled[1], (_, Err(Error::LocalAhead(_)))));
        assert_eq!(b.read_ref("main").unwrap(), Some(on_b));
        assert_eq!(b.read_ref("backup/docs").unwrap(), Some(docs));
        let pushed = b.push_all(&store, &mut |_| Ok(())).unwrap();
        assert!(pushed[0].1.as_ref().is_ok());
        assert!(matches!(pushed[1], (_, Err(Error::RemoteAhead(_)))));
        assert_eq!(store.read_ref("main").unwrap(), Some(on_a));

        // Any other error stops them all.
        let elsewhere = init(&root.join("c"), true);
        assert!(matches!(
            elsewhere.push_all(&store, &mut |_| Ok(())),
            Err(Error::OtherRepository(_))
        ));
    }

    #[test]
    fn retention_keeps_history_and_travels_with_every_copy() {
        let Setup {
            _dir,
            root,
            mut a,
            first,
        } = setup(true);
        a.update_ref("archive/docs", None, Some(first)).unwrap();
        a.add_retention_rule("archive/*", 3650).unwrap();
        // A ref that no rule matches changes freely.
        let second = commit(&mut a, b"second", Some(first));
        a.update_ref("main", Some(second), Some(first)).unwrap();

        // A kept ref only moves forward and is not deleted.
        let docs = commit_on(&mut a, "archive/docs", first);
        let refused = |r: Result<()>| matches!(r, Err(Error::Retained { .. }));
        assert!(refused(a.update_ref(
            "archive/docs",
            Some(docs),
            Some(first)
        )));
        assert!(refused(a.update_ref(
            "archive/docs",
            Some(docs),
            Some(second)
        )));
        assert!(refused(a.update_ref("archive/docs", Some(docs), None)));
        assert_eq!(a.read_ref("archive/docs").unwrap(), Some(docs));
        // A new ref may be made under the rule.
        a.update_ref("archive/new", None, Some(first)).unwrap();

        // The rule goes with a push and comes back with a clone.
        let remote = root.join("remote");
        let store = FolderStore::open(&remote).unwrap();
        a.push_all(&store, &mut |_| Ok(())).unwrap();
        let copy = Repository::unlock(&remote, PASSWORD).unwrap();
        assert_eq!(
            copy.retention_rules().unwrap(),
            a.retention_rules().unwrap()
        );
        assert!(refused(copy.update_ref("archive/docs", Some(docs), None)));
        let (b, _) =
            Repository::clone_from(&store, &root.join("b"), Some(PASSWORD), &mut |_| Ok(()))
                .unwrap();
        assert_eq!(b.retention_rules().unwrap().len(), 1);

        // A longer rule adds to the first; neither goes away.
        a.add_retention_rule("archive/docs", 36500).unwrap();
        assert_eq!(a.retention_rules().unwrap().len(), 2);
        assert!(a.add_retention_rule("archive/*", 0).is_err());

        // A damaged rule file stops every ref change until it is restored.
        let rules = root.join("a").join(RETENTION);
        let file = fs::read_dir(&rules)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let kept = fs::read(&file).unwrap();
        fs::write(&file, b"refs = \"x\"\ndays = 1\nadded = 0\n").unwrap();
        assert!(matches!(
            a.update_ref("main", Some(first), Some(second)),
            Err(Error::Corrupt { .. })
        ));
        fs::write(&file, kept).unwrap();
        a.update_ref("main", Some(first), Some(second)).unwrap();
    }

    #[test]
    fn diverged_refs_are_refused_and_change_nothing() {
        let Setup {
            _dir,
            root,
            mut a,
            first,
        } = setup(false);
        let remote = root.join("remote");
        push(&a, &remote).unwrap();
        copy_dir(&remote, &root.join("b"));
        let mut b = Repository::open(&root.join("b")).unwrap();
        let on_a = commit(&mut a, b"a", Some(first));
        push(&a, &remote).unwrap();
        let on_b = commit(&mut b, b"b", Some(first));

        assert!(matches!(push(&b, &remote), Err(Error::RemoteAhead(_))));
        assert!(matches!(pull(&mut b, &remote), Err(Error::LocalAhead(_))));
        assert_eq!(b.read_ref("main").unwrap(), Some(on_b));
        // Fetch copies what the remote has and moves no ref.
        let store = FolderStore::open(&remote).unwrap();
        let fetched = b.fetch(&store, "main", &mut |_| Ok(())).unwrap().unwrap();
        assert_eq!(fetched.snapshot, on_a);
        assert!(fetched.objects > 0);
        assert_eq!(content(&b, &on_a), b"a");
        assert_eq!(b.read_ref("main").unwrap(), Some(on_b));
        assert!(b.fetch(&store, "other", &mut |_| Ok(())).unwrap().is_none());
        assert_eq!(b.merge_base(on_b, on_a).unwrap(), Some(first));
        assert_eq!(b.merge_base(on_a, first).unwrap(), Some(first));
        assert!(b.is_ancestor(first, on_a).unwrap());
        assert!(!b.is_ancestor(on_a, on_b).unwrap());
        drop(store);
        let store = FolderStore::open(&remote).unwrap();
        assert_eq!(store.read_ref("main").unwrap(), Some(on_a));

        // A copy that is ahead has nothing to pull.
        let later = commit(&mut a, b"later", Some(on_a));
        let report = pull(&mut a, &remote).unwrap();
        assert_eq!((report.snapshot, report.objects), (later, 0));
        assert!(matches!(
            pull(&mut a, &root.join("nothing")),
            Err(Error::InvalidRepository { .. })
        ));
        assert!(matches!(
            a.pull(&store, "other", &mut |_| Ok(())),
            Err(Error::RefNotFound(_))
        ));
    }

    #[test]
    fn push_refuses_other_repositories_and_foreign_folders() {
        let s = setup(false);
        let other = init(&s.root.join("other"), false);
        assert!(matches!(
            push(&s.a, other.path()),
            Err(Error::OtherRepository(_))
        ));
        let folder = s.root.join("photos");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("cat.jpg"), b"meow").unwrap();
        assert!(matches!(
            push(&s.a, &folder),
            Err(Error::InvalidRepository { .. })
        ));
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
    }

    #[test]
    fn push_mirrors_key_files() {
        let s = setup(true);
        let remote = s.root.join("remote");
        push(&s.a, &remote).unwrap();
        let added = s.a.add_password("second").unwrap();
        let old: Vec<String> = s.a.list_keys().unwrap().into_iter().map(|k| k.id).collect();
        for id in old.iter().filter(|id| **id != added) {
            s.a.remove_key(id).unwrap();
        }
        push(&s.a, &remote).unwrap();
        let names = FolderStore::open(&remote).unwrap().list(KEYS).unwrap();
        assert_eq!(names, vec![format!("{added}.key")]);
        assert!(matches!(
            Repository::unlock(&remote, PASSWORD),
            Err(Error::WrongKey)
        ));
        Repository::unlock(&remote, "second").unwrap();
    }

    #[test]
    fn damaged_remote_index_is_skipped_and_its_objects_are_sent_again() {
        let s = setup(true);
        let remote = s.root.join("remote");
        let sent = push(&s.a, &remote).unwrap().objects;
        let index = remote.join(INDEX);
        let file = fs::read_dir(&index)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut bytes = fs::read(&file).unwrap();
        bytes[20] ^= 1;
        fs::write(&file, bytes).unwrap();

        let mut a = s.a;
        let second = commit(&mut a, b"more", Some(s.first));
        let report = push(&a, &remote).unwrap();
        assert!(report.objects > sent, "{report:?}");
        let mut copy = Repository::unlock(&remote, PASSWORD).unwrap();
        copy.repair().unwrap();
        assert_eq!(content(&copy, &second), b"more");
        assert_eq!(content(&copy, &s.first), noise(300 * 1024, 1));
    }
}
