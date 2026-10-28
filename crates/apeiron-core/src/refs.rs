//! Refs and reflogs (FORMAT.md §9.3, §9.4).

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::{ObjectId, to_hex};

const MAX_NAME_LEN: usize = 200;
const LOCK_SUFFIX: &str = ".lock";
/// Locks older than this are considered stale (FORMAT.md §9.3).
const LOCK_STALE_AGE: Duration = Duration::from_secs(600);
const NO_ID: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Checks a ref name against FORMAT.md §9.3.
pub fn validate_ref_name(name: &str) -> Result<()> {
    let invalid = || Error::InvalidRefName(name.to_owned());
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(invalid());
    }
    for part in name.split('/') {
        let allowed = |c: char| matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '-');
        if part.is_empty()
            || part.starts_with('.')
            || part.ends_with(LOCK_SUFFIX)
            || !part.chars().all(allowed)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

/// What [`crate::Repository::repair`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RepairReport {
    /// Stale ref lock files that were removed.
    pub removed_locks: Vec<PathBuf>,
    /// Refs whose reflog got a `repair` line.
    pub repaired_refs: Vec<String>,
}

/// Kind of change recorded in a reflog line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefOp {
    Create,
    Update,
    Delete,
    Repair,
}

impl RefOp {
    fn as_str(self) -> &'static str {
        match self {
            RefOp::Create => "create",
            RefOp::Update => "update",
            RefOp::Delete => "delete",
            RefOp::Repair => "repair",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "create" => Some(RefOp::Create),
            "update" => Some(RefOp::Update),
            "delete" => Some(RefOp::Delete),
            "repair" => Some(RefOp::Repair),
            _ => None,
        }
    }
}

/// One line of a reflog.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReflogEntry {
    pub old: Option<ObjectId>,
    pub new: Option<ObjectId>,
    /// Unix time in seconds.
    pub time: u64,
    pub op: RefOp,
}

impl ReflogEntry {
    fn encode(&self) -> String {
        let id = |v: Option<ObjectId>| v.map_or_else(|| NO_ID.to_owned(), |id| id.to_hex());
        let body = format!(
            "{} {} {} {}",
            id(self.old),
            id(self.new),
            self.time,
            self.op.as_str()
        );
        let check = short_check(&body);
        format!("{body} {check}\n")
    }

    fn decode(line: &str) -> Option<Self> {
        let (body, check) = line.rsplit_once(' ')?;
        if short_check(body) != check {
            return None;
        }
        let mut fields = body.split(' ');
        let mut id = || -> Option<Option<ObjectId>> {
            let field = fields.next()?;
            if field == NO_ID {
                Some(None)
            } else {
                ObjectId::from_hex(field).map(Some)
            }
        };
        let old = id()?;
        let new = id()?;
        let time = fields.next()?.parse().ok()?;
        let op = RefOp::parse(fields.next()?)?;
        if fields.next().is_some() {
            return None;
        }
        Some(ReflogEntry { old, new, time, op })
    }
}

fn short_check(body: &str) -> String {
    to_hex(&blake3::hash(body.as_bytes()).as_bytes()[..8])
}

fn encode_ref(target: ObjectId) -> String {
    let head = format!("apeiron-ref 1\ntarget {target}\n");
    let check = blake3::hash(head.as_bytes());
    format!("{head}check {}\n", to_hex(check.as_bytes()))
}

