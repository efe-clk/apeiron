//! A repository on local disk (FORMAT.md §2).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::chunker::ChunkerParams;
use crate::config::Config;
use crate::crypto::{Keys, SecretKey};
use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::{IdKey, ObjectId, ObjectType};
use crate::index::{self, Location};
use crate::keyfile::{self, KeyInfo};
use crate::lock::RepoLock;
use crate::object::{Blob, EntryKind, Snapshot, Tree};
use crate::pack::{self, FileHash, PackEntry, PackWriter};
use crate::refs::{RefStore, ReflogEntry, RepairReport};

mod gc;
mod transfer;

pub use gc::{GcOptions, GcReport};
pub use transfer::{OnCopy, TransferReport};

const CONFIG: &str = "config";
const PACKS: &str = "packs";
const INDEX: &str = "index";
const REFS: &str = "refs";
const LOGS: &str = "logs";
const LOCKS: &str = "locks";
const KEYS: &str = "keys";
const TMP: &str = "tmp";
const REMOTES: &str = "remotes";
const TMP_MAX_AGE: Duration = Duration::from_secs(3600);

/// An open repository.
#[derive(Debug)]
pub struct Repository {
    root: PathBuf,
    config: Config,
    keys: Keys,
    /// The key file an encrypted repository was opened with.
    key_id: Option<String>,
    index: HashMap<ObjectId, Location>,
    /// The index files `index` was loaded from.
    index_files: Vec<PathBuf>,
    /// Healthy packs that no healthy index covers.
    unindexed: Vec<(FileHash, Vec<PackEntry>)>,
    /// Index and pack files that failed verification while loading.
    damaged: Vec<PathBuf>,
    refs: RefStore,
}

impl Repository {
    /// Creates a repository with default chunker parameters.
    /// `path` must not exist or be an empty directory.
    pub fn init(path: &Path) -> Result<Self> {
        Repository::init_with(path, ChunkerParams::default())
    }

    pub fn init_with(path: &Path, chunker: ChunkerParams) -> Result<Self> {
        create_layout(path, false)?;
        let config = Config::generate(chunker, Some(IdKey::generate()?))?;
        write_config(path, &config)?;
        Repository::open(path)
    }

    /// Creates an encrypted repository (FORMAT.md §14) with one password
    /// key file and one recovery key file. Returns the repository and the
    /// recovery key, which is shown to the user once and never stored.
    pub fn init_encrypted(
        path: &Path,
        chunker: ChunkerParams,
        password: &str,
    ) -> Result<(Self, String)> {
        create_layout(path, true)?;
        let config = Config::generate(chunker, None)?;
        let (id_key, data_key) = (IdKey::generate()?, SecretKey::generate()?);
        let (keys_dir, tmp) = (path.join(KEYS), path.join(TMP));
        let key_id =
            keyfile::write_password(&keys_dir, &tmp, &config.id, &id_key, &data_key, password)?;
        let (_, recovery) =
            keyfile::write_recovery(&keys_dir, &tmp, &config.id, &id_key, &data_key)?;
        fsutil::sync_dir(&keys_dir)?;
        write_config(path, &config)?;
        let keys = Keys {
            id: id_key,
            data: Some(data_key),
        };
        let repo = Repository::open_with(path, Unlock::Keys(keys, Some(key_id)))?;
        Ok((repo, recovery))
    }

    /// Opens a repository. An encrypted one fails with
    /// [`Error::KeyRequired`]; see [`Repository::unlock`].
    pub fn open(path: &Path) -> Result<Self> {
        Repository::open_with(path, Unlock::None)
    }

    /// Opens a repository with a password or recovery key. A repository that
    /// is not encrypted opens without using `secret`.
    pub fn unlock(path: &Path, secret: &str) -> Result<Self> {
        Repository::open_with(path, Unlock::Secret(secret))
    }

    /// Opens the same repository again, with the keys this one was opened
    /// with, and loads its index from disk.
    pub fn reopen(&self) -> Result<Self> {
        Repository::open_with(
            &self.root,
            Unlock::Keys(self.keys.clone(), self.key_id.clone()),
        )
    }

