//! Sync rounds between two devices through a folder remote (SYNC.md).

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use apeiron_core::{FolderStore, ObjectId, Repository, Store};
use apeiron_services::{
    ChangeKind, ConflictKind, Control, SyncAction, SyncMode, SyncOptions, SyncOutcome, sync,
};

const REF: &str = "sync/docs";

struct Device {
    repo: Repository,
    folder: PathBuf,
    host: &'static str,
}

struct Setup {
    _dir: tempfile::TempDir,
    remote: PathBuf,
    a: Device,
    b: Device,
}

/// Device `a` has synced its folder; `b` is a clone with an empty folder.
fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let remote = root.join("remote");
    let repo = Repository::init(&root.join("a/repo")).unwrap();
    let mut a = Device {
        repo,
        folder: root.join("a/docs"),
        host: "alpha",
    };
    write(&a.folder, "notes.txt", "notes");
    write(&a.folder, "sub/plan.md", "plan");
    let out = round(&mut a, &remote);
    assert_eq!(out.action, SyncAction::Sent);
    assert!(out.pushed.is_some());

    let store = FolderStore::open(&remote).unwrap();
    let (repo, _) =
        Repository::clone_from(&store, &root.join("b/repo"), None, &mut |_| Ok(())).unwrap();
    drop(store);
    let b = Device {
        repo,
        folder: root.join("b/docs"),
        host: "beta",
    };
    fs::create_dir_all(&b.folder).unwrap();
    Setup {
        _dir: dir,
        remote,
        a,
        b,
    }
}

fn options(device: &Device) -> SyncOptions {
    SyncOptions {
        folder: device.folder.clone(),
        ref_name: REF.to_owned(),
        mode: SyncMode::TwoWay,
        dry_run: false,
        host: Some(device.host.to_owned()),
    }
}

fn round(device: &mut Device, remote: &Path) -> SyncOutcome {
    round_with(device, &FolderStore::open(remote).unwrap(), options(device))
}

