//! Merging one ref into another (BRANCH.md).

use std::fs;
use std::path::PathBuf;

use apeiron_core::{EntryKind, ObjectId, Repository};
use apeiron_services::{
    BackupOptions, ConflictKind, ContentMerger, Control, Error, MergeAction, MergeOptions, Merged,
    WarningKind, backup, merge,
};

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: Repository,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let repo = Repository::init(&root.join("repo")).unwrap();
    Setup {
        _dir: dir,
        root,
        repo,
    }
}

impl Setup {
    /// Backs up a folder holding `files` into `ref_name`.
    fn commit(&mut self, ref_name: &str, files: &[(&str, &str)]) -> ObjectId {
        let folder = self.root.join(format!("folder-{}", rand_name(files)));
        let _ = fs::remove_dir_all(&folder);
        for (name, text) in files {
            fs::create_dir_all(folder.join(name).parent().unwrap()).unwrap();
            fs::write(folder.join(name), text).unwrap();
        }
        let options = BackupOptions::new(&folder, ref_name);
        backup(&mut self.repo, &options, &Control::new())
            .unwrap()
            .snapshot
    }

    fn merge(&mut self, from: &str, into: &str, dry_run: bool) -> apeiron_services::MergeOutcome {
        self.merge_with(from, into, dry_run, None)
    }

    fn merge_with(
        &mut self,
        from: &str,
        into: &str,
        dry_run: bool,
        merger: Option<&dyn ContentMerger>,
    ) -> apeiron_services::MergeOutcome {
        let options = MergeOptions {
            from: self.repo.read_ref(from).unwrap().unwrap(),
            from_name: from.to_owned(),
            into: into.to_owned(),
            message: None,
            author: Some("Ada <ada@example.org>".to_owned()),
            host: None,
            dry_run,
        };
        merge(&mut self.repo, &options, merger).unwrap()
    }

    /// Every file of the snapshot `name` points to, with its content.
    fn files(&self, name: &str) -> Vec<(String, String)> {
        let snapshot = self.repo.read_ref(name).unwrap().unwrap();
        let mut out = Vec::new();
        collect(
            &self.repo,
            &self.repo.read_snapshot(&snapshot).unwrap().tree,
            "",
            &mut out,
        );
        out
    }
}

fn rand_name(files: &[(&str, &str)]) -> String {
    files
        .iter()
        .map(|(n, t)| format!("{n}{t}"))
        .collect::<String>()
        .bytes()
        .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)))
        .to_string()
}

