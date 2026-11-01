//! Files added to a backup from outside (SHARE.md §7).

use std::fs;

use apeiron_core::Repository;
use apeiron_services::{AddFileOptions, BackupOptions, Control, Error, add_file, backup, restore};

fn options(name: &str, folder: &str) -> AddFileOptions {
    AddFileOptions {
        ref_name: "backup/inbox".to_owned(),
        folder: folder.into(),
        name: name.to_owned(),
        message: Some("upload".to_owned()),
        host: None,
    }
}

#[test]
fn files_are_added_beside_what_the_backup_holds() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut repo = Repository::init(&root.join("repo")).unwrap();
    let src = root.join("src");
    fs::create_dir_all(src.join("uploads")).unwrap();
    fs::write(src.join("uploads/a.txt"), "already here").unwrap();
    fs::write(src.join("note.txt"), "note").unwrap();
    let first = backup(
        &mut repo,
        &BackupOptions::new(&src, "backup/inbox"),
        &Control::new(),
    )
    .unwrap()
    .snapshot;

    let added = add_file(&mut repo, &options("a.txt", "uploads"), &mut &b"new"[..]).unwrap();
    assert_eq!(added.path.to_string(), "uploads/a (2).txt");
    assert_eq!(added.size, 3);
    let snapshot = repo.read_snapshot(&added.snapshot).unwrap();
    assert_eq!(snapshot.parents, [first]);
    let added = add_file(
        &mut repo,
        &options("b.txt", "uploads/2026/nov"),
        &mut &b"deep"[..],
    )
    .unwrap();
    assert_eq!(added.path.to_string(), "uploads/2026/nov/b.txt");

    let out = root.join("out");
    let head = repo.read_ref("backup/inbox").unwrap().unwrap();
    restore(&repo, &head, b"", &out, &Control::new()).unwrap();
    let read = |p: &str| fs::read_to_string(out.join(p)).unwrap();
    assert_eq!(read("uploads/a.txt"), "already here");
    assert_eq!(read("uploads/a (2).txt"), "new");
    assert_eq!(read("uploads/2026/nov/b.txt"), "deep");
    assert_eq!(read("note.txt"), "note");

    // A file where a folder is asked for, and names that are not one name.
    for (name, folder) in [
        ("x", "note.txt"),
        ("../x", ""),
        ("a/b", ""),
        ("", ""),
        ("x", ".."),
    ] {
        let err = add_file(&mut repo, &options(name, folder), &mut &b""[..]);
        assert!(
            matches!(err, Err(Error::InvalidInput(_))),
            "{name} {folder}"
        );
    }
    assert_eq!(repo.read_ref("backup/inbox").unwrap(), Some(added.snapshot));

    // A ref that does not exist yet is made.
    let mut fresh = options("first.txt", "");
    fresh.ref_name = "backup/new".to_owned();
    let added = add_file(&mut repo, &fresh, &mut &b"1"[..]).unwrap();
    assert!(
        repo.read_snapshot(&added.snapshot)
            .unwrap()
            .parents
            .is_empty()
    );
}
