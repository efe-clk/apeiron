//! What a backup or a sync leaves out (CLI.md §4.1, SYNC.md §5.1).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::error::{Error, IoContext, Result};
use crate::name::Name;
use crate::search::wildcard;

/// The file of a folder that lists what to leave out of it.
pub const IGNORE_FILE: &str = ".apeironignore";

/// Ready sets of patterns, named `@<name>` (CLI.md §4.1).
const PRESETS: &[(&str, &[&str])] = &[
    ("rust", &["target"]),
    (
        "node",
        &[
            "node_modules",
            ".npm",
            ".pnpm-store",
            ".next",
            ".nuxt",
            ".svelte-kit",
            ".turbo",
            ".parcel-cache",
        ],
    ),
    (
        "python",
        &[
            "__pycache__",
            "*.pyc",
            ".venv",
            "venv",
            ".tox",
            ".nox",
            ".mypy_cache",
            ".pytest_cache",
            ".ruff_cache",
            "*.egg-info",
            ".ipynb_checkpoints",
        ],
    ),
    ("java", &[".gradle"]),
    ("cache", &[".cache"]),
    (
        "os",
        &[
            ".DS_Store",
            "Thumbs.db",
            "desktop.ini",
            ".Trash-*",
            "$RECYCLE.BIN",
        ],
    ),
    ("editors", &["*.swp", "*.swo", "*~"]),
];

/// The presets `@dev` stands for.
const DEV: &[&str] = &["rust", "node", "python", "java", "os", "editors"];