fn round_with(device: &mut Device, store: &dyn Store, options: SyncOptions) -> SyncOutcome {
    let out = sync(&mut device.repo, store, &options, None, &Control::new()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    out
}

fn write(folder: &Path, rel: &str, text: &str) {
    let path = folder.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Every file below `folder` with its content, sorted.
fn files(folder: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    collect(folder, "", &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if entry.file_type().unwrap().is_dir() {
            collect(&entry.path(), &format!("{name}/"), out);
        } else {
            out.push((name, fs::read_to_string(entry.path()).unwrap()));
        }
    }
}

fn names(folder: &Path) -> Vec<String> {
    files(folder).into_iter().map(|(n, _)| n).collect()
}

fn remote_ref(remote: &Path) -> Option<ObjectId> {
    FolderStore::open(remote).unwrap().read_ref(REF).unwrap()
}

#[test]
fn changes_on_one_device_reach_the_other() {
    let mut s = setup();
    let out = round(&mut s.b, &s.remote);
    assert_eq!(out.action, SyncAction::Received);
    assert_eq!(out.changes.len(), 3, "{:?}", out.changes);
    assert_eq!(files(&s.b.folder), files(&s.a.folder));
    assert_eq!(round(&mut s.b, &s.remote).action, SyncAction::UpToDate);
    assert_eq!(round(&mut s.a, &s.remote).action, SyncAction::UpToDate);

    // A change, a new file and a deletion travel back.
    write(&s.b.folder, "notes.txt", "notes, edited on beta");
    write(&s.b.folder, "new.txt", "new");
    fs::remove_dir_all(s.b.folder.join("sub")).unwrap();
    assert_eq!(round(&mut s.b, &s.remote).action, SyncAction::Sent);
    let out = round(&mut s.a, &s.remote);
    assert_eq!(out.action, SyncAction::Received);
    assert_eq!(files(&s.a.folder), files(&s.b.folder));
    assert!(!s.a.folder.join("sub").exists());
    let kinds: Vec<ChangeKind> = out.changes.iter().map(|c| c.kind).collect();
    assert!(kinds.contains(&ChangeKind::Added) && kinds.contains(&ChangeKind::Removed));
    assert_eq!(s.a.repo.read_ref(REF).unwrap(), remote_ref(&s.remote));
}

#[test]
fn changes_to_different_files_are_merged() {
    let mut s = setup();
    round(&mut s.b, &s.remote);
    write(&s.a.folder, "notes.txt", "notes from alpha");
    write(&s.b.folder, "sub/plan.md", "plan from beta");
    write(&s.b.folder, "sub/more.md", "more");
    round(&mut s.a, &s.remote);

    let out = round(&mut s.b, &s.remote);
    assert_eq!(out.action, SyncAction::Merged);
    assert!(out.conflicts.is_empty());
    let merged = s.b.repo.read_snapshot(&out.snapshot.unwrap()).unwrap();
    assert_eq!(merged.parents.len(), 2);
    assert_eq!(out.pushed.unwrap().snapshot, out.snapshot.unwrap());

    assert_eq!(round(&mut s.a, &s.remote).action, SyncAction::Received);
    let expected = vec![
        ("notes.txt".to_owned(), "notes from alpha".to_owned()),
        ("sub/more.md".to_owned(), "more".to_owned()),
        ("sub/plan.md".to_owned(), "plan from beta".to_owned()),
    ];
    assert_eq!(files(&s.a.folder), expected);
    assert_eq!(files(&s.b.folder), expected);

    // Folder times differ between the devices; that alone sends nothing.
    let settled = remote_ref(&s.remote);
    for _ in 0..2 {
        assert_eq!(round(&mut s.b, &s.remote).action, SyncAction::UpToDate);
        assert_eq!(round(&mut s.a, &s.remote).action, SyncAction::UpToDate);
    }
    assert_eq!(remote_ref(&s.remote), settled);
}

#[test]
fn conflicting_changes_keep_both_versions() {
    let mut s = setup();
    round(&mut s.b, &s.remote);
    write(&s.a.folder, "notes.txt", "alpha's notes");
    write(&s.b.folder, "notes.txt", "beta's notes");
    // Deleted on one side and changed on the other: the change wins.
    fs::remove_file(s.a.folder.join("sub/plan.md")).unwrap();
    write(&s.b.folder, "sub/plan.md", "beta's plan");
    round(&mut s.a, &s.remote);

    let out = round(&mut s.b, &s.remote);
    assert_eq!(out.action, SyncAction::Merged);
    let mut conflicts: Vec<_> = out
        .conflicts
        .iter()
        .map(|c| (c.path.to_string(), c.kind))
        .collect();
    conflicts.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        conflicts,
        [
            ("notes.txt".to_owned(), ConflictKind::BothChanged),
            ("sub/plan.md".to_owned(), ConflictKind::DeletedThere),
        ]
    );
    let copy = out.conflicts[0].copy.as_ref().unwrap().to_string();
    assert!(
        copy.starts_with("notes (conflict alpha ") && copy.ends_with(").txt"),
        "{copy}"
    );
    round(&mut s.a, &s.remote);
    for folder in [&s.a.folder, &s.b.folder] {
        assert_eq!(
            files(folder),
            vec![
                (copy.clone(), "alpha's notes".to_owned()),
                ("notes.txt".to_owned(), "beta's notes".to_owned()),
                ("sub/plan.md".to_owned(), "beta's plan".to_owned()),
            ]
        );
    }
    // The merge message lists the conflicts.
    let message =
        s.a.repo
            .read_snapshot(&remote_ref(&s.remote).unwrap())
            .unwrap()
            .message;
    assert!(message.unwrap().contains("conflict: notes.txt"));
}

#[test]
fn a_new_device_with_files_merges_them() {
    let mut s = setup();
    write(&s.b.folder, "notes.txt", "notes");
    write(&s.b.folder, "beta.txt", "only on beta");
    let out = round(&mut s.b, &s.remote);
    assert_eq!(out.action, SyncAction::Merged);
    assert!(out.conflicts.is_empty());
    round(&mut s.a, &s.remote);
    assert_eq!(names(&s.a.folder), ["beta.txt", "notes.txt", "sub/plan.md"]);
    assert_eq!(files(&s.a.folder), files(&s.b.folder));
}

#[test]
fn dry_run_changes_nothing() {
    let mut s = setup();
    write(&s.b.folder, "beta.txt", "beta");
    let before = remote_ref(&s.remote);
    let mut options = options(&s.b);
    options.dry_run = true;
    let store = FolderStore::open(&s.remote).unwrap();
    let out = round_with(&mut s.b, &store, options);
    assert_eq!(out.action, SyncAction::Merged);
    assert!(!out.changes.is_empty());
    assert!(out.pushed.is_none());
    assert_eq!(names(&s.b.folder), ["beta.txt"]);
    assert_eq!(s.b.repo.read_ref(REF).unwrap(), before);
    assert_eq!(remote_ref(&s.remote), before);
}

#[test]
fn send_and_receive_go_one_way() {
    let mut s = setup();
    let store = FolderStore::open(&s.remote).unwrap();
    let mut receive = options(&s.b);
    receive.mode = SyncMode::Receive;
    write(&s.b.folder, "beta.txt", "beta");
    let out = round_with(&mut s.b, &store, receive.clone());
    assert_eq!(out.action, SyncAction::Merged);
    assert!(out.pushed.is_none());
    assert_eq!(names(&s.b.folder), ["beta.txt", "notes.txt", "sub/plan.md"]);
    assert_eq!(
        round_with(&mut s.b, &store, receive).action,
        SyncAction::UpToDate
    );

    // Send refuses once the remote has something this device lacks.
    write(&s.a.folder, "alpha.txt", "alpha");
    let mut send = options(&s.a);
    send.mode = SyncMode::Send;
    assert_eq!(
        round_with(&mut s.a, &store, send.clone()).action,
        SyncAction::Sent
    );
    write(&s.b.folder, "beta.txt", "beta, changed");
    let mut send_b = options(&s.b);
    send_b.mode = SyncMode::Send;
    let err = sync(&mut s.b.repo, &store, &send_b, None, &Control::new()).unwrap_err();
    assert!(matches!(
        err,
        apeiron_services::Error::Core(apeiron_core::Error::RemoteAhead(_))
    ));
    assert!(!s.b.folder.join("alpha.txt").exists());
    drop(store);
}

/// Pushes another device's snapshot just before this device's push.
struct Racing {
    inner: FolderStore,
    other: RefCell<Option<Repository>>,
}

impl Store for Racing {
    fn location(&self, path: &str) -> PathBuf {
        self.inner.location(path)
    }
    fn read(&self, path: &str) -> apeiron_core::Result<Option<Vec<u8>>> {
        self.inner.read(path)
    }
    fn read_range(&self, path: &str, offset: u64, len: u64) -> apeiron_core::Result<Vec<u8>> {
        self.inner.read_range(path, offset, len)
    }
    fn list(&self, dir: &str) -> apeiron_core::Result<Vec<String>> {
        self.inner.list(dir)
    }
    fn write_new(&self, path: &str, bytes: &[u8]) -> apeiron_core::Result<()> {
        self.inner.write_new(path, bytes)
    }
    fn create_dirs(&self, dirs: &[&str]) -> apeiron_core::Result<()> {
        self.inner.create_dirs(dirs)
    }
    fn delete(&self, path: &str) -> apeiron_core::Result<()> {
        self.inner.delete(path)
    }
    fn read_ref(&self, name: &str) -> apeiron_core::Result<Option<ObjectId>> {
        self.inner.read_ref(name)
    }
    fn list_refs(&self) -> apeiron_core::Result<Vec<String>> {
        self.inner.list_refs()
    }
    fn swap_ref(
        &self,
        name: &str,
        expected: Option<ObjectId>,
        new: ObjectId,
    ) -> apeiron_core::Result<()> {
        if let Some(other) = self.other.borrow_mut().take() {
            other.push(&self.inner, name, &mut |_| Ok(())).unwrap();
        }
        self.inner.swap_ref(name, expected, new)
    }
}

#[test]
fn a_round_that_loses_a_push_starts_again() {
    let mut s = setup();
    round(&mut s.b, &s.remote);
    // Alpha records a change but does not push it yet.
    write(&s.a.folder, "alpha.txt", "alpha");
    let mut receive = options(&s.a);
    receive.mode = SyncMode::Receive;
    round_with(&mut s.a, &FolderStore::open(&s.remote).unwrap(), receive);

    write(&s.b.folder, "beta.txt", "beta");
    let racing = Racing {
        inner: FolderStore::open(&s.remote).unwrap(),
        other: RefCell::new(Some(Repository::open(s.a.repo.path()).unwrap())),
    };
    let opts = options(&s.b);
    let out = round_with(&mut s.b, &racing, opts);
    assert_eq!(out.attempts, 2);
    assert_eq!(out.action, SyncAction::Merged);
    assert_eq!(
        names(&s.b.folder),
        ["alpha.txt", "beta.txt", "notes.txt", "sub/plan.md"]
    );
    assert_eq!(remote_ref(&s.remote), out.snapshot);
}

#[test]
fn a_round_interrupted_while_changing_the_folder_is_finished_by_the_next() {
    let mut s = setup();
    round(&mut s.b, &s.remote);
    write(&s.a.folder, "notes.txt", "notes from alpha");
    write(&s.a.folder, "new.txt", "new");
    round(&mut s.a, &s.remote);
    // Beta stopped after writing one of the two files: its folder ref still
    // points to the old snapshot.
    let before =
        s.b.repo
            .read_ref(&apeiron_services::folder_ref(REF))
            .unwrap();
    write(&s.b.folder, "new.txt", "new");

    let out = round(&mut s.b, &s.remote);
    assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
    assert_ne!(
        s.b.repo
            .read_ref(&apeiron_services::folder_ref(REF))
            .unwrap(),
        before
    );
    assert_eq!(files(&s.b.folder), files(&s.a.folder));
    round(&mut s.a, &s.remote);
    assert_eq!(files(&s.b.folder), files(&s.a.folder));
}
