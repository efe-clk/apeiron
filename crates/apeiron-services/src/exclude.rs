//! What a backup or a sync leaves out (CLI.md §4.1, SYNC.md §5.1).

use std::fs;
use std::io;
use std::path::Path;

use crate::error::{Error, IoContext, Result};
use crate::name::Name;
use crate::search::wildcard;

/// The file of a folder that lists what to leave out of it.
pub const IGNORE_FILE: &str = ".apeironignore";

/// Patterns, each matched against an entry's name or, with a `/`, against its
/// whole path from the root of the folder; ignoring case.
#[derive(Debug, Default)]
pub(crate) struct Exclude {
    patterns: Vec<(bool, Vec<char>)>,
}

impl Exclude {
    /// The patterns given and those of the folder's `.apeironignore`.
    pub(crate) fn for_folder(folder: &Path, given: &[String]) -> Result<Self> {
        let mut exclude = Exclude::default();
        for pattern in given {
            if !exclude.add(pattern) {
                return Err(Error::InvalidInput(format!(
                    "{pattern:?} is not a pattern to leave out"
                )));
            }
        }
        let path = folder.join(IGNORE_FILE);
        match fs::read(&path) {
            Ok(bytes) => {
                for line in String::from_utf8_lossy(&bytes).lines() {
                    let line = line.trim();
                    if !line.starts_with('#') {
                        exclude.add(line);
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            Err(e) => return Err(e).at(&path),
        }
        Ok(exclude)
    }

    /// Adds a pattern; `false` if it matches nothing.
    fn add(&mut self, pattern: &str) -> bool {
        let pattern = pattern.trim_end_matches('/');
        let whole = pattern.contains('/');
        let pattern = pattern.trim_start_matches('/');
        if pattern.is_empty() {
            return false;
        }
        self.patterns
            .push((whole, pattern.to_lowercase().chars().collect()));
        true
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Whether the entry `name` at the path `path` is left out.
    pub(crate) fn matches(&self, path: &Name, name: &[u8]) -> bool {
        if self.is_empty() {
            return false;
        }
        let lower = |bytes: &[u8]| -> Vec<char> {
            String::from_utf8_lossy(bytes)
                .to_lowercase()
                .chars()
                .collect()
        };
        let (path, name) = (lower(path.as_bytes()), lower(name));
        self.patterns
            .iter()
            .any(|(whole, pattern)| wildcard(pattern, if *whole { &path } else { &name }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exclude(patterns: &[&str]) -> Exclude {
        let mut e = Exclude::default();
        for p in patterns {
            assert!(e.add(p));
        }
        e
    }

    #[test]
    fn names_match_at_any_depth_and_paths_from_the_root() {
        let e = exclude(&["*.TMP", "build/", "docs/old", "/top.txt"]);
        let m = |path: &str| {
            let name = path.rsplit('/').next().unwrap();
            e.matches(&Name::from(path), name.as_bytes())
        };
        assert!(m("a.tmp"));
        assert!(m("deep/in/a.tmp"));
        assert!(m("build"));
        assert!(m("deep/build"));
        assert!(m("docs/old"));
        assert!(!m("x/docs/old"));
        assert!(m("top.txt"));
        assert!(!m("deep/top.txt"));
        assert!(!m("a.txt"));
    }

    #[test]
    fn an_empty_pattern_is_refused() {
        assert!(!Exclude::default().add("/"));
        assert!(!Exclude::default().add(""));
        let dir = tempfile::tempdir().unwrap();
        assert!(Exclude::for_folder(dir.path(), &[String::new()]).is_err());
    }

    #[test]
    fn the_file_of_the_folder_counts_without_its_comments() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(IGNORE_FILE), "# a comment\n\n*.log\n").unwrap();
        let e = Exclude::for_folder(dir.path(), &[]).unwrap();
        assert!(e.matches(&Name::from("a.log"), b"a.log"));
        assert!(!e.matches(&Name::from("# a comment"), b"# a comment"));
    }
}
