//! The repository lock (FORMAT.md §12).

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::{Error, IoContext, Result};
use crate::fsutil;

const LOCKS: &str = "locks";
const LOCK_FILE: &str = "repository";

/// A held repository lock; released when dropped, or by the operating
/// system when the program ends.
#[derive(Debug)]
pub struct RepoLock {
    _file: File,
}

impl RepoLock {
    /// Takes the lock shared, as every reader and writer does.
    pub fn shared(repo: &Path) -> Result<Self> {
        RepoLock::take(repo, false)
    }

    /// Takes the lock exclusive, as garbage collection does.
    pub fn exclusive(repo: &Path) -> Result<Self> {
        RepoLock::take(repo, true)
    }

    fn take(repo: &Path, exclusive: bool) -> Result<Self> {
        let dir = repo.join(LOCKS);
        fsutil::ensure_dir(&dir)?;
        let path = dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .at(&path)?;
        // Called through the trait: newer standard libraries have methods
        // with the same names but different error types.
        let taken = if exclusive {
            fs4::FileExt::try_lock(&file)
        } else {
            fs4::FileExt::try_lock_shared(&file)
        };
        match taken {
            Ok(()) => Ok(RepoLock { _file: file }),
            Err(fs4::TryLockError::WouldBlock) => Err(Error::RepoLocked(lock_path(repo))),
            Err(fs4::TryLockError::Error(e)) => Err(e).at(&path),
        }
    }
}

fn lock_path(repo: &Path) -> PathBuf {
    repo.join(LOCKS).join(LOCK_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_locks_coexist_and_exclude_an_exclusive_one() {
        let dir = tempfile::tempdir().unwrap();
        let a = RepoLock::shared(dir.path()).unwrap();
        let b = RepoLock::shared(dir.path()).unwrap();
        assert!(matches!(
            RepoLock::exclusive(dir.path()),
            Err(Error::RepoLocked(_))
        ));
        drop((a, b));
        let gc = RepoLock::exclusive(dir.path()).unwrap();
        assert!(matches!(
            RepoLock::shared(dir.path()),
            Err(Error::RepoLocked(_))
        ));
        assert!(matches!(
            RepoLock::exclusive(dir.path()),
            Err(Error::RepoLocked(_))
        ));
        drop(gc);
        RepoLock::shared(dir.path()).unwrap();
    }
}
