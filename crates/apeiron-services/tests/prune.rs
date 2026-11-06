//! Thinning out the history of a backup (FORMAT.md §13.4).

use std::fs;
use std::path::Path;

use apeiron_core::{
    EntryKind, FolderStore, GcOptions, ObjectId, Repository, Snapshot, Store, Time, Tree, TreeEntry,
};
use apeiron_services::{Error, PruneOptions, prune};

const DAY: i64 = 86_400;

/// Adds a snapshot holding one file with `text` on `day`.
fn commit(repo: &mut Repository, name: &str, day: i64, text: &str) -> ObjectId {
    let parent = repo.read_ref(name).unwrap();
    let mut writer = repo.writer();
    let (size, blob) = writer.put_file(text.as_bytes()).unwrap();
    let tree = writer
        .put_tree(&Tree {
            entries: vec![TreeEntry {
                name: b"a.txt".to_vec(),
                kind: EntryKind::File { size, blob },
                mode: None,
                mtime: None,
                uid: None,
                gid: None,
                xattrs: Vec::new(),
            }],
        })
        .unwrap();
    let id = writer
        .put_snapshot(&Snapshot {
            tree,
            parents: parent.into_iter().collect(),
            time: Time {
                seconds: day * DAY,
                nanos: 0,
            },
            author: None,
            host: None,
            message: Some(format!("day {day}")),
            meta: Vec::new(),
        })
        .unwrap();
    writer.finish().unwrap();
    repo.update_ref(name, parent, Some(id)).unwrap();
    id
}

fn messages(repo: &Repository, name: &str) -> Vec<String> {
    let head = repo.read_ref(name).unwrap().unwrap();
    repo.history(head)
        .map(|s| s.unwrap().1.message.unwrap())
        .collect()
}

fn five_days(path: &Path) -> Repository {
    let mut repo = Repository::init(path).unwrap();
    for day in 1..=5 {
        commit(&mut repo, "backup/home", day, &format!("content {day}"));
    }
    repo
}

fn keep_last(n: u32) -> PruneOptions {
    PruneOptions {
        ref_name: "backup/home".to_owned(),
        keep_last: n,
        ..PruneOptions::default()
    }
}

#[test]
fn old_snapshots_leave_and_gc_frees_their_data() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = five_days(&dir.path().join("repo"));
    let before = repo.read_ref("backup/home").unwrap();

    let trial = prune(
        &mut repo,
        &PruneOptions {
            dry_run: true,
            ..keep_last(2)
        },
    )
    .unwrap();
    assert_eq!(trial.removed.len(), 3);
    assert_eq!(repo.read_ref("backup/home").unwrap(), before);

    let out = prune(&mut repo, &keep_last(2)).unwrap();
    assert_eq!(out.removed.len(), 3);
    assert_eq!(out.snapshot, trial.snapshot);
    assert_eq!(messages(&repo, "backup/home"), ["day 5", "day 4"]);
    // The old head stays in the reflog until garbage collection.
    assert!(repo.read_snapshot(&out.previous).is_ok());
    let kept = repo.gc(&GcOptions::default()).unwrap();
    assert_eq!(kept.removed_objects, 0);
    let freed = repo
        .gc(&GcOptions {
            keep_reflog_days: 0,
            dry_run: false,
        })
        .unwrap();
    assert!(freed.removed_objects > 0);
    assert!(repo.read_snapshot(&out.previous).is_err());
    assert!(repo.verify().unwrap().is_ok());
    assert_eq!(messages(&repo, "backup/home"), ["day 5", "day 4"]);
}

#[test]
fn keeping_everything_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = five_days(&dir.path().join("repo"));
    let before = repo.read_ref("backup/home").unwrap().unwrap();
    let out = prune(&mut repo, &keep_last(9)).unwrap();
    assert!(out.removed.is_empty());
    assert_eq!(out.snapshot, before);
    assert_eq!(out.kept[4].new, out.kept[4].snapshot);
}

#[test]
fn without_a_rule_nothing_happens() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = five_days(&dir.path().join("repo"));
    assert!(matches!(
        prune(&mut repo, &keep_last(0)),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn a_kept_or_held_ref_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = five_days(&dir.path().join("repo"));
    let hold = repo.add_legal_hold("backup/*", "Case 1").unwrap();
    assert!(prune(&mut repo, &keep_last(1)).is_err());
    repo.lift_legal_hold(&hold.id, "Closed").unwrap();
    prune(&mut repo, &keep_last(4)).unwrap();
    repo.add_retention_rule("backup/*", 30).unwrap();
    assert!(prune(&mut repo, &keep_last(1)).is_err());
    assert_eq!(messages(&repo, "backup/home").len(), 4);
}

#[test]
fn refs_of_sync_and_histories_with_merges_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repository::init(&dir.path().join("repo")).unwrap();
    let id = commit(&mut repo, "sync/docs", 1, "x");
    repo.update_ref("folder/sync/docs", None, Some(id)).unwrap();
    let mut options = keep_last(1);
    options.ref_name = "sync/docs".to_owned();
    assert!(matches!(
        prune(&mut repo, &options),
        Err(Error::Unsupported(_))
    ));
    options.ref_name = "folder/sync/docs".to_owned();
    assert!(matches!(
        prune(&mut repo, &options),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn a_copy_follows_only_with_replace() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("copy");
    fs::create_dir(&copy).unwrap();
    let store = FolderStore::open(&copy).unwrap();
    let mut repo = five_days(&dir.path().join("repo"));
    repo.push(&store, "backup/home", &mut |_| Ok(())).unwrap();
    prune(&mut repo, &keep_last(2)).unwrap();

    // A plain push refuses: the copy's history is not in the new one.
    assert!(matches!(
        repo.push(&store, "backup/home", &mut |_| Ok(())),
        Err(apeiron_core::Error::RemoteAhead(_))
    ));
    repo.push_replacing(&store, "backup/home", &mut |_| Ok(()))
        .unwrap();
    assert_eq!(
        store.read_ref("backup/home").unwrap(),
        repo.read_ref("backup/home").unwrap()
    );
    // Run again, it has nothing to do.
    assert_eq!(
        repo.push_replacing(&store, "backup/home", &mut |_| Ok(()))
            .unwrap()
            .objects,
        0
    );
    drop(store);
    let reopened = Repository::open(&copy).unwrap();
    assert_eq!(messages(&reopened, "backup/home"), ["day 5", "day 4"]);
    assert!(reopened.verify().unwrap().is_ok());
}

#[test]
fn replace_refuses_a_copy_with_snapshots_this_one_never_had() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("copy");
    fs::create_dir(&copy).unwrap();
    let store = FolderStore::open(&copy).unwrap();
    let mut repo = five_days(&dir.path().join("repo"));
    repo.push(&store, "backup/home", &mut |_| Ok(())).unwrap();
    drop(store);

    // Another device adds a backup to the copy.
    let mut other = Repository::open(&copy).unwrap();
    commit(&mut other, "backup/home", 6, "from elsewhere");
    drop(other);

    prune(&mut repo, &keep_last(2)).unwrap();
    let store = FolderStore::open(&copy).unwrap();
    assert!(matches!(
        repo.push_replacing(&store, "backup/home", &mut |_| Ok(())),
        Err(apeiron_core::Error::RemoteAhead(_))
    ));
}
