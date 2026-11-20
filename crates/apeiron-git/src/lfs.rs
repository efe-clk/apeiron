//! Git LFS (GIT.md §2.3): a file kept in Git as a pointer is stored with
//! its content, marked with the attribute `git.lfs`, and exported as the
//! same pointer again, so commit ids do not change.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The tree entry attribute of a file that is an LFS pointer in Git.
pub(crate) const ATTRIBUTE: &[u8] = b"git.lfs";

const VERSION: &str = "version https://git-lfs.github.com/spec/v1\n";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The pointer Git LFS writes for `content`.
pub(crate) fn pointer(content: &[u8]) -> Vec<u8> {
    format!(
        "{VERSION}oid sha256:{}\nsize {}\n",
        hex(&Sha256::digest(content)),
        content.len()
    )
    .into_bytes()
}

/// The object id of a pointer, if `data` is one.
fn oid(data: &[u8]) -> Option<&str> {
    if data.len() > 1024 {
        return None;
    }
    let text = std::str::from_utf8(data).ok()?.strip_prefix(VERSION)?;
    text.lines()
        .find_map(|l| l.strip_prefix("oid sha256:"))
        .filter(|o| o.len() == 64 && o.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn object_path(git_dir: &Path, oid: &str) -> PathBuf {
    git_dir
        .join("lfs/objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid)
}

/// The content behind the pointer `data`, when it is in the repository's
/// LFS store and `data` is exactly the pointer Git LFS writes for it; any
/// other pointer is kept as it is.
pub(crate) fn content(git_dir: &Path, data: &[u8]) -> Option<Vec<u8>> {
    let content = fs::read(object_path(git_dir, oid(data)?)).ok()?;
    (pointer(&content) == data).then_some(content)
}

/// Puts `content` into the LFS store of `git_dir`, unless it is there.
pub(crate) fn store(git_dir: &Path, content: &[u8]) -> std::io::Result<()> {
    let path = object_path(git_dir, &hex(&Sha256::digest(content)));
    if path.exists() {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(git_dir);
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".apeiron-{}.tmp", std::process::id()));
    let mut file = fs::File::create(&tmp)?;
    file.write_all(content)?;
    file.sync_all()?;
    fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pointer_is_rebuilt_from_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"a large model".repeat(100);
        let p = pointer(&content);
        assert!(p.starts_with(VERSION.as_bytes()));
        assert_eq!(content_of(dir.path(), &p), None);
        store(dir.path(), &content).unwrap();
        assert_eq!(content_of(dir.path(), &p), Some(content.clone()));
        // A pointer with other keys stays a pointer.
        let mut other = p.clone();
        other.extend_from_slice(b"ext-0-foo sha256:00\n");
        assert_eq!(content_of(dir.path(), &other), None);
        assert_eq!(oid(b"hello"), None);
    }

    #[test]
    fn the_pointer_matches_git_lfs() {
        // echo -n hello | sha256sum
        let p = String::from_utf8(pointer(b"hello")).unwrap();
        assert_eq!(
            p,
            "version https://git-lfs.github.com/spec/v1\noid sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824\nsize 5\n"
        );
    }

    fn content_of(dir: &Path, data: &[u8]) -> Option<Vec<u8>> {
        content(dir, data)
    }
}
