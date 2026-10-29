//! The public API used the way an application would use it.

use std::fs;
use std::path::PathBuf;

use apeiron::{
    BackupRequest, ChangeKind, DeleteRefRequest, DiffRequest, ErrorKind, KeyKind, ListRequest,
    LogRequest, Phase, Repository, RestoreRequest, TransferRequest,
};

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: Repository,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let repo = Repository::init(root.join("repo")).unwrap();
    fs::create_dir_all(root.join("src/sub")).unwrap();
    fs::write(root.join("src/a.txt"), b"first").unwrap();
    fs::write(root.join("src/sub/b.txt"), b"second").unwrap();
    Setup {
        _dir: dir,
        root,
        repo,
    }
}

fn backup(s: &Setup, message: &str) -> apeiron::BackupReport {
    let mut request = BackupRequest::new(s.root.join("src"), "main");
    request.message = Some(message.to_owned());
    let job = s.repo.backup(request).unwrap();
    let report = job.wait().unwrap();
    assert!(report.warnings.is_empty());
    report
}

#[test]
fn backup_log_list_diff_restore_verify() {
    let s = setup();
    let first = backup(&s, "one");
    fs::write(s.root.join("src/a.txt"), b"changed").unwrap();
    let second = backup(&s, "two");
    assert_eq!(second.parent, Some(first.snapshot));

    let page = s
        .repo
        .log(&LogRequest {
            start: "main".into(),
            limit: 1,
        })
        .unwrap();
    assert_eq!(page.snapshots[0].id, second.snapshot);
    assert_eq!(page.snapshots[0].message.as_deref(), Some("two"));
    assert_eq!(page.next, Some(first.snapshot));
    let all = s
        .repo
        .log(&LogRequest {
            start: "main".into(),
            limit: 10,
        })
        .unwrap();
    assert_eq!(all.snapshots.len(), 2);
    assert_eq!(all.next, None);

    let entries = s
        .repo
        .list(&ListRequest {
            snapshot: "main~1".into(),
            path: "sub".into(),
        })
        .unwrap();
    assert_eq!(entries[0].name.to_string(), "b.txt");

    let changes = s
        .repo
        .diff(&DiffRequest {
            from: "main~1".into(),
            to: "main".into(),
        })
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].path.to_string(), changes[0].kind),
        ("a.txt".into(), ChangeKind::Modified)
    );

    let target = s.root.join("restored");
    let job = s
        .repo
        .restore(RestoreRequest {
            snapshot: "main~1".into(),
            target: target.clone(),
            path: Default::default(),
        })
        .unwrap();
    let report = job.wait().unwrap();
    assert_eq!(report.files, 2);
    assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"first");

    let job = s
        .repo
        .export(apeiron::ExportRequest {
            snapshot: "main".into(),
            output: s.root.join("out.7z"),
            format: None,
            path: Default::default(),
        })
        .unwrap();
    assert_eq!(job.wait().unwrap().files, 2);
    let err = s
        .repo
        .export(apeiron::ExportRequest {
            snapshot: "main".into(),
            output: s.root.join("out.rar"),
            format: None,
            path: Default::default(),
        })
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);

    let job = s.repo.verify().unwrap();
    assert!(job.wait().unwrap().is_ok());

    let reopened = Repository::open(s.repo.path()).unwrap();
    assert_eq!(reopened.refs().unwrap()[0].snapshot, second.snapshot);
    assert_eq!(reopened.reflog("main").unwrap().len(), 2);
}

#[test]
fn snapshot_specs() {
    let s = setup();
    let first = backup(&s, "one");
    let hex = first.snapshot.to_hex();
    assert_eq!(s.repo.resolve("main").unwrap(), first.snapshot);
    assert_eq!(s.repo.resolve(&hex).unwrap(), first.snapshot);
    assert_eq!(s.repo.resolve(&hex[..8]).unwrap(), first.snapshot);
    assert_eq!(
        s.repo.resolve(&format!("{}~0", &hex[..10])).unwrap(),
        first.snapshot
    );

    let kind = |spec: &str| s.repo.resolve(spec).unwrap_err().kind;
    assert_eq!(kind("main~1"), ErrorKind::NotFound);
    assert_eq!(kind("other"), ErrorKind::NotFound);
    assert_eq!(kind("Main!"), ErrorKind::InvalidInput);
    assert_eq!(kind("main~x"), ErrorKind::InvalidInput);
    let wrong_prefix = if hex.starts_with('0') {
        "11111111"
    } else {
        "00000000"
    };
    assert_eq!(kind(wrong_prefix), ErrorKind::NotFound);
}

