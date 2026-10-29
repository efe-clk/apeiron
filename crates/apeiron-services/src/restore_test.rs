//! The restore test: restoring a sample of a snapshot into a temporary
//! folder and checking it (DAEMON.md §5).

use std::collections::hash_map::RandomState;
use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use apeiron_core::{EntryKind, ObjectId, Repository};
use serde::{Deserialize, Serialize};

use crate::control::{Control, Phase};
use crate::error::{Error, IoContext, Result};
use crate::name::Name;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreTestOutcome {
    /// Files restored and checked.
    pub files: u64,
    pub bytes: u64,
    /// Files that could not be restored intact.
    pub problems: Vec<RestoreProblem>,
}

impl RestoreTestOutcome {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreProblem {
    pub path: Name,
    pub message: String,
}

/// Restores files of `snapshot`, chosen at random until `sample_bytes` are
/// reached (at least one file), into a new folder below `temp_dir`. Each
/// file is split into chunks again and must give the chunk ids and size the
/// snapshot records. The folder is removed afterwards.
pub fn restore_test(
    repo: &Repository,
    snapshot: &ObjectId,
    sample_bytes: u64,
    temp_dir: &Path,
    ctl: &Control,
) -> Result<RestoreTestOutcome> {
    ctl.set_phase(Phase::Scanning);
    let mut files = Vec::new();
    collect(
        repo,
        &repo.read_snapshot(snapshot)?.tree,
        &Name::default(),
        &mut files,
        ctl,
    )?;
    let mut random = random_seed();
    shuffle(&mut files, &mut random);
    let mut total = 0;
    let sample: Vec<_> = files
        .into_iter()
        .take_while(|(_, size, _)| {
            let take = total == 0 || total < sample_bytes;
            total += size;
            take
        })
        .collect();
    ctl.update(|p| {
        p.phase = Phase::Restoring;
        p.items_total = Some(sample.len() as u64);
        p.bytes_total = Some(sample.iter().map(|(_, size, _)| size).sum());
    });

    let dir = TempFolder::create(temp_dir, random)?;
    let mut out = RestoreTestOutcome::default();
    for (i, (name, size, blob)) in sample.into_iter().enumerate() {
        ctl.check()?;
        let path = dir.0.join(i.to_string());
        match check_file(repo, &blob, size, &path) {
            Ok(None) => {}
            Ok(Some(message)) => out.problems.push(RestoreProblem {
                path: name.clone(),
                message,
            }),
            Err(Error::Core(e)) => out.problems.push(RestoreProblem {
                path: name.clone(),
                message: e.to_string(),
            }),
            Err(e) => return Err(e),
        }
        let _ = fs::remove_file(&path);
        out.files += 1;
        out.bytes += size;
        ctl.update(|p| {
            p.items_done = out.files;
            p.bytes_done = out.bytes;
            p.current = Some(name);
        });
    }
    ctl.set_phase(Phase::Done);
    Ok(out)
}

/// Restores one file to `path` and checks it; returns what is wrong.
fn check_file(
    repo: &Repository,
    blob: &ObjectId,
    size: u64,
    path: &Path,
) -> Result<Option<String>> {
    let file = File::create(path).at(path)?;
    let written = repo.read_file(blob, BufWriter::new(file))?;
    let expected = repo.read_blob(blob)?;
    let file = File::open(path).at(path)?;
    let chunks = repo.chunk_ids(file)?;
    if written != size || fs::metadata(path).at(path)?.len() != size {
        return Ok(Some(format!("restored {written} bytes instead of {size}")));
    }
    if chunks != expected.chunks {
        return Ok(Some(
            "the restored content does not split into the stored chunks".to_owned(),
        ));
    }
    Ok(None)
}

/// Every file with content below `tree`: its path, size and blob.
fn collect(
    repo: &Repository,
    tree: &ObjectId,
    prefix: &Name,
    out: &mut Vec<(Name, u64, ObjectId)>,
    ctl: &Control,
) -> Result<()> {
    ctl.check()?;
    for entry in repo.read_tree(tree)?.entries {
        let name = prefix.join(&entry.name);
        match entry.kind {
            EntryKind::File {
                size,
                blob: Some(blob),
            } => out.push((name, size, blob)),
            EntryKind::Directory { tree } => collect(repo, &tree, &name, out, ctl)?,
            EntryKind::File { blob: None, .. } | EntryKind::Symlink { .. } => {}
        }
    }
    Ok(())
}

/// A random number from the standard library's per-process hash keys.
fn random_seed() -> u64 {
    RandomState::new().build_hasher().finish() | 1
}

/// Fisher–Yates with xorshift; good enough to pick a sample.
fn shuffle<T>(items: &mut [T], state: &mut u64) {
    for i in (1..items.len()).rev() {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        items.swap(i, (*state % (i as u64 + 1)) as usize);
    }
}

/// A new folder that is removed with everything in it when dropped.
struct TempFolder(PathBuf);

impl TempFolder {
    fn create(parent: &Path, random: u64) -> Result<Self> {
        let path = parent.join(format!("apeiron-restore-test-{random:016x}"));
        fs::create_dir(&path).at(&path)?;
        Ok(TempFolder(path))
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{BackupOptions, backup};

    #[test]
    fn a_sample_is_restored_checked_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        let big: Vec<u8> = (0..400_000u32).map(|i| (i * 7 % 251) as u8).collect();
        fs::write(src.join("big"), &big).unwrap();
        fs::write(src.join("sub/small"), b"small").unwrap();
        fs::write(src.join("empty"), b"").unwrap();
        let mut repo = Repository::init(&dir.path().join("repo")).unwrap();
        let ctl = Control::new();
        let snapshot = backup(&mut repo, &BackupOptions::new(&src, "main"), &ctl)
            .unwrap()
            .snapshot;
        let temp = dir.path().join("temp");
        fs::create_dir(&temp).unwrap();

        let all = restore_test(&repo, &snapshot, u64::MAX, &temp, &ctl).unwrap();
        assert!(all.is_ok(), "{all:?}");
        assert_eq!((all.files, all.bytes), (2, 400_005));
        let one = restore_test(&repo, &snapshot, 1, &temp, &ctl).unwrap();
        assert_eq!(one.files, 1);
        assert_eq!(fs::read_dir(&temp).unwrap().count(), 0);

        // Damage the first stored object, file content (pack header: 16
        // bytes): the test reports it instead of failing.
        let packs = dir.path().join("repo/packs");
        for shard in fs::read_dir(&packs).unwrap() {
            for pack in fs::read_dir(shard.unwrap().path()).unwrap() {
                let path = pack.unwrap().path();
                let mut bytes = fs::read(&path).unwrap();
                bytes[20] ^= 1;
                fs::write(&path, bytes).unwrap();
            }
        }
        let damaged = restore_test(&repo, &snapshot, u64::MAX, &temp, &ctl).unwrap();
        assert!(!damaged.is_ok());
        assert_eq!(fs::read_dir(&temp).unwrap().count(), 0);
    }
}
