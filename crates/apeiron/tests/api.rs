//! The public API used the way an application would use it.

use std::fs;
use std::path::PathBuf;

use apeiron::{
    BackupRequest, ChangeKind, DeleteRefRequest, DiffRequest, ErrorKind, ListRequest, LogRequest,
    Phase, Repository, RestoreRequest,
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
