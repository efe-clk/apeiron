//! Durable file writes (FORMAT.md §9.1).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::error::{IoContext, Result};
use crate::id::{random_bytes, to_hex};

pub(crate) fn random_name() -> Result<String> {
    Ok(to_hex(&random_bytes::<16>()?))
}

/// Creates a new, uniquely named file in `tmp_dir`.
pub(crate) fn create_tmp(tmp_dir: &Path) -> Result<(File, PathBuf)> {
    let path = tmp_dir.join(random_name()?);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .at(&path)?;
    Ok((file, path))
}

/// Flushes a directory entry change (create, rename, delete) to disk.
#[cfg(not(windows))]
pub(crate) fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir).and_then(|d| d.sync_all()).at(dir)
}

/// Windows cannot open directories for flushing; NTFS journals the rename itself.
#[cfg(windows)]
pub(crate) fn sync_dir(_dir: &Path) -> Result<()> {
    Ok(())
}

/// Creates `dir` and any missing parents, syncing each parent that gains an entry.
pub(crate) fn ensure_dir(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let parent = dir.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        ensure_dir(parent)?;
    }
    match fs::create_dir(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && dir.is_dir() => return Ok(()),
        Err(e) => return Err(e).at(dir),
    }
    match parent {
        Some(parent) => sync_dir(parent),
        None => Ok(()),
    }
}

/// Moves a fully synced temporary file to `dest` and syncs the directory.
pub(crate) fn persist(tmp_path: &Path, dest: &Path) -> Result<()> {
    let parent = parent_of(dest);
    ensure_dir(parent)?;
    if let Err(e) = fs::rename(tmp_path, dest).at(dest) {
        let _ = fs::remove_file(tmp_path);
        return Err(e);
    }
    sync_dir(parent)
}

/// Like [`persist`] for content-addressed files: an existing `dest` already has
/// the same content, so the temporary file is discarded.
pub(crate) fn persist_new(tmp_path: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        return fs::remove_file(tmp_path).at(tmp_path);
    }
    persist(tmp_path, dest)
}

/// Writes `bytes` to `dest` atomically and durably via `tmp_dir`.
pub(crate) fn write_durable(tmp_dir: &Path, dest: &Path, bytes: &[u8]) -> Result<()> {
    persist(&write_synced_tmp(tmp_dir, bytes)?, dest)
}

/// Like [`write_durable`] for content-addressed files (see [`persist_new`]).
pub(crate) fn write_durable_new(tmp_dir: &Path, dest: &Path, bytes: &[u8]) -> Result<()> {
    persist_new(&write_synced_tmp(tmp_dir, bytes)?, dest)
}

fn write_synced_tmp(tmp_dir: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let (mut file, tmp_path) = create_tmp(tmp_dir)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .at(&tmp_path);
    drop(file);
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(tmp_path)
}

pub(crate) fn parent_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_durable_creates_parents_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("tmp");
        fs::create_dir(&tmp).unwrap();
        let dest = dir.path().join("a/b/c");
        write_durable(&tmp, &dest, b"data").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"data");
        write_durable(&tmp, &dest, b"new").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert_eq!(fs::read_dir(&tmp).unwrap().count(), 0);
    }

    #[test]
    fn persist_new_keeps_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        fs::write(&dest, b"old").unwrap();
        let (mut file, tmp_path) = create_tmp(dir.path()).unwrap();
        file.write_all(b"new").unwrap();
        drop(file);
        persist_new(&tmp_path, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert!(!tmp_path.exists());
    }
}