fn decode_ref(path: &Path, text: &str) -> Result<ObjectId> {
    let corrupt = || Error::corrupt(path, "invalid ref file");
    let mut lines = text.split_inclusive('\n');
    let (Some(magic), Some(target), Some(check), None) =
        (lines.next(), lines.next(), lines.next(), lines.next())
    else {
        return Err(corrupt());
    };
    if magic != "apeiron-ref 1\n" {
        return Err(corrupt());
    }
    let expected = to_hex(blake3::hash(format!("{magic}{target}").as_bytes()).as_bytes());
    if check != format!("check {expected}\n") {
        return Err(Error::corrupt(path, "ref checksum mismatch"));
    }
    target
        .strip_prefix("target ")
        .and_then(|t| t.strip_suffix('\n'))
        .and_then(ObjectId::from_hex)
        .ok_or_else(corrupt)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Ref and reflog files of one repository.
#[derive(Clone, Debug)]
pub(crate) struct RefStore {
    refs_dir: PathBuf,
    logs_dir: PathBuf,
    tmp_dir: PathBuf,
}

impl RefStore {
    pub(crate) fn new(refs_dir: PathBuf, logs_dir: PathBuf, tmp_dir: PathBuf) -> Self {
        RefStore {
            refs_dir,
            logs_dir,
            tmp_dir,
        }
    }

    pub(crate) fn read(&self, name: &str) -> Result<Option<ObjectId>> {
        validate_ref_name(name)?;
        self.read_path(&self.refs_dir.join(name))
    }

    fn read_path(&self, path: &Path) -> Result<Option<ObjectId>> {
        match fs::metadata(path) {
            Ok(meta) if meta.is_dir() => return Ok(None),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            // A parent component is a file: `name` lies below another ref.
            Err(e) if e.kind() == io::ErrorKind::NotADirectory => return Ok(None),
            Err(e) => return Err(e).at(path),
        }
        let text = fs::read_to_string(path).at(path)?;
        decode_ref(path, &text).map(Some)
    }

    /// Compare-and-swap: sets `name` to `new` (or deletes it when `None`)
    /// only if it currently points to `expected`.
    pub(crate) fn update(
        &self,
        name: &str,
        expected: Option<ObjectId>,
        new: Option<ObjectId>,
    ) -> Result<()> {
        validate_ref_name(name)?;
        let path = self.refs_dir.join(name);
        if new.is_some() {
            self.check_name_free(name)?;
        }
        let parent = fsutil::parent_of(&path);
        fsutil::ensure_dir(parent)?;

        let lock = Lock::acquire(name, path.with_file_name(lock_file_name(&path)))?;
        let actual = self.read_path(&path)?;
        if actual != expected {
            return Err(Error::RefConflict {
                name: name.to_owned(),
                expected,
                actual,
            });
        }
        self.repair_log(name, actual)?;
        let op = match (actual, new) {
            (None, None) => return Ok(()),
            (None, Some(_)) => RefOp::Create,
            (Some(_), Some(_)) => RefOp::Update,
            (Some(_), None) => RefOp::Delete,
        };
        match new {
            Some(target) => lock.commit(&path, &encode_ref(target))?,
            None => {
                let graveyard = self.tmp_dir.join(fsutil::random_name()?);
                fsutil::checkpoint()
                    .and_then(|()| apeiron_sys::rename(&path, &graveyard))
                    .at(&path)?;
                fsutil::sync_dir(parent)?;
                drop(lock);
            }
        }
        self.append_log(
            name,
            &ReflogEntry {
                old: actual,
                new,
                time: now(),
                op,
            },
        )
    }

    /// A ref cannot share its name with a directory of other refs, or sit below
    /// another ref. Reflogs of deleted refs keep reserving their names.
    fn check_name_free(&self, name: &str) -> Result<()> {
        let conflict = || Error::RefNameConflict(name.to_owned());
        let parts: Vec<&str> = name.split('/').collect();
        for root in [&self.refs_dir, &self.logs_dir] {
            let mut prefix = root.clone();
            for part in &parts[..parts.len() - 1] {
                prefix.push(part);
                if prefix.is_file() {
                    return Err(conflict());
                }
            }
            let path = root.join(name);
            if path.is_dir() && fs::remove_dir(&path).is_err() {
                return Err(conflict());
            }
        }
        Ok(())
    }

    /// All refs and their targets, sorted by name.
    pub(crate) fn list(&self) -> Result<Vec<(String, ObjectId)>> {
        let mut refs = Vec::new();
        for (path, name) in walk(&self.refs_dir)? {
            if name.ends_with(LOCK_SUFFIX) || validate_ref_name(&name).is_err() {
                continue;
            }
            if let Some(target) = self.read_path(&path)? {
                refs.push((name, target));
            }
        }
        refs.sort();
        Ok(refs)
    }

    /// Removes stale locks and records ref changes whose reflog line was lost.
    pub(crate) fn repair(&self) -> Result<RepairReport> {
        let mut removed = Vec::new();
        let mut repaired = Vec::new();
        let mut names = BTreeSet::new();
        for (path, name) in walk(&self.refs_dir)? {
            if !name.ends_with(LOCK_SUFFIX) {
                names.insert(name);
            } else if is_stale(&path)? {
                fsutil::checkpoint()
                    .and_then(|()| fs::remove_file(&path))
                    .at(&path)?;
                fsutil::sync_dir(fsutil::parent_of(&path))?;
                removed.push(path);
            }
        }
        // Deleted refs have a reflog but no ref file.
        names.extend(walk(&self.logs_dir)?.into_iter().map(|(_, name)| name));

        for name in names {
            if validate_ref_name(&name).is_err() {
                continue;
            }
            let path = self.refs_dir.join(&name);
            fsutil::ensure_dir(fsutil::parent_of(&path))?;
            let _lock = match Lock::acquire(&name, path.with_file_name(lock_file_name(&path))) {
                Ok(lock) => lock,
                // Another writer is active; it repairs the reflog itself.
                Err(Error::RefLocked { .. }) => continue,
                Err(e) => return Err(e),
            };
            let actual = self.read_path(&path)?;
            if self.repair_log(&name, actual)? {
                repaired.push(name);
            }
        }
        Ok(RepairReport {
            removed_locks: removed,
            repaired_refs: repaired,
        })
    }

    pub(crate) fn reflog(&self, name: &str) -> Result<Vec<ReflogEntry>> {
        validate_ref_name(name)?;
        let path = self.logs_dir.join(name);
        Ok(read_log(&path)?.0)
    }

    /// Records a ref change whose reflog line was lost (FORMAT.md §9.4).
    /// Returns whether a line was added.
    fn repair_log(&self, name: &str, actual: Option<ObjectId>) -> Result<bool> {
        let path = self.logs_dir.join(name);
        let last = read_log(&path)?.0.last().and_then(|e| e.new);
        if last == actual {
            return Ok(false);
        }
        self.append_log(
            name,
            &ReflogEntry {
                old: last,
                new: actual,
                time: now(),
                op: RefOp::Repair,
            },
        )?;
        Ok(true)
    }

    fn append_log(&self, name: &str, entry: &ReflogEntry) -> Result<()> {
        let path = self.logs_dir.join(name);
        let parent = fsutil::parent_of(&path);
        fsutil::ensure_dir(parent)?;
        let (_, valid_len, existed) = read_log(&path)?;
        let mut file = fsutil::checkpoint()
            .and_then(|()| {
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(false)
                    .open(&path)
            })
            .at(&path)?;
        // Drop a torn last line left by a crash before appending.
        fsutil::checkpoint()
            .and_then(|()| file.set_len(valid_len))
            .and_then(|()| fsutil::checkpoint())
            .and_then(|()| file.sync_all())
            .at(&path)?;
        append(&mut file, &path, valid_len, entry.encode().as_bytes())?;
        if !existed {
            fsutil::sync_dir(parent)?;
        }
        Ok(())
    }
}

fn append(file: &mut File, path: &Path, offset: u64, bytes: &[u8]) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset)).at(path)?;
    fsutil::checkpoint()
        .and_then(|()| file.write_all(bytes))
        .and_then(|()| fsutil::checkpoint())
        .and_then(|()| file.sync_all())
        .at(path)
}