    fn open_with(path: &Path, unlock: Unlock<'_>) -> Result<Self> {
        let config_path = path.join(CONFIG);
        let text = match fs::read_to_string(&config_path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::InvalidRepository {
                    path: path.to_path_buf(),
                    reason: "no config file".to_owned(),
                });
            }
            Err(e) => return Err(e).at(&config_path),
        };
        let config = Config::from_toml(&config_path, &text)?;
        let (keys, key_id) = match (&config.id_key, unlock) {
            (Some(id), _) => {
                let keys = Keys {
                    id: id.clone(),
                    data: None,
                };
                (keys, None)
            }
            (None, Unlock::None) => return Err(Error::KeyRequired(path.to_path_buf())),
            (None, Unlock::Secret(secret)) => {
                let (keys, key_id) = keyfile::unlock(&path.join(KEYS), &config.id, secret)?;
                (keys, Some(key_id))
            }
            (None, Unlock::Keys(keys, key_id)) => (keys, key_id),
        };
        let mut repo = Repository {
            root: path.to_path_buf(),
            config,
            keys,
            key_id,
            index: HashMap::new(),
            index_files: Vec::new(),
            unindexed: Vec::new(),
            damaged: Vec::new(),
            refs: RefStore::new(path.join(REFS), path.join(LOGS), path.join(TMP)),
        };
        repo.load_index()?;
        repo.clean_tmp();
        Ok(repo)
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn chunker(&self) -> ChunkerParams {
        self.config.chunker
    }

    pub fn is_encrypted(&self) -> bool {
        self.config.is_encrypted()
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn data_key(&self) -> Result<&SecretKey> {
        self.keys.data.as_ref().ok_or(Error::NotEncrypted)
    }

    /// The passwords and recovery keys of an encrypted repository, oldest
    /// first (FORMAT.md §14.2).
    pub fn list_keys(&self) -> Result<Vec<KeyInfo>> {
        self.data_key()?;
        keyfile::list(&self.dir(KEYS), self.key_id.as_deref())
    }

    /// Adds a password and returns the id of its key file.
    pub fn add_password(&self, password: &str) -> Result<String> {
        keyfile::write_password(
            &self.dir(KEYS),
            &self.dir(TMP),
            &self.config.id,
            &self.keys.id,
            self.data_key()?,
            password,
        )
    }

    /// Adds a new recovery key. Returns the id of its key file and the
    /// recovery key, which is shown to the user once and never stored.
    pub fn add_recovery_key(&self) -> Result<(String, String)> {
        keyfile::write_recovery(
            &self.dir(KEYS),
            &self.dir(TMP),
            &self.config.id,
            &self.keys.id,
            self.data_key()?,
        )
    }

    /// Removes a password or recovery key, unless it is the last one. Takes
    /// the repository lock exclusive.
    pub fn remove_key(&self, id: &str) -> Result<()> {
        self.data_key()?;
        let _lock = RepoLock::exclusive(&self.root)?;
        keyfile::remove(&self.dir(KEYS), id)
    }

    /// Forgets the loaded index and loads it again from disk.
    pub fn reload_index(&mut self) -> Result<()> {
        self.index.clear();
        self.load_index()
    }

    /// Reloads the index if another program changed the index files, for
    /// example by garbage collection. Returns whether it reloaded.
    pub fn refresh_index(&mut self) -> Result<bool> {
        let mut current = list_files(&self.dir(INDEX))?;
        current.sort();
        if current == self.index_files {
            return Ok(false);
        }
        self.reload_index()?;
        Ok(true)
    }

    /// Loads all index files, then reads the tables of packs no index covers
    /// (left behind by a crash between writing a pack and its index).
    /// Damaged index files are skipped and their packs read directly; damaged
    /// packs without an index are skipped (FORMAT.md §8.2). Both are recorded
    /// in [`Repository::damaged_files`].
    fn load_index(&mut self) -> Result<()> {
        self.unindexed.clear();
        self.damaged.clear();
        let mut indexed: HashSet<FileHash> = HashSet::new();
        let index_dir = self.dir(INDEX);
        let mut index_files = list_files(&index_dir)?;
        index_files.sort();
        self.index_files = index_files.clone();
        for path in index_files {
            if index::hash_from_file_name(&path, ".idx").is_none() {
                continue;
            }
            let locations = match index::read_index(&path, &self.keys) {
                Ok(locations) => locations,
                Err(Error::Corrupt { .. }) => {
                    self.damaged.push(path);
                    continue;
                }
                Err(e) => return Err(e),
            };
            for location in locations {
                indexed.insert(location.pack);
                self.index.entry(location.entry.id).or_insert(location);
            }
        }

        let packs_dir = self.dir(PACKS);
        for shard in list_dirs(&packs_dir)? {
            for path in list_files(&shard)? {
                let Some(name) = index::hash_from_file_name(&path, ".pack") else {
                    continue;
                };
                if indexed.contains(&name) {
                    continue;
                }
                let entries = match pack::verify_file_hash(&path, &name)
                    .and_then(|()| pack::read_table(&path, &self.keys))
                {
                    Ok(entries) => entries,
                    Err(Error::Corrupt { .. }) => {
                        self.damaged.push(path);
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                for entry in &entries {
                    self.index.entry(entry.id).or_insert(Location {
                        pack: name,
                        entry: *entry,
                    });
                }
                self.unindexed.push((name, entries));
            }
        }
        Ok(())
    }

    /// Best-effort removal of stale temporary files.
    fn clean_tmp(&self) {
        let Ok(files) = list_files(&self.dir(TMP)) else {
            return;
        };
        let now = SystemTime::now();
        for path in files {
            let stale = fs::metadata(&path)
                .and_then(|m| m.modified())
                .is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age > TMP_MAX_AGE));
            if stale {
                let _ = fs::remove_file(&path);
            }
        }
    }

    /// Whether the object is stored and indexed.
    pub fn contains(&self, id: &ObjectId) -> bool {
        self.index.contains_key(id)
    }

    /// Reads an object and verifies its hash.
    pub fn read_object(&self, id: &ObjectId) -> Result<(ObjectType, Vec<u8>)> {
        let location = self.index.get(id).ok_or(Error::ObjectNotFound(*id))?;
        let path = pack::pack_path(&self.dir(PACKS), &location.pack);
        let data = pack::read_object(&path, &location.entry, &self.keys)?;
        Ok((location.entry.ty, data))
    }

    /// Reads an object that must have type `ty`.
    fn read_typed(&self, id: &ObjectId, ty: ObjectType) -> Result<Vec<u8>> {
        let (found, data) = self.read_object(id)?;
        if found != ty {
            return Err(Error::WrongObjectType {
                id: *id,
                expected: ty,
                found,
            });
        }
        Ok(data)
    }

    /// Decodes a stored structured object; a stored object that does not
    /// decode is corrupt.
    fn decode_stored<T>(
        &self,
        id: &ObjectId,
        decode: fn(&[u8]) -> Result<T>,
        data: &[u8],
    ) -> Result<T> {
        decode(data).map_err(|e| match e {
            Error::InvalidObject { reason, .. } => {
                let pack = self.index.get(id).map(|l| l.pack).unwrap_or_default();
                Error::corrupt(
                    &pack::pack_path(&self.dir(PACKS), &pack),
                    format!("object {id}: {reason}"),
                )
            }
            e => e,
        })
    }

    pub fn read_tree(&self, id: &ObjectId) -> Result<Tree> {
        let data = self.read_typed(id, ObjectType::Tree)?;
        self.decode_stored(id, Tree::decode, &data)
    }

    pub fn read_snapshot(&self, id: &ObjectId) -> Result<Snapshot> {
        let data = self.read_typed(id, ObjectType::Snapshot)?;
        self.decode_stored(id, Snapshot::decode, &data)
    }

    pub fn read_blob(&self, id: &ObjectId) -> Result<Blob> {
        let data = self.read_typed(id, ObjectType::Blob)?;
        self.decode_stored(id, Blob::decode, &data)
    }

    /// Writes the content of the blob `id` to `out`, verifying every chunk.
    /// Returns the number of bytes written.
    pub fn read_file<W: Write>(&self, id: &ObjectId, mut out: W) -> Result<u64> {
        let blob = self.read_blob(id)?;
        for (chunk_id, len) in &blob.chunks {
            let chunk = self.read_chunk(chunk_id, *len)?;
            out.write_all(&chunk).map_err(Error::Output)?;
        }
        out.flush().map_err(Error::Output)?;
        Ok(blob.size)
    }

    /// Reads one chunk of a blob and checks its length.
    pub fn read_chunk(&self, id: &ObjectId, len: u32) -> Result<Vec<u8>> {
        let chunk = self.read_typed(id, ObjectType::Chunk)?;
        if chunk.len() != len as usize {
            return Err(Error::corrupt(
                &self.dir(PACKS),
                format!("chunk {id} has the wrong length"),
            ));
        }
        Ok(chunk)
    }

    /// Splits `reader` into chunks as a backup would and returns each
    /// chunk's id and length, without storing anything.
    pub fn chunk_ids<R: Read>(&self, reader: R) -> Result<Vec<(ObjectId, u32)>> {
        let mut ids = Vec::new();
        for chunk in self.config.chunker.chunks(reader) {
            let chunk = chunk?;
            // Chunks are at most `max_size` (16 MiB), so the length fits in a u32.
            ids.push((
                self.keys.id.object_id(ObjectType::Chunk, &chunk),
                chunk.len() as u32,
            ));
        }
        Ok(ids)
    }

    /// Stored snapshots whose hex id starts with `prefix`, sorted.
    pub fn snapshots_with_prefix(&self, prefix: &str) -> Vec<ObjectId> {
        let mut ids: Vec<_> = self
            .index
            .iter()
            .filter(|(id, l)| l.entry.ty == ObjectType::Snapshot && id.to_hex().starts_with(prefix))
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }

    /// Walks from the snapshot `id` back through first parents.
    pub fn history(&self, id: ObjectId) -> impl Iterator<Item = Result<(ObjectId, Snapshot)>> + '_ {
        let mut next = Some(id);
        std::iter::from_fn(move || {
            let id = next.take()?;
            let result = self.read_snapshot(&id);
            if let Ok(snapshot) = &result {
                next = snapshot.parents.first().copied();
            }
            Some(result.map(|s| (id, s)))
        })
    }

    /// Reads every pack, index and object and reports what is damaged.
    pub fn verify(&self) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        for path in list_files(&self.dir(INDEX))? {
            if index::hash_from_file_name(&path, ".idx").is_some() {
                check(
                    &mut report.damaged_files,
                    path.clone(),
                    index::read_index(&path, &self.keys),
                )?;
            }
        }
        for shard in list_dirs(&self.dir(PACKS))? {
            for path in list_files(&shard)? {
                if let Some(name) = index::hash_from_file_name(&path, ".pack") {
                    let checked = pack::verify_file_hash(&path, &name)
                        .and_then(|()| pack::read_table(&path, &self.keys));
                    check(&mut report.damaged_files, path, checked)?;
                }
            }
        }
        let mut ids: Vec<_> = self.index.iter().map(|(id, l)| (*id, l.entry.ty)).collect();
        ids.sort_by_key(|(id, _)| *id);
        for (id, ty) in ids {
            let checked = match ty {
                ObjectType::Chunk => self.read_object(&id).map(drop),
                ObjectType::Blob => self.read_blob(&id).map(drop),
                ObjectType::Tree => self.read_tree(&id).map(drop),
                ObjectType::Snapshot => self.read_snapshot(&id).map(drop),
            };
            check(&mut report.damaged_objects, id, checked)?;
            report.objects_checked += 1;
        }
        Ok(report)
    }

    /// Starts a batch of writes. Objects become readable after [`ObjectWriter::finish`].
    pub fn writer(&mut self) -> ObjectWriter<'_> {
        ObjectWriter {
            repo: self,
            pack: None,
            pending: HashMap::new(),
            packs: Vec::new(),
            new_bytes: 0,
        }
    }

    pub fn read_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        self.refs.read(name)
    }

    /// Points `name` at `new` (or deletes it when `None`) if it currently
    /// points at `expected`. The target must be a stored snapshot.
    pub fn update_ref(
        &self,
        name: &str,
        expected: Option<ObjectId>,
        new: Option<ObjectId>,
    ) -> Result<()> {
        if let Some(target) = new {
            match self.index.get(&target) {
                None => return Err(Error::ObjectNotFound(target)),
                Some(l) if l.entry.ty != ObjectType::Snapshot => {
                    return Err(Error::NotASnapshot(target));
                }
                Some(_) => {}
            }
        }
        self.refs.update(name, expected, new)
    }

    /// All refs and their targets, sorted by name.
    pub fn list_refs(&self) -> Result<Vec<(String, ObjectId)>> {
        self.refs.list()
    }

    pub fn reflog(&self, name: &str) -> Result<Vec<ReflogEntry>> {
        self.refs.reflog(name)
    }

    /// Removes ref locks left behind more than 10 minutes ago, records ref
    /// changes whose reflog line was lost in a crash (FORMAT.md §9.3, §9.4)
    /// and indexes packs without a healthy index (§8.2).
    pub fn repair(&mut self) -> Result<RepairReport> {
        let mut report = self.refs.repair()?;
        let damaged_indexes: Vec<PathBuf> = self
            .damaged
            .iter()
            .filter(|p| p.starts_with(self.dir(INDEX)))
            .cloned()
            .collect();
        if self.unindexed.is_empty() && damaged_indexes.is_empty() {
            return Ok(report);
        }
        // The new index covers every pack the damaged ones covered before
        // they are deleted (FORMAT.md §8.2).
        if !self.unindexed.is_empty() {
            index::write_index(
                &self.dir(TMP),
                &self.dir(INDEX),
                &self.unindexed,
                &self.keys,
            )?;
            report.indexed_packs = self.unindexed.len() as u64;
        }
        for path in damaged_indexes {
            fsutil::checkpoint()
                .and_then(|()| fs::remove_file(&path))
                .at(&path)?;
            report.removed_index_files.push(path);
        }
        fsutil::sync_dir(&self.dir(INDEX))?;
        self.reload_index()?;
        Ok(report)
    }

    /// Index and pack files that failed verification when the index was
    /// loaded. Objects only in damaged packs cannot be read.
    pub fn damaged_files(&self) -> &[PathBuf] {
        &self.damaged
    }
}