#[test]
fn progress_reaches_done_and_errors_are_typed() {
    let s = setup();
    let job = s
        .repo
        .backup(BackupRequest::new(s.root.join("src"), "main"))
        .unwrap();
    let mut job = job;
    while !job.is_finished() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let progress = job.progress();
    assert_eq!(progress.phase, Phase::Done);
    assert_eq!(progress.items_total, Some(2));
    let report = job.wait().unwrap();

    let err = s
        .repo
        .restore(RestoreRequest {
            snapshot: "main".into(),
            target: s.root.join("src"),
            path: Default::default(),
        })
        .unwrap()
        .wait()
        .unwrap_err();
    assert_eq!((err.kind, err.code), (ErrorKind::AlreadyExists, 3));
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json["code"], 3);
    assert_eq!(json["kind"], "already_exists");

    let stale = apeiron::ObjectId::from_bytes([1; 32]);
    let err = s
        .repo
        .delete_ref(&DeleteRefRequest {
            name: "main".into(),
            expected: stale,
        })
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Conflict);
    s.repo
        .delete_ref(&DeleteRefRequest {
            name: "main".into(),
            expected: report.snapshot,
        })
        .unwrap();
    assert!(s.repo.refs().unwrap().is_empty());

    // The deleted ref's snapshot stays while its reflog line is recent.
    let keep = s
        .repo
        .gc(apeiron::GcRequest::default())
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(keep.removed_objects, 0);
    {
        let _reader = apeiron_core::RepoLock::shared(s.repo.path()).unwrap();
        let err = s
            .repo
            .gc(apeiron::GcRequest::default())
            .unwrap()
            .wait()
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Locked);
    }
    let request = apeiron::GcRequest {
        keep_reflog_days: 0,
        dry_run: false,
    };
    let gone = s.repo.gc(request).unwrap().wait().unwrap();
    assert!(gone.removed_objects > 0);
    assert_eq!(
        s.repo.resolve(&report.snapshot.to_hex()).unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert!(s.repo.repair().unwrap().removed_locks.is_empty());

    assert_eq!(
        Repository::open(s.root.join("nothing")).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn requests_and_results_round_trip_as_json() {
    let s = setup();
    let report = backup(&s, "one");
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains(&report.snapshot.to_hex()));
    assert_eq!(
        serde_json::from_str::<apeiron::BackupReport>(&json).unwrap(),
        report
    );

    let request: BackupRequest =
        serde_json::from_str(r#"{"source": "/data", "ref": "backup/home"}"#).unwrap();
    assert_eq!(request.ref_name, "backup/home");
    let request: RestoreRequest =
        serde_json::from_str(r#"{"snapshot": "main", "target": "/tmp/x"}"#).unwrap();
    assert!(request.path.as_bytes().is_empty());
}

#[test]
fn encrypted_repository_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/a.txt"), b"private").unwrap();
    let err = Repository::init_encrypted(root.join("empty-pw"), "").unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    let (repo, recovery) = Repository::init_encrypted(root.join("repo"), "pw").unwrap();
    assert!(repo.info().encrypted);
    let job = repo
        .backup(BackupRequest::new(root.join("src"), "main"))
        .unwrap();
    job.wait().unwrap();

    let err = Repository::open(root.join("repo")).unwrap_err();
    assert_eq!((err.kind, err.code), (ErrorKind::Key, 12));
    let err = Repository::unlock(root.join("repo"), "nope").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Key);

    let repo = Repository::unlock(root.join("repo"), &recovery).unwrap();
    let target = root.join("out");
    let job = repo
        .restore(RestoreRequest {
            snapshot: "main".into(),
            target: target.clone(),
            path: Default::default(),
        })
        .unwrap();
    job.wait().unwrap();
    assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"private");
    assert!(repo.verify().unwrap().wait().unwrap().is_ok());
}

