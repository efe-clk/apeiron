//! A repository on local disk (FORMAT.md §2).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::chunker::ChunkerParams;
use crate::config::Config;
use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::{ObjectId, ObjectType};
use crate::index::{self, Location};
use crate::pack::{self, FileHash, PackEntry, PackWriter};
use crate::refs::{RefStore, ReflogEntry};

const CONFIG: &str = "config";
const PACKS: &str = "packs";
const INDEX: &str = "index";
const REFS: &str = "refs";
const LOGS: &str = "logs";
const TMP: &str = "tmp";
const TMP_MAX_AGE: Duration = Duration::from_secs(3600);

/// An open repository.
#[derive(Debug)]
pub struct Repository {
    root: PathBuf,
    config: Config,
    index: HashMap<ObjectId, Location>,
    refs: RefStore,
}

impl Repository {
    /// Creates a repository with default chunker parameters.
    /// `path` must not exist or be an empty directory.
    pub fn init(path: &Path) -> Result<Self> {
        Repository::init_with(path, ChunkerParams::default())
    }

    pub fn init_with(path: &Path, chunker: ChunkerParams) -> Result<Self> {
        fsutil::ensure_dir(path)?;
        if fs::read_dir(path).at(path)?.next().is_some() {
            return Err(Error::InvalidRepository {
                path: path.to_path_buf(),
                reason: "directory is not empty".to_owned(),
            });
        }
        for dir in [PACKS, INDEX, REFS, LOGS, TMP] {
            fsutil::ensure_dir(&path.join(dir))?;
        }
        // The config file is written last: its presence marks a complete repository.
        let config = Config::generate(chunker)?;
        fsutil::write_durable(
            &path.join(TMP),
            &path.join(CONFIG),
            config.to_toml().as_bytes(),
        )?;
        Repository::open(path)
    }

    pub fn open(path: &Path) -> Result<Self> {
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
        let mut repo = Repository {
            root: path.to_path_buf(),
            config,
            index: HashMap::new(),
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

    fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Loads all index files, then reads the tables of packs no index covers
    /// (left behind by a crash between writing a pack and its index).
    fn load_index(&mut self) -> Result<()> {
        let mut indexed: HashSet<FileHash> = HashSet::new();
        let index_dir = self.dir(INDEX);
        for path in list_files(&index_dir)? {
            if index::hash_from_file_name(&path, ".idx").is_none() {
                continue;
            }
            for location in index::read_index(&path)? {
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
                pack::verify_file_hash(&path, &name)?;
                for entry in pack::read_table(&path)? {
                    self.index
                        .entry(entry.id)
                        .or_insert(Location { pack: name, entry });
                }
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
        let data = pack::read_object(&path, &location.entry, &self.config.id_key)?;
        Ok((location.entry.ty, data))
    }

    /// Starts a batch of writes. Objects become readable after [`ObjectWriter::finish`].
    pub fn writer(&mut self) -> ObjectWriter<'_> {
        ObjectWriter {
            repo: self,
            pack: None,
            pending: HashSet::new(),
            packs: Vec::new(),
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

    pub fn reflog(&self, name: &str) -> Result<Vec<ReflogEntry>> {
        self.refs.reflog(name)
    }
}

/// Writes objects into packs; see [`Repository::writer`].
pub struct ObjectWriter<'a> {
    repo: &'a mut Repository,
    pack: Option<PackWriter>,
    pending: HashSet<ObjectId>,
    packs: Vec<(FileHash, Vec<PackEntry>)>,
}

impl ObjectWriter<'_> {
    /// Stores an object unless it already exists and returns its id.
    pub fn put(&mut self, ty: ObjectType, data: &[u8]) -> Result<ObjectId> {
        let id = self.repo.config.id_key.object_id(ty, data);
        if self.repo.contains(&id) || self.pending.contains(&id) {
            return Ok(id);
        }
        let pack = match &mut self.pack {
            Some(pack) => pack,
            None => self.pack.insert(PackWriter::new(&self.repo.dir(TMP))?),
        };
        pack.add(id, ty, data)?;
        self.pending.insert(id);
        if pack.len() >= pack::TARGET_SIZE {
            self.finish_pack()?;
        }
        Ok(id)
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
        index::write_index(&self.repo.dir(TMP), &self.repo.dir(INDEX), &self.packs)?;
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
        assert_eq!(reopened.config.id_key, repo.config.id_key);
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
}