/// Result of [`Repository::verify`].
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerifyReport {
    pub objects_checked: u64,
    /// Pack and index files whose hash or structure is wrong.
    pub damaged_files: Vec<PathBuf>,
    /// Objects that cannot be read back intact.
    pub damaged_objects: Vec<ObjectId>,
}

impl VerifyReport {
    pub fn is_ok(&self) -> bool {
        self.damaged_files.is_empty() && self.damaged_objects.is_empty()
    }
}

/// Records `item` as damaged if `result` is a corruption error.
fn check<T, U>(damaged: &mut Vec<T>, item: T, result: Result<U>) -> Result<()> {
    match result {
        Ok(_) => Ok(()),
        Err(Error::Corrupt { .. } | Error::UnsupportedVersion { .. }) => {
            damaged.push(item);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Writes objects into packs; see [`Repository::writer`].
pub struct ObjectWriter<'a> {
    repo: &'a mut Repository,
    pack: Option<PackWriter>,
    pending: HashMap<ObjectId, ObjectType>,
    packs: Vec<(FileHash, Vec<PackEntry>)>,
    new_bytes: u64,
}

impl ObjectWriter<'_> {
    /// Stores an object unless it already exists and returns its id.
    pub fn put(&mut self, ty: ObjectType, data: &[u8]) -> Result<ObjectId> {
        let id = self.repo.keys.id.object_id(ty, data);
        if self.repo.contains(&id) || self.pending.contains_key(&id) {
            return Ok(id);
        }
        let pack = match &mut self.pack {
            Some(pack) => pack,
            None => self
                .pack
                .insert(PackWriter::new(&self.repo.dir(TMP), &self.repo.keys)?),
        };
        pack.add(id, ty, data)?;
        self.pending.insert(id, ty);
        if ty == ObjectType::Chunk {
            self.new_bytes += data.len() as u64;
        }
        if pack.len() >= pack::TARGET_SIZE {
            self.finish_pack()?;
        }
        Ok(id)
    }

    /// Stores an object exactly as the pack at `from` stored it, after
    /// checking that the bytes decode to it.
    pub(crate) fn put_stored(
        &mut self,
        entry: &PackEntry,
        stored: &[u8],
        from: &Path,
    ) -> Result<()> {
        if self.repo.contains(&entry.id) || self.pending.contains_key(&entry.id) {
            return Ok(());
        }
        pack::decode_stored(from, entry, &self.repo.keys, stored)?;
        let pack = match &mut self.pack {
            Some(pack) => pack,
            None => self
                .pack
                .insert(PackWriter::new(&self.repo.dir(TMP), &self.repo.keys)?),
        };
        pack.add_stored(entry, stored)?;
        self.pending.insert(entry.id, entry.ty);
        if entry.ty == ObjectType::Chunk {
            self.new_bytes += u64::from(entry.raw_len);
        }
        if pack.len() >= pack::TARGET_SIZE {
            self.finish_pack()?;
        }
        Ok(())
    }

    /// Splits `reader` into chunks, stores them and returns each chunk's id and length.
    pub fn put_chunks<R: Read>(&mut self, reader: R) -> Result<Vec<(ObjectId, u32)>> {
        let mut chunks = Vec::new();
        for chunk in self.repo.config.chunker.chunks(reader) {
            let chunk = chunk?;
            // Chunks are at most `max_size` (16 MiB), so the length fits in a u32.
            let len = chunk.len() as u32;
            chunks.push((self.put(ObjectType::Chunk, &chunk)?, len));
        }
        Ok(chunks)
    }

    /// Bytes of file content (chunks) this batch stored that were not
    /// stored before.
    pub fn new_bytes(&self) -> u64 {
        self.new_bytes
    }

    /// Stores the content of a file and returns its size and blob id
    /// (`None` for an empty file, which has no blob).
    pub fn put_file<R: Read>(&mut self, reader: R) -> Result<(u64, Option<ObjectId>)> {
        let chunks = self.put_chunks(reader)?;
        let size = chunks.iter().map(|(_, len)| u64::from(*len)).sum();
        if chunks.is_empty() {
            return Ok((0, None));
        }
        let blob = Blob { size, chunks }.encode()?;
        Ok((size, Some(self.put(ObjectType::Blob, &blob)?)))
    }

    /// Stores a tree whose blobs and subtrees are already stored.
    pub fn put_tree(&mut self, tree: &Tree) -> Result<ObjectId> {
        let data = tree.encode()?;
        for entry in &tree.entries {
            match entry.kind {
                EntryKind::File { blob: Some(id), .. } => self.require(&id, ObjectType::Blob)?,
                EntryKind::Directory { tree } => self.require(&tree, ObjectType::Tree)?,
                EntryKind::File { blob: None, .. } | EntryKind::Symlink { .. } => {}
            }
        }
        self.put(ObjectType::Tree, &data)
    }

    /// Stores a snapshot whose tree and parents are already stored.
    pub fn put_snapshot(&mut self, snapshot: &Snapshot) -> Result<ObjectId> {
        let data = snapshot.encode()?;
        self.require(&snapshot.tree, ObjectType::Tree)?;
        for parent in &snapshot.parents {
            self.require(parent, ObjectType::Snapshot)?;
        }
        self.put(ObjectType::Snapshot, &data)
    }

    /// Checks that `id` is stored or written in this batch with type `ty`.
    fn require(&self, id: &ObjectId, ty: ObjectType) -> Result<()> {
        let found = match self.pending.get(id) {
            Some(found) => *found,
            None => match self.repo.index.get(id) {
                Some(location) => location.entry.ty,
                None => return Err(Error::ObjectNotFound(*id)),
            },
        };
        if found != ty {
            return Err(Error::WrongObjectType {
                id: *id,
                expected: ty,
                found,
            });
        }
        Ok(())
    }

    fn finish_pack(&mut self) -> Result<()> {
        if let Some(pack) = self.pack.take() {
            self.packs.push(pack.finish(&self.repo.dir(PACKS))?);
        }
        Ok(())
    }

    /// Makes all written objects durable and readable.
    pub fn finish(mut self) -> Result<()> {
        self.finish_pack()?;
        if self.packs.is_empty() {
            return Ok(());
        }
        index::write_index(
            &self.repo.dir(TMP),
            &self.repo.dir(INDEX),
            &self.packs,
            &self.repo.keys,
        )?;
        for (name, entries) in self.packs.drain(..) {
            for entry in entries {
                self.repo
                    .index
                    .entry(entry.id)
                    .or_insert(Location { pack: name, entry });
            }
        }
        Ok(())
    }
}

/// How [`Repository::open_with`] gets the keys of an encrypted repository.
enum Unlock<'a> {
    None,
    Secret(&'a str),
    Keys(Keys, Option<String>),
}

/// Creates the folders of a new repository in a new or empty folder.
fn create_layout(path: &Path, encrypted: bool) -> Result<()> {
    fsutil::ensure_dir(path)?;
    if fs::read_dir(path).at(path)?.next().is_some() {
        return Err(Error::InvalidRepository {
            path: path.to_path_buf(),
            reason: "directory is not empty".to_owned(),
        });
    }
    let keys = encrypted.then_some(KEYS);
    for dir in [PACKS, INDEX, REFS, LOGS, LOCKS, TMP]
        .into_iter()
        .chain(keys)
    {
        fsutil::ensure_dir(&path.join(dir))?;
    }
    Ok(())
}

/// Writes the config file of a new repository. It is written last: its
/// presence marks a complete repository.
fn write_config(path: &Path, config: &Config) -> Result<()> {
    fsutil::write_durable(
        &path.join(TMP),
        &path.join(CONFIG),
        config.to_toml().as_bytes(),
    )
}

fn list_entries(dir: &Path, want_dir: bool) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).at(dir)? {
        let entry = entry.at(dir)?;
        let file_type = entry.file_type().at(&entry.path())?;
        if file_type.is_dir() == want_dir {
            out.push(entry.path());
        }
    }
    Ok(out)
}