#[test]
fn change_the_password_of_an_encrypted_repository() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repo");
    let (repo, _) = Repository::init_encrypted(&path, "old").unwrap();
    let keys = repo.keys().unwrap();
    assert_eq!(keys.len(), 2);
    let old = keys.iter().find(|k| k.current).unwrap();
    assert_eq!(old.kind, KeyKind::Password);

    assert_eq!(
        repo.add_password("").unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let added = repo.add_password("new").unwrap();
    assert_eq!(added.recovery_key, None);
    repo.remove_key(&old.id[..6]).unwrap();
    assert_eq!(
        Repository::unlock(&path, "old").unwrap_err().kind,
        ErrorKind::Key
    );
    let repo = Repository::unlock(&path, "new").unwrap();
    assert!(
        repo.keys()
            .unwrap()
            .iter()
            .any(|k| k.current && k.id == added.id)
    );

    let recovery = repo.add_recovery_key().unwrap();
    Repository::unlock(&path, &recovery.recovery_key.unwrap()).unwrap();
    assert_eq!(repo.remove_key("").unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(repo.remove_key("zz").unwrap_err().kind, ErrorKind::NotFound);

    let plain = Repository::init(dir.path().join("plain")).unwrap();
    assert_eq!(plain.keys().unwrap_err().kind, ErrorKind::InvalidInput);
}

#[test]
fn push_and_pull_through_a_folder_remote() {
    let s = setup();
    let first = backup(&s, "one");
    let disk = s.root.join("disk");
    assert_eq!(
        s.repo.add_remote("Disk", "x").unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        s.repo.add_remote("cloud", "ftp://host/x").unwrap_err().kind,
        ErrorKind::Unsupported
    );
    // Only encrypted data goes over the network.
    s.repo.add_remote("nas", "sftp://nas.invalid/srv").unwrap();
    let err = s
        .repo
        .push(TransferRequest::new("nas", "main"))
        .unwrap()
        .wait()
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    s.repo.remove_remote("nas").unwrap();
    s.repo.add_remote("cloud", "s3://bucket/x").unwrap();
    let err = s
        .repo
        .push(TransferRequest::new("cloud", "main"))
        .unwrap()
        .wait()
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidInput);
    s.repo.remove_remote("cloud").unwrap();
    let remote = s.repo.add_remote("disk", disk.to_str().unwrap()).unwrap();
    assert_eq!(s.repo.remotes().unwrap(), vec![remote]);
    assert_eq!(
        s.repo.add_remote("disk", "y").unwrap_err().kind,
        ErrorKind::AlreadyExists
    );

    let request = || TransferRequest::new("disk", "main");
    let report = s.repo.push(request()).unwrap().wait().unwrap();
    assert_eq!((report.snapshot, report.previous), (first.snapshot, None));

    // A second copy made from the disk pulls what the first one pushes.
    fs::write(s.root.join("src/a.txt"), b"changed").unwrap();
    let second = backup(&s, "two");
    s.repo.push(request()).unwrap().wait().unwrap();
    let copy = s.root.join("copy");
    copy_dir(&disk, &copy);
    let copy = Repository::open(&copy).unwrap();
    copy.add_remote("disk", disk.to_str().unwrap()).unwrap();
    let report = copy.pull(request()).unwrap().wait().unwrap();
    assert_eq!(report.objects, 0);
    assert_eq!(copy.resolve("main").unwrap(), second.snapshot);

    fs::write(s.root.join("src/a.txt"), b"again").unwrap();
    let third = backup(&s, "three");
    s.repo.push(request()).unwrap().wait().unwrap();
    let report = copy.pull(request()).unwrap().wait().unwrap();
    assert_eq!(report.snapshot, third.snapshot);
    assert!(report.objects > 0);
    assert_eq!(copy.resolve("main").unwrap(), third.snapshot);

    let err = copy
        .pull(TransferRequest::new("nowhere", "main"))
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    s.repo.remove_remote("disk").unwrap();
    assert!(s.repo.remotes().unwrap().is_empty());
}

fn copy_dir(src: &std::path::Path, dest: &std::path::Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
        }
    }
}