/// Lists the files below `root` with their `/`-separated relative names.
fn walk(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut out = Vec::new();
    let mut dirs = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = dirs.pop() {
        for entry in fs::read_dir(&dir).at(&dir)? {
            let entry = entry.at(&dir)?;
            let path = entry.path();
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.file_type().at(&path)?.is_dir() {
                dirs.push((path, format!("{name}/")));
            } else {
                out.push((path, name));
            }
        }
    }
    Ok(out)
}

fn is_stale(path: &Path) -> Result<bool> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).at(path)?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age > LOCK_STALE_AGE))
}

/// Returns the valid entries, the byte length they occupy and whether the file exists.
fn read_log(path: &Path) -> Result<(Vec<ReflogEntry>, u64, bool)> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), 0, false)),
        Err(e) => return Err(e).at(path),
    };
    let mut entries = Vec::new();
    let mut valid_len = 0usize;
    let lines: Vec<&[u8]> = bytes.split_inclusive(|&b| b == b'\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let entry = line
            .strip_suffix(b"\n")
            .and_then(|l| std::str::from_utf8(l).ok())
            .and_then(ReflogEntry::decode);
        match entry {
            Some(entry) => {
                entries.push(entry);
                valid_len += line.len();
            }
            // A crash can only damage the last line.
            None if i + 1 == lines.len() => break,
            None => {
                return Err(Error::corrupt(
                    path,
                    format!("invalid reflog line {}", i + 1),
                ));
            }
        }
    }
    Ok((entries, valid_len as u64, true))
}