fn list_files(dir: &Path) -> Result<Vec<PathBuf>> {
    list_entries(dir, false)
}

fn list_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    list_entries(dir, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::tests::noise;
    use crate::object::{Time, TreeEntry};

    fn new_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        let params = ChunkerParams::new(64 * 1024).unwrap();
        let repo = Repository::init_with(&path, params).unwrap();
        (dir, repo)
    }

    fn put_snapshot(repo: &mut Repository, data: &[u8]) -> ObjectId {
        let mut writer = repo.writer();
        let id = writer.put(ObjectType::Snapshot, data).unwrap();
        writer.finish().unwrap();
        id
    }

    #[test]
    fn init_creates_layout_and_reopens() {
        let (_dir, repo) = new_repo();
        for dir in [PACKS, INDEX, REFS, LOGS, TMP] {
            assert!(repo.path().join(dir).is_dir());
        }
        let reopened = Repository::open(repo.path()).unwrap();
        assert_eq!(reopened.chunker(), repo.chunker());
        assert_eq!(reopened.keys.id, repo.keys.id);
    }

    #[test]
    fn encrypted_repository_needs_a_key_and_hides_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        let params = ChunkerParams::new(64 * 1024).unwrap();
        let (mut repo, recovery) = Repository::init_encrypted(&path, params, "pw").unwrap();
        assert!(repo.is_encrypted());
        let secret = b"top secret content ".repeat(50);
        let mut writer = repo.writer();
        let (_, blob) = writer.put_file(secret.as_slice()).unwrap();
        writer.finish().unwrap();
        let blob = blob.unwrap();

        for file in walk(&path) {
            let bytes = fs::read(&file).unwrap();
            assert!(!bytes.windows(10).any(|w| w == b"top secret"), "{file:?}");
            assert!(!bytes.windows(32).any(|w| w == blob.as_bytes()), "{file:?}");
        }

        assert!(matches!(
            Repository::open(&path),
            Err(Error::KeyRequired(_))
        ));
        assert!(matches!(
            Repository::unlock(&path, "wrong"),
            Err(Error::WrongKey)
        ));
        for secret_key in ["pw", recovery.as_str()] {
            let opened = Repository::unlock(&path, secret_key).unwrap();
            let mut out = Vec::new();
            opened.read_file(&blob, &mut out).unwrap();
            assert_eq!(out, secret);
            assert!(opened.verify().unwrap().is_ok());
        }
        let mut out = Vec::new();
        repo.reopen().unwrap().read_file(&blob, &mut out).unwrap();
        assert_eq!(out, secret);
    }

    #[test]
    fn keys_can_be_added_listed_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        let params = ChunkerParams::new(64 * 1024).unwrap();
        let (repo, _) = Repository::init_encrypted(&path, params, "pw").unwrap();
        let keys = repo.list_keys().unwrap();
        assert_eq!(keys.len(), 2);
        let first = keys.iter().find(|k| k.current).unwrap().id.clone();
        let first_recovery = keys.iter().find(|k| !k.current).unwrap().id.clone();
        assert_eq!(repo.reopen().unwrap().list_keys().unwrap(), keys);

        let second = repo.add_password("second").unwrap();
        let (third, recovery) = repo.add_recovery_key().unwrap();
        let opened = Repository::unlock(&path, &recovery).unwrap();
        let listed = opened.list_keys().unwrap();
        assert_eq!(listed.len(), 4);
        assert_eq!(
            listed
                .iter()
                .filter(|k| k.current)
                .map(|k| &k.id)
                .collect::<Vec<_>>(),
            [&third]
        );

        repo.remove_key(&first).unwrap();
        assert!(matches!(
            Repository::unlock(&path, "pw"),
            Err(Error::WrongKey)
        ));
        Repository::unlock(&path, "second").unwrap();
        let held = RepoLock::shared(&path).unwrap();
        assert!(matches!(
            repo.remove_key(&second),
            Err(Error::RepoLocked(_))
        ));
        drop(held);
        repo.remove_key(&second).unwrap();
        repo.remove_key(&first_recovery).unwrap();
        assert!(matches!(repo.remove_key(&third), Err(Error::LastKey)));

        let (_dir, plain) = new_repo();
        assert!(matches!(plain.list_keys(), Err(Error::NotEncrypted)));
        assert!(matches!(plain.add_password("x"), Err(Error::NotEncrypted)));
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = list_files(dir).unwrap();
        for sub in list_dirs(dir).unwrap() {
            out.extend(walk(&sub));
        }
        out
    }

    #[test]
    fn init_refuses_non_empty_dir_and_open_refuses_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file"), b"x").unwrap();
        assert!(matches!(
            Repository::init(dir.path()),
            Err(Error::InvalidRepository { .. })
        ));
        assert!(matches!(
            Repository::open(dir.path()),
            Err(Error::InvalidRepository { .. })
        ));
    }

    #[test]
    fn chunks_round_trip_and_deduplicate() {
        let (_dir, mut repo) = new_repo();
        let data = noise(1024 * 1024, 5);
        let mut writer = repo.writer();
        let first = writer.put_chunks(data.as_slice()).unwrap();
        let second = writer.put_chunks(data.as_slice()).unwrap();
        writer.finish().unwrap();
        assert_eq!(first, second);

        let mut rebuilt = Vec::new();
        for (id, len) in &first {
            let (ty, chunk) = repo.read_object(id).unwrap();
            assert_eq!(ty, ObjectType::Chunk);
            assert_eq!(chunk.len(), *len as usize);
            rebuilt.extend_from_slice(&chunk);
        }
        assert_eq!(rebuilt, data);
        assert_eq!(list_files(&repo.dir(INDEX)).unwrap().len(), 1);

        // Writing the same data again in a new batch adds no pack.
        let mut writer = repo.writer();
        writer.put_chunks(data.as_slice()).unwrap();
        writer.finish().unwrap();
        assert_eq!(list_files(&repo.dir(INDEX)).unwrap().len(), 1);
    }

    #[test]
    fn objects_are_invisible_until_finish() {
        let (_dir, mut repo) = new_repo();
        let mut writer = repo.writer();
        let id = writer.put(ObjectType::Chunk, b"pending").unwrap();
        drop(writer);
        assert!(!repo.contains(&id));
        assert_eq!(list_files(&repo.dir(TMP)).unwrap().len(), 0);
        assert!(
            Repository::open(repo.path())
                .unwrap()
                .read_object(&id)
                .is_err()
        );
    }

    #[test]
    fn objects_survive_reopen() {
        let (_dir, mut repo) = new_repo();
        let id = put_snapshot(&mut repo, b"state");
        let reopened = Repository::open(repo.path()).unwrap();
        assert_eq!(
            reopened.read_object(&id).unwrap(),
            (ObjectType::Snapshot, b"state".to_vec())
        );
    }

    #[test]
    fn unindexed_pack_is_found_on_open() {
        let (_dir, mut repo) = new_repo();
        let id = put_snapshot(&mut repo, b"state");
        for path in list_files(&repo.dir(INDEX)).unwrap() {
            fs::remove_file(path).unwrap();
        }
        let reopened = Repository::open(repo.path()).unwrap();
        assert_eq!(reopened.read_object(&id).unwrap().1, b"state");
    }

    #[test]
    fn corrupt_pack_is_reported_on_read() {
        let (_dir, mut repo) = new_repo();
        let data = noise(4096, 6);
        let mut writer = repo.writer();
        let id = writer.put(ObjectType::Chunk, &data).unwrap();
        writer.finish().unwrap();

        let shard = &list_dirs(&repo.dir(PACKS)).unwrap()[0];
        let pack_path = &list_files(shard).unwrap()[0];
        let mut bytes = fs::read(pack_path).unwrap();
        bytes[100] ^= 0xff;
        fs::write(pack_path, bytes).unwrap();

        assert!(matches!(repo.read_object(&id), Err(Error::Corrupt { .. })));
    }

    #[test]
    fn refs_point_only_at_stored_snapshots() {
        let (_dir, mut repo) = new_repo();
        let missing = ObjectId::from_bytes([9; 32]);
        assert!(matches!(
            repo.update_ref("main", None, Some(missing)),
            Err(Error::ObjectNotFound(_))
        ));

        let mut writer = repo.writer();
        let chunk = writer.put(ObjectType::Chunk, b"data").unwrap();
        writer.finish().unwrap();
        assert!(matches!(
            repo.update_ref("main", None, Some(chunk)),
            Err(Error::NotASnapshot(_))
        ));

        let snapshot = put_snapshot(&mut repo, b"snap");
        repo.update_ref("main", None, Some(snapshot)).unwrap();
        let reopened = Repository::open(repo.path()).unwrap();
        assert_eq!(reopened.read_ref("main").unwrap(), Some(snapshot));
        assert_eq!(reopened.reflog("main").unwrap().len(), 1);
    }

    #[test]
    fn stale_tmp_files_are_removed_on_open() {
        let (_dir, repo) = new_repo();
        let stale = repo.dir(TMP).join("stale");
        let fresh = repo.dir(TMP).join("fresh");
        fs::write(&stale, b"").unwrap();
        fs::write(&fresh, b"").unwrap();
        let old = SystemTime::now() - Duration::from_secs(2 * 3600);
        fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old)
            .unwrap();
        Repository::open(repo.path()).unwrap();
        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    fn entry(name: &str, kind: EntryKind) -> TreeEntry {
        TreeEntry {
            name: name.as_bytes().to_vec(),
            kind,
            mode: None,
            mtime: None,
            uid: None,
            gid: None,
            xattrs: vec![],
        }
    }

    fn snapshot(tree: ObjectId, parents: Vec<ObjectId>, message: &str) -> Snapshot {
        Snapshot {
            tree,
            parents,
            time: Time {
                seconds: 1_700_000_000,
                nanos: 0,
            },
            author: None,
            host: None,
            message: Some(message.to_owned()),
            meta: vec![],
        }
    }

    /// Stores an empty tree and a snapshot of it with the given parents.
    fn commit(repo: &mut Repository, parents: Vec<ObjectId>, message: &str) -> ObjectId {
        let mut writer = repo.writer();
        let tree = writer.put_tree(&Tree { entries: vec![] }).unwrap();
        let id = writer
            .put_snapshot(&snapshot(tree, parents, message))
            .unwrap();
        writer.finish().unwrap();
        id
    }

    #[test]
    fn files_trees_and_snapshots_round_trip() {
        let (_dir, mut repo) = new_repo();
        let data = noise(300 * 1024, 9);
        let mut writer = repo.writer();
        let (size, blob) = writer.put_file(data.as_slice()).unwrap();
        assert_eq!(size, data.len() as u64);
        assert_eq!(writer.put_file(&b""[..]).unwrap(), (0, None));
        let sub = writer.put_tree(&Tree { entries: vec![] }).unwrap();
        let tree = Tree {
            entries: vec![
                entry("data.bin", EntryKind::File { size, blob }),
                entry(
                    "empty",
                    EntryKind::File {
                        size: 0,
                        blob: None,
                    },
                ),
                entry("sub", EntryKind::Directory { tree: sub }),
            ],
        };
        let tree_id = writer.put_tree(&tree).unwrap();
        let snap = snapshot(tree_id, vec![], "first");
        let snap_id = writer.put_snapshot(&snap).unwrap();
        writer.finish().unwrap();

        let repo = Repository::open(repo.path()).unwrap();
        assert_eq!(repo.read_snapshot(&snap_id).unwrap(), snap);
        assert_eq!(repo.read_tree(&tree_id).unwrap(), tree);
        let mut out = Vec::new();
        let blob = blob.unwrap();
        assert_eq!(repo.read_file(&blob, &mut out).unwrap(), size);
        assert_eq!(out, data);
    }

    #[test]
    fn references_must_exist_with_the_right_type() {
        let (_dir, mut repo) = new_repo();
        let missing = ObjectId::from_bytes([9; 32]);
        let mut writer = repo.writer();
        let (size, blob) = writer.put_file(&b"content"[..]).unwrap();
        let blob = blob.unwrap();

        let file_to_missing = Tree {
            entries: vec![entry(
                "f",
                EntryKind::File {
                    size,
                    blob: Some(missing),
                },
            )],
        };
        assert!(matches!(
            writer.put_tree(&file_to_missing),
            Err(Error::ObjectNotFound(_))
        ));
        let dir_to_blob = Tree {
            entries: vec![entry("d", EntryKind::Directory { tree: blob })],
        };
        assert!(matches!(
            writer.put_tree(&dir_to_blob),
            Err(Error::WrongObjectType { .. })
        ));
        // A blob written earlier in the same batch counts as stored.
        let tree = writer
            .put_tree(&Tree {
                entries: vec![entry(
                    "f",
                    EntryKind::File {
                        size,
                        blob: Some(blob),
                    },
                )],
            })
            .unwrap();
        assert!(matches!(
            writer.put_snapshot(&snapshot(blob, vec![], "x")),
            Err(Error::WrongObjectType { .. })
        ));
        assert!(matches!(
            writer.put_snapshot(&snapshot(tree, vec![missing], "x")),
            Err(Error::ObjectNotFound(_))
        ));
        writer.put_snapshot(&snapshot(tree, vec![], "x")).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn typed_reads_check_type_and_encoding() {
        let (_dir, mut repo) = new_repo();
        let snap = commit(&mut repo, vec![], "s");
        assert!(matches!(
            repo.read_tree(&snap),
            Err(Error::WrongObjectType { .. })
        ));
        let mut writer = repo.writer();
        let junk = writer.put(ObjectType::Tree, b"junk").unwrap();
        writer.finish().unwrap();
        assert!(matches!(repo.read_tree(&junk), Err(Error::Corrupt { .. })));
    }

    #[test]
    fn history_follows_first_parents() {
        let (_dir, mut repo) = new_repo();
        let first = commit(&mut repo, vec![], "1");
        let side = commit(&mut repo, vec![], "side");
        let second = commit(&mut repo, vec![first], "2");
        let third = commit(&mut repo, vec![second, side], "3");
        let ids: Vec<_> = repo.history(third).map(|r| r.unwrap().0).collect();
        assert_eq!(ids, [third, second, first]);
    }

    #[test]
    fn list_refs_is_sorted_and_skips_locks() {
        let (_dir, mut repo) = new_repo();
        let a = commit(&mut repo, vec![], "a");
        let b = commit(&mut repo, vec![], "b");
        repo.update_ref("main", None, Some(a)).unwrap();
        repo.update_ref("backup/home", None, Some(b)).unwrap();
        fs::write(repo.dir(REFS).join("other.lock"), b"").unwrap();
        assert_eq!(
            repo.list_refs().unwrap(),
            [("backup/home".to_owned(), b), ("main".to_owned(), a)]
        );
    }

    #[test]
    fn verify_reports_damaged_files_and_objects() {
        let (_dir, mut repo) = new_repo();
        let data = noise(4096, 10);
        let mut writer = repo.writer();
        let (_, blob) = writer.put_file(data.as_slice()).unwrap();
        writer.finish().unwrap();
        let report = repo.verify().unwrap();
        assert!(report.is_ok());
        assert_eq!(report.objects_checked, 2);

        let shard = &list_dirs(&repo.dir(PACKS)).unwrap()[0];
        let pack_path = list_files(shard).unwrap().remove(0);
        let mut bytes = fs::read(&pack_path).unwrap();
        bytes[100] ^= 0xff;
        fs::write(&pack_path, bytes).unwrap();
        let index_path = list_files(&repo.dir(INDEX)).unwrap().remove(0);
        let mut bytes = fs::read(&index_path).unwrap();
        bytes[20] ^= 0xff;
        fs::write(&index_path, bytes).unwrap();

        let report = repo.verify().unwrap();
        assert!(!report.is_ok());
        let mut files = report.damaged_files.clone();
        files.sort();
        let mut expected = vec![index_path, pack_path];
        expected.sort();
        assert_eq!(files, expected);
        assert!(!report.damaged_objects.is_empty());
        assert!(matches!(
            repo.read_file(&blob.unwrap(), Vec::new()),
            Err(Error::Corrupt { .. })
        ));
    }
}
