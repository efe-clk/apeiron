//! Backing up real folders and restoring them.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use apeiron_core::Repository;
use apeiron_services::{
    BackupOptions, ChangeKind, Control, Error, Kind, Name, WarningKind, backup, diff, list, restore,
};

fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    src: PathBuf,
    repo: Repository,
}

impl Setup {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let repo = Repository::init(&root.join("repo")).unwrap();
        fs::create_dir(root.join("src")).unwrap();
        Setup {
            _dir: dir,
            src: root.join("src"),
            root,
            repo,
        }
    }

    fn backup(&mut self) -> apeiron_services::BackupOutcome {
        let options = BackupOptions::new(self.src.clone(), "main");
        backup(&mut self.repo, &options, &Control::new()).unwrap()
    }
}

/// Read-only folders made by the tests would stop the temporary folder from being removed.
impl Drop for Setup {
    fn drop(&mut self) {
        #[cfg(unix)]
        make_writable(&self.root);
    }
}

#[cfg(unix)]
fn make_writable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            make_writable(&entry.path());
        }
    }
}

/// A folder with nested folders, empty and large files and fixed times.
fn fill(src: &Path) {
    fs::create_dir_all(src.join("docs/deep")).unwrap();
    fs::create_dir(src.join("empty-dir")).unwrap();
    fs::write(src.join("empty.txt"), b"").unwrap();
    fs::write(src.join("small.txt"), b"hello").unwrap();
    fs::write(src.join("docs/big.bin"), noise(3 * 1024 * 1024 + 17, 1)).unwrap();
    fs::write(src.join("docs/deep/note.md"), "çğüşöı").unwrap();
    let time = SystemTime::UNIX_EPOCH + Duration::new(1_600_000_000, 123_456_789);
    for file in [
        "empty.txt",
        "small.txt",
        "docs/big.bin",
        "docs/deep/note.md",
    ] {
        fs::File::options()
            .write(true)
            .open(src.join(file))
            .unwrap()
            .set_modified(time)
            .unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::os::unix::fs::symlink("docs/deep/note.md", src.join("link")).unwrap();
        fs::set_permissions(src.join("small.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        fs::set_permissions(src.join("docs/deep"), fs::Permissions::from_mode(0o555)).unwrap();
    }
}

/// Asserts that two folders hold the same entries, contents and attributes.
fn assert_same(a: &Path, b: &Path) {
    let names = |dir: &Path| {
        let mut v: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    };
    let (left, right) = (names(a), names(b));
    assert_eq!(left, right, "{} vs {}", a.display(), b.display());
    for name in left {
        let (x, y) = (a.join(&name), b.join(&name));
        let (mx, my) = (
            fs::symlink_metadata(&x).unwrap(),
            fs::symlink_metadata(&y).unwrap(),
        );
        assert_eq!(mx.file_type(), my.file_type(), "{}", x.display());
        if mx.is_dir() {
            assert_same(&x, &y);
        } else if mx.is_file() {
            assert_eq!(
                fs::read(&x).unwrap(),
                fs::read(&y).unwrap(),
                "{}",
                x.display()
            );
            assert_eq!(
                mx.modified().unwrap(),
                my.modified().unwrap(),
                "{}",
                x.display()
            );
        } else {
            assert_eq!(fs::read_link(&x).unwrap(), fs::read_link(&y).unwrap());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                mx.permissions().mode(),
                my.permissions().mode(),
                "{}",
                x.display()
            );
        }
    }
}

#[test]
fn backup_and_restore_round_trip() {
    let mut s = Setup::new();
    fill(&s.src.clone());
    let out = s.backup();
    assert_eq!(out.parent, None);
    assert_eq!((out.files, out.dirs), (4, 4));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(s.repo.read_ref("main").unwrap(), Some(out.snapshot));

    let ctl = Control::new();
    let target = s.root.join("restored");
    let restored = restore(&s.repo, &out.snapshot, b"", &target, &ctl).unwrap();
    assert!(restored.warnings.is_empty(), "{:?}", restored.warnings);
    assert_eq!(restored.files, 4);
    assert_same(&s.src.clone(), &target);
    let progress = ctl.progress();
    assert_eq!(progress.items_done, 4);
    assert_eq!(Some(progress.bytes_done), progress.bytes_total);
}

#[test]
fn second_backup_deduplicates_and_diff_lists_changes() {
    let mut s = Setup::new();
    fill(&s.src.clone());
    let first = s.backup();
    fs::write(s.src.clone().join("small.txt"), b"changed").unwrap();
    fs::write(s.src.clone().join("new.txt"), b"new").unwrap();
    fs::remove_dir(s.src.clone().join("empty-dir")).unwrap();
    let second = s.backup();

    assert_eq!(second.parent, Some(first.snapshot));
    assert!(
        second.new_bytes < 4096,
        "stored {} new bytes",
        second.new_bytes
    );
    let changes: Vec<_> = diff(&s.repo, &first.snapshot, &second.snapshot)
        .unwrap()
        .into_iter()
        .map(|c| (c.path.to_string(), c.kind))
        .collect();
    assert_eq!(
        changes,
        [
            ("empty-dir".to_owned(), ChangeKind::Removed),
            ("new.txt".to_owned(), ChangeKind::Added),
            ("small.txt".to_owned(), ChangeKind::Modified),
        ]
    );
    assert!(
        diff(&s.repo, &second.snapshot, &second.snapshot)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn list_shows_folders_and_single_files() {
    let mut s = Setup::new();
    fill(&s.src.clone());
    let out = s.backup();
    let root: Vec<_> = list(&s.repo, &out.snapshot, b"")
        .unwrap()
        .into_iter()
        .map(|e| (e.name.to_string(), e.kind))
        .collect();
    assert!(root.contains(&("docs".to_owned(), Kind::Dir)));
    assert!(root.contains(&("small.txt".to_owned(), Kind::File)));

    let file = list(&s.repo, &out.snapshot, b"/docs/big.bin/").unwrap();
    assert_eq!(file.len(), 1);
    assert_eq!(file[0].size, Some(3 * 1024 * 1024 + 17));
    assert!(matches!(
        list(&s.repo, &out.snapshot, b"docs/missing"),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        list(&s.repo, &out.snapshot, b"small.txt/x"),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn restore_refuses_non_empty_target_and_restores_one_file() {
    let mut s = Setup::new();
    fill(&s.src.clone());
    let out = s.backup();
    let busy = s.root.join("busy");
    fs::create_dir(&busy).unwrap();
    fs::write(busy.join("keep"), b"mine").unwrap();
    assert!(matches!(
        restore(&s.repo, &out.snapshot, b"", &busy, &Control::new()),
        Err(Error::AlreadyExists(_))
    ));
    assert_eq!(fs::read(busy.join("keep")).unwrap(), b"mine");

    let one = s.root.join("one");
    restore(
        &s.repo,
        &out.snapshot,
        b"docs/deep/note.md",
        &one,
        &Control::new(),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(one.join("note.md")).unwrap(), "çğüşöı");
}

#[test]
fn cancelled_backup_leaves_ref_unchanged() {
    let mut s = Setup::new();
    fill(&s.src.clone());
    let first = s.backup();
    fs::write(s.src.clone().join("more.txt"), b"more").unwrap();
    let ctl = Control::new();
    ctl.cancel();
    let result = backup(
        &mut s.repo,
        &BackupOptions::new(s.src.clone(), "main"),
        &ctl,
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(s.repo.read_ref("main").unwrap(), Some(first.snapshot));
}

#[test]
fn cancelled_restore_removes_partial_file() {
    let mut s = Setup::new();
    fs::write(s.src.clone().join("big.bin"), noise(4 * 1024 * 1024, 2)).unwrap();
    let out = s.backup();
    let ctl = Control::new();
    ctl.cancel();
    let target = s.root.join("t");
    assert!(matches!(
        restore(&s.repo, &out.snapshot, b"", &target, &ctl),
        Err(Error::Cancelled)
    ));
    assert!(!target.join("big.bin").exists());
}

#[test]
fn source_must_be_a_folder_and_ref_name_valid() {
    let mut s = Setup::new();
    fs::write(s.root.join("file"), b"x").unwrap();
    let ctl = Control::new();
    assert!(matches!(
        backup(
            &mut s.repo,
            &BackupOptions::new(s.root.join("file"), "main"),
            &ctl
        ),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        backup(
            &mut s.repo,
            &BackupOptions::new(s.src.clone(), "Main"),
            &ctl
        ),
        Err(Error::Core(apeiron_core::Error::InvalidRefName(_)))
    ));
    assert!(matches!(
        backup(
            &mut s.repo,
            &BackupOptions::new(s.root.join("missing"), "main"),
            &ctl
        ),
        Err(Error::Io { .. })
    ));
}

#[cfg(unix)]
#[test]
fn unix_special_cases_become_warnings_or_round_trip() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;

    let mut s = Setup::new();
    let odd = std::ffi::OsStr::from_bytes(b"bad\xffname");
    fs::write(s.src.clone().join(odd), b"bytes").unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(s.src.clone().join("sock")).unwrap();
    let secret = s.src.clone().join("secret");
    fs::write(&secret, b"x").unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    // Running as root can read anything; then the file is simply stored.
    let readable = fs::read(&secret).is_ok();

    let out = s.backup();
    let kinds: Vec<_> = out
        .warnings
        .iter()
        .map(|w| (w.path.to_string(), w.kind))
        .collect();
    assert!(kinds.contains(&("sock".to_owned(), WarningKind::Unsupported)));
    assert_eq!(
        kinds.contains(&("secret".to_owned(), WarningKind::Unreadable)),
        !readable
    );

    let names: Vec<Name> = list(&s.repo, &out.snapshot, b"")
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.contains(&Name(b"bad\xffname".to_vec())));
    let json = serde_json::to_string(&Name(b"bad\xffname".to_vec())).unwrap();
    assert_eq!(json, r#"{"hex":"626164ff6e616d65"}"#);
    assert_eq!(
        serde_json::from_str::<Name>(&json).unwrap(),
        Name(b"bad\xffname".to_vec())
    );
    assert_eq!(serde_json::to_string(&Name::from("ok")).unwrap(), r#""ok""#);

    let target = s.root.join("restored");
    restore(&s.repo, &out.snapshot, b"", &target, &Control::new()).unwrap();
    assert_eq!(fs::read(target.join(odd)).unwrap(), b"bytes");
}