fn lock_file_name(path: &Path) -> String {
    let base = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{base}{LOCK_SUFFIX}")
}

/// An exclusively created `.lock` file, removed on drop unless committed.
struct Lock {
    path: PathBuf,
    file: Option<File>,
}

impl Lock {
    fn acquire(name: &str, path: PathBuf) -> Result<Self> {
        let opened = fsutil::checkpoint()
            .and_then(|()| OpenOptions::new().write(true).create_new(true).open(&path));
        match opened {
            Ok(file) => Ok(Lock {
                path,
                file: Some(file),
            }),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(Error::RefLocked {
                name: name.to_owned(),
                lock: path,
            }),
            Err(e) => Err(e).at(&path),
        }
    }

    /// Writes `content` into the lock file and renames it over `dest`.
    fn commit(mut self, dest: &Path, content: &str) -> Result<()> {
        if let Some(mut file) = self.file.take() {
            fsutil::checkpoint()
                .and_then(|()| file.write_all(content.as_bytes()))
                .and_then(|()| fsutil::checkpoint())
                .and_then(|()| file.sync_all())
                .at(&self.path)?;
        }
        fsutil::persist(&self.path, dest)?;
        // The lock path no longer exists; nothing to clean up.
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.file.take();
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, RefStore) {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("tmp");
        fs::create_dir(&tmp).unwrap();
        let store = RefStore::new(dir.path().join("refs"), dir.path().join("logs"), tmp);
        (dir, store)
    }

    fn id(byte: u8) -> ObjectId {
        ObjectId::from_bytes([byte; 32])
    }

    #[test]
    fn name_rules() {
        for ok in ["main", "backup/home", "tag/v1.0", "a_b-c"] {
            validate_ref_name(ok).unwrap();
        }
        for bad in [
            "", "Main", "a//b", "/a", "a/", ".hidden", "a/.b", "x.lock", "a/b.lock", "ğ", "a b",
        ] {
            assert!(validate_ref_name(bad).is_err(), "{bad:?}");
        }
        assert!(validate_ref_name(&"a".repeat(201)).is_err());
    }