/// Patterns, each matched against an entry's name or, with a `/`, against its
/// whole path from the root of the folder; ignoring case. Paths given with
/// `+` keep only themselves, and `@gitignore` adds the folder's `.gitignore`
/// files.
#[derive(Debug, Default)]
pub(crate) struct Exclude {
    patterns: Vec<(bool, Vec<char>)>,
    /// Lowercase paths from the root; when there are any, all else is out.
    only: Vec<String>,
    git_wanted: bool,
    git: Option<GitIgnores>,
    /// The patterns as given and read, for `sync.left-out` (FORMAT.md §6.4).
    sources: Vec<String>,
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
            exclude.sources.push(pattern.clone());
        }
        let path = folder.join(IGNORE_FILE);
        match fs::read(&path) {
            Ok(bytes) => {
                for line in String::from_utf8_lossy(&bytes).lines() {
                    let line = line.trim();
                    if !line.starts_with('#') && exclude.add(line) {
                        exclude.sources.push(line.to_owned());
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
        exclude.find_gitignores(folder);
        Ok(exclude)
    }

    /// The patterns of an earlier round, as [`Exclude::sources`] gave them;
    /// one that is no longer valid is skipped.
    pub(crate) fn from_sources(folder: &Path, sources: &[String]) -> Self {
        let mut exclude = Exclude::default();
        for pattern in sources {
            if exclude.add(pattern) {
                exclude.sources.push(pattern.clone());
            }
        }
        exclude.find_gitignores(folder);
        exclude
    }

    fn find_gitignores(&mut self, folder: &Path) {
        if self.git_wanted {
            self.git = Some(GitIgnores {
                root: folder.to_path_buf(),
                files: Mutex::default(),
            });
        }
    }

    /// The patterns given and read from the folder, in order.
    pub(crate) fn sources(&self) -> &[String] {
        &self.sources
    }

    /// Adds a pattern; `false` if it matches nothing.
    fn add(&mut self, pattern: &str) -> bool {
        if let Some(name) = pattern.strip_prefix('@') {
            return self.add_preset(&name.to_lowercase());
        }
        if let Some(path) = pattern.strip_prefix('+') {
            let path = path.trim_matches('/');
            if path.is_empty() || path.split('/').any(|p| p.is_empty() || p == "..") {
                return false;
            }
            self.only.push(path.to_lowercase());
            return true;
        }
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

    fn add_preset(&mut self, name: &str) -> bool {
        match name {
            "gitignore" => self.git_wanted = true,
            "dev" => {
                for preset in DEV {
                    self.add_preset(preset);
                }
            }
            _ => match PRESETS.iter().find(|(n, _)| *n == name) {
                Some((_, patterns)) => {
                    for pattern in *patterns {
                        self.add(pattern);
                    }
                }
                None => return false,
            },
        }
        true
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.patterns.is_empty() && self.only.is_empty() && self.git.is_none()
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
        let (path_chars, name) = (lower(path.as_bytes()), lower(name));
        let by_pattern = self
            .patterns
            .iter()
            .any(|(whole, pattern)| wildcard(pattern, if *whole { &path_chars } else { &name }));
        if by_pattern {
            return true;
        }
        if !self.only.is_empty() {
            let path: String = path_chars.iter().collect();
            let kept = self.only.iter().any(|p| {
                path == *p
                    || path
                        .strip_prefix(p.as_str())
                        .is_some_and(|r| r.starts_with('/'))
                    || p.strip_prefix(path.as_str())
                        .is_some_and(|r| r.starts_with('/'))
            });
            if !kept {
                return true;
            }
        }
        self.git.as_ref().is_some_and(|git| git.ignores(path))
    }
}

/// The `.gitignore` files of a folder, read when first needed.
#[derive(Debug)]
struct GitIgnores {
    root: PathBuf,
    /// By folder path from the root; `None` for a folder without the file.
    files: Mutex<HashMap<String, Option<Arc<Gitignore>>>>,
}

impl GitIgnores {
    fn file(&self, folder: &str) -> Option<Arc<Gitignore>> {
        let mut files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        files
            .entry(folder.to_owned())
            .or_insert_with(|| {
                let dir = self.root.join(folder);
                let file = dir.join(".gitignore");
                if !file.is_file() {
                    return None;
                }
                let mut builder = GitignoreBuilder::new(&dir);
                // A line that is not a valid pattern is skipped, as Git does.
                let _ = builder.add(&file);
                builder.build().ok().map(Arc::new)
            })
            .clone()
    }

    /// Whether the `.gitignore` files in the folders above `path` leave it
    /// out; the deepest file that names it decides, as in Git.
    fn ignores(&self, path: &Name) -> bool {
        let path = String::from_utf8_lossy(path.as_bytes()).into_owned();
        let full = self.root.join(&path);
        let is_dir = fs::symlink_metadata(&full).is_ok_and(|m| m.is_dir());
        let mut folder = path.as_str();
        while let Some(end) = folder.rfind('/').or((!folder.is_empty()).then_some(0)) {
            folder = &folder[..end];
            if let Some(file) = self.file(folder) {
                match file.matched(&full, is_dir) {
                    Match::Ignore(_) => return true,
                    Match::Whitelist(_) => return false,
                    Match::None => {}
                }
            }
            if folder.is_empty() {
                break;
            }
        }
        false
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

    #[test]
    fn presets_stand_for_their_patterns() {
        let e = exclude(&["@node", "@Python"]);
        let m = |path: &str| {
            let name = path.rsplit('/').next().unwrap();
            e.matches(&Name::from(path), name.as_bytes())
        };
        assert!(m("web/node_modules"));
        assert!(m("app/__pycache__"));
        assert!(m("app/x.pyc"));
        assert!(!m("target"));
        let dev = exclude(&["@dev"]);
        assert!(dev.matches(&Name::from("crate/target"), b"target"));
        assert!(dev.matches(&Name::from(".DS_Store"), b".DS_Store"));
        assert!(!Exclude::default().add("@nothing"));
    }

    #[test]
    fn plus_paths_keep_only_themselves() {
        let e = exclude(&["+.bashrc", "+/.config/nvim/"]);
        let m = |path: &str| {
            let name = path.rsplit('/').next().unwrap();
            e.matches(&Name::from(path), name.as_bytes())
        };
        assert!(!m(".bashrc"));
        assert!(!m(".config"));
        assert!(!m(".config/nvim"));
        assert!(!m(".config/nvim/init.lua"));
        assert!(m(".config/other"));
        assert!(m(".config/nvim-old"));
        assert!(m("Documents"));
        assert!(!Exclude::default().add("+"));
        assert!(!Exclude::default().add("+a/../b"));
    }

    #[test]
    fn gitignore_files_count_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("app/build")).unwrap();
        fs::create_dir_all(root.join("app/logs")).unwrap();
        fs::write(root.join(".gitignore"), "*.log\nbuild/\n").unwrap();
        fs::write(root.join("app/.gitignore"), "!keep.log\nsecret.txt\n").unwrap();
        for f in [
            "a.log",
            "app/b.log",
            "app/keep.log",
            "app/secret.txt",
            "app/main.rs",
        ] {
            fs::write(root.join(f), "x").unwrap();
        }
        let m = |e: &Exclude, path: &str| {
            let name = path.rsplit('/').next().unwrap();
            e.matches(&Name::from(path), name.as_bytes())
        };
        let without = Exclude::for_folder(root, &[]).unwrap();
        assert!(!m(&without, "a.log"));

        let e = Exclude::for_folder(root, &["@gitignore".to_owned()]).unwrap();
        assert!(m(&e, "a.log"));
        assert!(m(&e, "app/b.log"));
        assert!(!m(&e, "app/keep.log"));
        assert!(m(&e, "app/secret.txt"));
        assert!(m(&e, "app/build"));
        assert!(!m(&e, "app/logs"));
        assert!(!m(&e, "app/main.rs"));
        assert!(!m(&e, ".gitignore"));

        // The same through the folder's own file.
        fs::write(root.join(IGNORE_FILE), "@gitignore\n").unwrap();
        assert!(m(&Exclude::for_folder(root, &[]).unwrap(), "a.log"));
    }
}
