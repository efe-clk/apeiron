//! A working folder that follows a branch (BRANCH.md §6).

use std::fs;
use std::path::Path;

use apeiron_core::Repository;
use apeiron_services::{
    BackupOptions, ChangeKind, Control, Error, SwitchOptions, SwitchOutcome, backup, restore,
    switch,
};

fn write(folder: &Path, files: &[(&str, &str)]) {
    let _ = fs::remove_dir_all(folder);
    for (name, text) in files {
        fs::create_dir_all(folder.join(name).parent().unwrap()).unwrap();
        fs::write(folder.join(name), text).unwrap();
    }
}

fn read(folder: &Path) -> Vec<(String, String)> {
    let mut out: Vec<_> = walkdir(folder)
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(folder)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (rel, fs::read_to_string(&p).unwrap())
        })
        .collect();
    out.sort();
    out
}

fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walkdir(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

fn options(repo: &Repository, folder: &Path, to: &str, dry_run: bool) -> SwitchOptions {
    SwitchOptions {
        folder: folder.to_path_buf(),
        target: repo.read_ref(to).unwrap().unwrap(),
        target_name: to.to_owned(),
        name: "switch/work".to_owned(),
        dry_run,
        host: None,
    }
}

fn go(
    repo: &mut Repository,
    folder: &Path,
    to: &str,
    dry_run: bool,
) -> apeiron_services::Result<SwitchOutcome> {
    let options = options(repo, folder, to, dry_run);
    switch(repo, &options, &Control::new())
}

#[test]
fn a_folder_follows_branches_and_keeps_unsaved_work() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut repo = Repository::init(&root.join("repo")).unwrap();
    let src = root.join("src");
    write(&src, &[("a.txt", "main a"), ("sub/b.txt", "b")]);
    backup(
        &mut repo,
        &BackupOptions::new(&src, "main"),
        &Control::new(),
    )
    .unwrap();
    write(&src, &[("a.txt", "feature a"), ("c.txt", "c")]);
    backup(
        &mut repo,
        &BackupOptions::new(&src, "feature"),
        &Control::new(),
    )
    .unwrap();

    // A new folder is filled.
    let work = root.join("work");
    let out = go(&mut repo, &work, "main", false).unwrap();
    assert_eq!(out.saved, None);
    assert_eq!(
        read(&work),
        pairs(&[("a.txt", "main a"), ("sub/b.txt", "b")])
    );

    // A dry run changes nothing but tells what would change.
    let out = go(&mut repo, &work, "feature", true).unwrap();
    assert_eq!(out.changes.len(), 4, "{:?}", out.changes);
    assert_eq!(
        read(&work),
        pairs(&[("a.txt", "main a"), ("sub/b.txt", "b")])
    );

    // Switching changes only what differs.
    let out = go(&mut repo, &work, "feature", false).unwrap();
    assert_eq!(out.saved, None);
    assert!(out.conflicts.is_empty() && out.skipped.is_empty());
    assert_eq!(
        read(&work),
        pairs(&[("a.txt", "feature a"), ("c.txt", "c")])
    );
    let added = out.changes.iter().filter(|c| c.kind == ChangeKind::Added);
    assert_eq!(added.count(), 1);

    // Work not backed up is recorded before the folder changes.
    fs::write(work.join("new.txt"), "unsaved").unwrap();
    let out = go(&mut repo, &work, "main", false).unwrap();
    let saved = out.saved.expect("the folder is recorded first");
    assert_eq!(
        read(&work),
        pairs(&[("a.txt", "main a"), ("sub/b.txt", "b")])
    );
    let back = root.join("back");
    restore(&repo, &saved, b"", &back, &Control::new()).unwrap();
    assert_eq!(
        read(&back),
        pairs(&[
            ("a.txt", "feature a"),
            ("c.txt", "c"),
            ("new.txt", "unsaved")
        ])
    );
    assert_eq!(
        repo.read_ref("folder/switch/work").unwrap(),
        repo.read_ref("main").unwrap()
    );
}

#[test]
fn a_folder_never_switched_must_hold_what_a_ref_has() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut repo = Repository::init(&root.join("repo")).unwrap();
    let src = root.join("src");
    write(&src, &[("a.txt", "a")]);
    backup(
        &mut repo,
        &BackupOptions::new(&src, "main"),
        &Control::new(),
    )
    .unwrap();

    let other = root.join("other");
    write(&other, &[("mine.txt", "not in any ref")]);
    let err = go(&mut repo, &other, "main", false);
    assert!(matches!(err, Err(Error::InvalidInput(_))), "{err:?}");
    assert_eq!(read(&other), pairs(&[("mine.txt", "not in any ref")]));

    // A folder restored from a ref may switch.
    let restored = root.join("restored");
    let main = repo.read_ref("main").unwrap().unwrap();
    restore(&repo, &main, b"", &restored, &Control::new()).unwrap();
    let out = go(&mut repo, &restored, "main", false).unwrap();
    assert!(out.changes.is_empty() && out.saved.is_none(), "{out:?}");
}