    #[test]
    fn ref_file_format_matches_spec() {
        let text = encode_ref(id(0xab));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "apeiron-ref 1");
        assert_eq!(lines[1], format!("target {}", "ab".repeat(32)));
        let head = format!("{}\n{}\n", lines[0], lines[1]);
        assert_eq!(
            lines[2],
            format!("check {}", blake3::hash(head.as_bytes()).to_hex())
        );
        assert_eq!(decode_ref(Path::new("r"), &text).unwrap(), id(0xab));
        let tampered = text.replace("target ab", "target ac");
        assert!(decode_ref(Path::new("r"), &tampered).is_err());
    }

    #[test]
    fn create_update_delete_with_reflog() {
        let (_dir, store) = store();
        assert_eq!(store.read("backup/home").unwrap(), None);
        store.update("backup/home", None, Some(id(1))).unwrap();
        store
            .update("backup/home", Some(id(1)), Some(id(2)))
            .unwrap();
        assert_eq!(store.read("backup/home").unwrap(), Some(id(2)));
        store.update("backup/home", Some(id(2)), None).unwrap();
        assert_eq!(store.read("backup/home").unwrap(), None);

        let log = store.reflog("backup/home").unwrap();
        let summary: Vec<_> = log.iter().map(|e| (e.old, e.new, e.op)).collect();
        assert_eq!(
            summary,
            [
                (None, Some(id(1)), RefOp::Create),
                (Some(id(1)), Some(id(2)), RefOp::Update),
                (Some(id(2)), None, RefOp::Delete),
            ]
        );
    }

    #[test]
    fn compare_and_swap_rejects_stale_expectation() {
        let (_dir, store) = store();
        store.update("main", None, Some(id(1))).unwrap();
        let err = store.update("main", None, Some(id(2))).unwrap_err();
        assert!(matches!(err, Error::RefConflict { actual: Some(a), .. } if a == id(1)));
        assert_eq!(store.read("main").unwrap(), Some(id(1)));
        assert!(!store.refs_dir.join("main.lock").exists());
    }

    #[test]
    fn existing_lock_blocks_update() {
        let (_dir, store) = store();
        store.update("main", None, Some(id(1))).unwrap();
        fs::write(store.refs_dir.join("main.lock"), b"").unwrap();
        let err = store.update("main", Some(id(1)), Some(id(2))).unwrap_err();
        assert!(matches!(err, Error::RefLocked { .. }));
        assert_eq!(store.read("main").unwrap(), Some(id(1)));
        assert!(store.refs_dir.join("main.lock").exists());
    }

    #[test]
    fn names_cannot_nest() {
        let (_dir, store) = store();
        store.update("backup/home", None, Some(id(1))).unwrap();
        assert!(matches!(
            store.update("backup", None, Some(id(2))),
            Err(Error::RefNameConflict(_))
        ));
        assert!(matches!(
            store.update("backup/home/x", None, Some(id(2))),
            Err(Error::RefNameConflict(_))
        ));
        // The deleted ref's reflog keeps the name structure reserved.
        store.update("backup/home", Some(id(1)), None).unwrap();
        assert!(matches!(
            store.update("backup", None, Some(id(3))),
            Err(Error::RefNameConflict(_))
        ));
        store.update("backup/home", None, Some(id(3))).unwrap();
    }

    #[test]
    fn torn_reflog_line_is_ignored_and_truncated() {
        let (_dir, store) = store();
        store.update("main", None, Some(id(1))).unwrap();
        let log_path = store.logs_dir.join("main");
        let mut file = OpenOptions::new().append(true).open(&log_path).unwrap();
        file.write_all(b"0000 half a li").unwrap();
        drop(file);
        assert_eq!(store.reflog("main").unwrap().len(), 1);

        store.update("main", Some(id(1)), Some(id(2))).unwrap();
        let log = store.reflog("main").unwrap();
        assert_eq!(log.len(), 2);
        assert!(!fs::read_to_string(&log_path).unwrap().contains("half"));
    }

    #[test]
    fn corrupt_middle_reflog_line_is_an_error() {
        let (_dir, store) = store();
        store.update("main", None, Some(id(1))).unwrap();
        store.update("main", Some(id(1)), Some(id(2))).unwrap();
        let log_path = store.logs_dir.join("main");
        let text = fs::read_to_string(&log_path).unwrap();
        fs::write(&log_path, text.replacen("create", "creatf", 1)).unwrap();
        assert!(matches!(store.reflog("main"), Err(Error::Corrupt { .. })));
    }

    #[test]
    fn missing_reflog_line_is_repaired_on_next_update() {
        let (_dir, store) = store();
        store.update("main", None, Some(id(1))).unwrap();
        // Simulate a crash after the ref rename but before the reflog append.
        let ref_path = store.refs_dir.join("main");
        fs::write(&ref_path, encode_ref(id(2))).unwrap();
        store.update("main", Some(id(2)), Some(id(3))).unwrap();

        let ops: Vec<_> = store
            .reflog("main")
            .unwrap()
            .iter()
            .map(|e| (e.old, e.new, e.op))
            .collect();
        assert_eq!(
            ops,
            [
                (None, Some(id(1)), RefOp::Create),
                (Some(id(1)), Some(id(2)), RefOp::Repair),
                (Some(id(2)), Some(id(3)), RefOp::Update),
            ]
        );
    }
}