fn collect(repo: &Repository, tree: &ObjectId, prefix: &str, out: &mut Vec<(String, String)>) {
    for entry in repo.read_tree(tree).unwrap().entries {
        let name = format!("{prefix}{}", String::from_utf8_lossy(&entry.name));
        match entry.kind {
            EntryKind::Directory { tree } => collect(repo, &tree, &format!("{name}/"), out),
            EntryKind::File { blob, .. } => {
                let mut data = Vec::new();
                if let Some(blob) = blob {
                    repo.read_file(&blob, &mut data).unwrap();
                }
                out.push((name, String::from_utf8(data).unwrap()));
            }
            EntryKind::Symlink { .. } => {}
        }
    }
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

#[test]
fn a_branch_that_only_moved_ahead_is_fast_forwarded() {
    let mut s = setup();
    let first = s.commit("main", &[("a.txt", "a"), ("b.txt", "b")]);
    s.repo.update_ref("feature", None, Some(first)).unwrap();
    let ahead = s.commit("feature", &[("a.txt", "a2"), ("b.txt", "b")]);

    let preview = s.merge("feature", "main", true);
    assert_eq!(preview.action, MergeAction::FastForward);
    assert_eq!(s.repo.read_ref("main").unwrap(), Some(first));
    let out = s.merge("feature", "main", false);
    assert_eq!(
        (out.action, out.previous, out.snapshot),
        (MergeAction::FastForward, first, ahead)
    );
    assert_eq!(out.changes.len(), 1);
    assert_eq!(s.repo.read_ref("main").unwrap(), Some(ahead));
    assert_eq!(
        s.merge("feature", "main", false).action,
        MergeAction::UpToDate
    );
}

#[test]
fn changes_on_both_sides_are_joined() {
    let mut s = setup();
    let first = s.commit("main", &[("a.txt", "a"), ("sub/b.txt", "b")]);
    s.repo.update_ref("feature", None, Some(first)).unwrap();
    let theirs = s.commit(
        "feature",
        &[("a.txt", "a"), ("sub/b.txt", "b2"), ("new.txt", "n")],
    );
    let ours = s.commit("main", &[("a.txt", "a2"), ("sub/b.txt", "b")]);

    let preview = s.merge("feature", "main", true);
    assert_eq!(preview.action, MergeAction::Merged);
    assert_eq!(s.repo.read_ref("main").unwrap(), Some(ours));
    let out = s.merge("feature", "main", false);
    assert_eq!(out.action, MergeAction::Merged);
    assert!(out.conflicts.is_empty());
    assert_eq!(out.changes.len(), 2, "{:?}", out.changes);
    let merged = s.repo.read_snapshot(&out.snapshot).unwrap();
    assert_eq!(merged.parents, [ours, theirs]);
    assert_eq!(merged.message.as_deref(), Some("merge feature into main"));
    assert_eq!(merged.author.as_deref(), Some("Ada <ada@example.org>"));
    assert_eq!(
        s.files("main"),
        pairs(&[("a.txt", "a2"), ("new.txt", "n"), ("sub/b.txt", "b2")])
    );
    // Merged again: nothing to do, in either direction it is the same history.
    assert_eq!(
        s.merge("feature", "main", false).action,
        MergeAction::UpToDate
    );
    assert_eq!(
        s.merge("main", "feature", false).action,
        MergeAction::FastForward
    );
}

#[test]
fn conflicting_changes_keep_both_versions() {
    let mut s = setup();
    let first = s.commit("main", &[("a.txt", "a"), ("b.txt", "b")]);
    s.repo.update_ref("topic/x", None, Some(first)).unwrap();
    s.commit("topic/x", &[("a.txt", "theirs")]);
    s.commit("main", &[("a.txt", "ours"), ("b.txt", "b2")]);

    let out = s.merge("topic/x", "main", false);
    let kinds: Vec<(String, ConflictKind)> = out
        .conflicts
        .iter()
        .map(|c| (c.path.to_string(), c.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("a.txt".to_owned(), ConflictKind::BothChanged),
            ("b.txt".to_owned(), ConflictKind::DeletedThere),
        ]
    );
    assert_eq!(
        s.files("main"),
        pairs(&[
            ("a (conflict topic-x).txt", "theirs"),
            ("a.txt", "ours"),
            ("b.txt", "b2"),
        ])
    );
    let message = s
        .repo
        .read_snapshot(&out.snapshot)
        .unwrap()
        .message
        .unwrap();
    assert!(message.contains("conflict: a.txt"), "{message}");
}

#[test]
fn unrelated_histories_merge_without_losing_anything() {
    let mut s = setup();
    s.commit("main", &[("a.txt", "a")]);
    s.commit("other", &[("a.txt", "x"), ("o.txt", "o")]);
    let out = s.merge("other", "main", false);
    assert_eq!(out.action, MergeAction::Merged);
    assert_eq!(
        s.files("main"),
        pairs(&[
            ("a (conflict other).txt", "x"),
            ("a.txt", "a"),
            ("o.txt", "o")
        ])
    );

    let options = MergeOptions {
        from: out.snapshot,
        from_name: "x".to_owned(),
        into: "missing".to_owned(),
        message: None,
        author: None,
        host: None,
        dry_run: false,
    };
    assert!(matches!(
        merge(&mut s.repo, &options, None),
        Err(Error::NotFound(_))
    ));
}

/// Joins the lines of both sides after the base for `*.log` files; refuses
/// `*.bad` files and fails on `*.err` files.
struct Lines;

impl ContentMerger for Lines {
    fn handles(&self, name: &[u8]) -> bool {
        [&b".log"[..], b".bad", b".err"]
            .iter()
            .any(|ext| name.ends_with(ext))
    }

    fn merge(
        &self,
        name: &[u8],
        base: Option<&[u8]>,
        ours: &[u8],
        theirs: &[u8],
    ) -> Result<Option<Merged>, String> {
        if name.ends_with(b".err") {
            return Err("the plugin crashed".to_owned());
        }
        if name.ends_with(b".bad") {
            return Ok(None);
        }
        let base = base.unwrap_or_default();
        let mut data = ours.to_vec();
        data.extend_from_slice(&theirs[base.len()..]);
        Ok(Some(Merged {
            data,
            plugin: "lines".to_owned(),
        }))
    }
}

#[test]
fn a_type_driver_merges_files_both_sides_changed() {
    let mut s = setup();
    let first = s.commit(
        "main",
        &[
            ("a.log", "1\n"),
            ("b.bad", "x"),
            ("c.err", "x"),
            ("d.txt", "x"),
        ],
    );
    s.repo.update_ref("feature", None, Some(first)).unwrap();
    s.commit(
        "feature",
        &[
            ("a.log", "1\ntheirs\n"),
            ("b.bad", "t"),
            ("c.err", "t"),
            ("d.txt", "t"),
        ],
    );
    s.commit(
        "main",
        &[
            ("a.log", "1\nours\n"),
            ("b.bad", "o"),
            ("c.err", "o"),
            ("d.txt", "o"),
        ],
    );

    let out = s.merge_with("feature", "main", false, Some(&Lines));
    assert_eq!(out.plugin_merges.len(), 1);
    assert_eq!(
        (
            out.plugin_merges[0].path.to_string(),
            out.plugin_merges[0].plugin.as_str()
        ),
        ("a.log".to_owned(), "lines")
    );
    let conflicts: Vec<String> = out.conflicts.iter().map(|c| c.path.to_string()).collect();
    assert_eq!(conflicts, ["b.bad", "c.err", "d.txt"]);
    assert_eq!(out.warnings.len(), 1);
    assert_eq!(
        (out.warnings[0].path.to_string(), out.warnings[0].kind),
        ("c.err".to_owned(), WarningKind::Plugin)
    );
    let files = s.files("main");
    assert!(
        files.contains(&("a.log".to_owned(), "1\nours\ntheirs\n".to_owned())),
        "{files:?}"
    );
    assert!(!files.iter().any(|(n, _)| n.starts_with("a (conflict")));
    let message = s
        .repo
        .read_snapshot(&out.snapshot)
        .unwrap()
        .message
        .unwrap();
    assert!(
        message.contains("merged by plugin lines: a.log"),
        "{message}"
    );
}
