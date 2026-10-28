//! Exporting snapshots into tar, zip and 7z archives and reading them back.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use apeiron_core::{ObjectId, Repository};
use apeiron_services::{ArchiveFormat, BackupOptions, Control, Error, backup, export};

/// Contents of an archive: name → file content, or `None` for a folder.
type Listing = BTreeMap<String, Option<Vec<u8>>>;

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: Repository,
    snapshot: ObjectId,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(src.join("docs/deep")).unwrap();
    fs::create_dir(src.join("empty-dir")).unwrap();
    fs::write(src.join("empty.txt"), b"").unwrap();
    fs::write(src.join("small.txt"), b"hello").unwrap();
    let big: Vec<u8> = (0..2_500_000u32).map(|i| (i * 7 % 251) as u8).collect();
    fs::write(src.join("docs/big.bin"), big).unwrap();
    fs::write(src.join("docs/deep/note.md"), "çğüşöı").unwrap();
    let mut repo = Repository::init(&root.join("repo")).unwrap();
    let out = backup(
        &mut repo,
        &BackupOptions::new(&src, "main"),
        &Control::new(),
    )
    .unwrap();
    Setup {
        _dir: dir,
        root,
        repo,
        snapshot: out.snapshot,
    }
}

fn expected() -> Listing {
    let big: Vec<u8> = (0..2_500_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let mut m = Listing::new();
    m.insert("docs".into(), None);
    m.insert("docs/big.bin".into(), Some(big));
    m.insert("docs/deep".into(), None);
    m.insert(
        "docs/deep/note.md".into(),
        Some("çğüşöı".as_bytes().to_vec()),
    );
    m.insert("empty-dir".into(), None);
    m.insert("empty.txt".into(), Some(Vec::new()));
    m.insert("small.txt".into(), Some(b"hello".to_vec()));
    m
}

fn read_tar(path: &Path) -> Listing {
    let mut archive = tar::Archive::new(fs::File::open(path).unwrap());
    let mut out = Listing::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry
            .path()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches('/')
            .to_owned();
        if entry.header().entry_type().is_dir() {
            out.insert(name, None);
        } else {
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            out.insert(name, Some(data));
        }
    }
    out
}

fn read_zip(path: &Path) -> Listing {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut out = Listing::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let name = file.name().trim_end_matches('/').to_owned();
        if file.is_dir() {
            out.insert(name, None);
        } else {
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            out.insert(name, Some(data));
        }
    }
    out
}

fn read_7z(path: &Path) -> Listing {
    let mut reader =
        sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).unwrap();
    let mut out = Listing::new();
    reader
        .for_each_entries(|entry, content| {
            if entry.is_directory() {
                out.insert(entry.name().to_owned(), None);
            } else {
                let mut data = Vec::new();
                content.read_to_end(&mut data)?;
                out.insert(entry.name().to_owned(), Some(data));
            }
            Ok(true)
        })
        .unwrap();
    out
}

fn export_to(s: &Setup, name: &str, format: ArchiveFormat, path: &[u8]) -> PathBuf {
    let output = s.root.join(name);
    let out = export(&s.repo, &s.snapshot, path, &output, format, &Control::new()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    output
}

#[test]
fn every_format_round_trips() {
    let s = setup();
    assert_eq!(
        read_tar(&export_to(&s, "a.tar", ArchiveFormat::Tar, b"")),
        expected()
    );
    assert_eq!(
        read_zip(&export_to(&s, "a.zip", ArchiveFormat::Zip, b"")),
        expected()
    );
    assert_eq!(
        read_7z(&export_to(&s, "a.7z", ArchiveFormat::SevenZ, b"")),
        expected()
    );
}

#[test]
fn exporting_a_folder_or_file_puts_it_at_the_root() {
    let s = setup();
    let docs = read_zip(&export_to(&s, "docs.zip", ArchiveFormat::Zip, b"docs/deep"));
    assert_eq!(docs.keys().collect::<Vec<_>>(), ["note.md"]);
    let one = read_tar(&export_to(&s, "one.tar", ArchiveFormat::Tar, b"small.txt"));
    assert_eq!(one["small.txt"], Some(b"hello".to_vec()));
}

#[test]
fn existing_output_is_never_overwritten_and_cancel_removes_partial_archive() {
    let s = setup();
    let output = s.root.join("taken.zip");
    fs::write(&output, b"mine").unwrap();
    let result = export(
        &s.repo,
        &s.snapshot,
        b"",
        &output,
        ArchiveFormat::Zip,
        &Control::new(),
    );
    assert!(matches!(result, Err(Error::AlreadyExists(_))));
    assert_eq!(fs::read(&output).unwrap(), b"mine");

    let ctl = Control::new();
    ctl.cancel();
    let output = s.root.join("cancelled.7z");
    let result = export(
        &s.repo,
        &s.snapshot,
        b"",
        &output,
        ArchiveFormat::SevenZ,
        &ctl,
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(!output.exists());
}

#[test]
fn format_from_extension() {
    assert_eq!(
        ArchiveFormat::from_path(Path::new("x.TAR")),
        Some(ArchiveFormat::Tar)
    );
    assert_eq!(
        ArchiveFormat::from_path(Path::new("x.zip")),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(
        ArchiveFormat::from_path(Path::new("x.7z")),
        Some(ArchiveFormat::SevenZ)
    );
    assert_eq!(ArchiveFormat::from_path(Path::new("x.rar")), None);
    assert_eq!(ArchiveFormat::from_path(Path::new("x")), None);
}

#[cfg(unix)]
#[test]
fn symlinks_and_permissions_are_kept() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("run.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(src.join("run.sh"), fs::Permissions::from_mode(0o750)).unwrap();
    std::os::unix::fs::symlink("run.sh", src.join("link")).unwrap();
    let mut repo = Repository::init(&dir.path().join("repo")).unwrap();
    let snap = backup(
        &mut repo,
        &BackupOptions::new(&src, "main"),
        &Control::new(),
    )
    .unwrap()
    .snapshot;

    let tar_path = dir.path().join("a.tar");
    export(
        &repo,
        &snap,
        b"",
        &tar_path,
        ArchiveFormat::Tar,
        &Control::new(),
    )
    .unwrap();
    let mut archive = tar::Archive::new(fs::File::open(&tar_path).unwrap());
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        let name = entry.path().unwrap().to_string_lossy().into_owned();
        match name.as_str() {
            "run.sh" => assert_eq!(entry.header().mode().unwrap(), 0o750),
            "link" => assert_eq!(
                entry.link_name().unwrap().unwrap().to_string_lossy(),
                "run.sh"
            ),
            other => panic!("unexpected {other}"),
        }
    }

    let zip_path = dir.path().join("a.zip");
    export(
        &repo,
        &snap,
        b"",
        &zip_path,
        ArchiveFormat::Zip,
        &Control::new(),
    )
    .unwrap();
    let mut archive = zip::ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
    assert_eq!(
        archive.by_name("run.sh").unwrap().unix_mode().unwrap() & 0o7777,
        0o750
    );
    assert!(archive.by_name("link").unwrap().is_symlink());
}

#[cfg(target_os = "linux")]
#[test]
fn names_zip_cannot_hold_become_warnings() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join(std::ffi::OsStr::from_bytes(b"bad\xff")), b"x").unwrap();
    fs::write(src.join("good"), b"y").unwrap();
    let mut repo = Repository::init(&dir.path().join("repo")).unwrap();
    let snap = backup(
        &mut repo,
        &BackupOptions::new(&src, "main"),
        &Control::new(),
    )
    .unwrap()
    .snapshot;

    let zip_path = dir.path().join("a.zip");
    let out = export(
        &repo,
        &snap,
        b"",
        &zip_path,
        ArchiveFormat::Zip,
        &Control::new(),
    )
    .unwrap();
    assert_eq!((out.files, out.warnings.len()), (1, 1));
    // tar keeps any bytes.
    let tar_path = dir.path().join("a.tar");
    let out = export(
        &repo,
        &snap,
        b"",
        &tar_path,
        ArchiveFormat::Tar,
        &Control::new(),
    )
    .unwrap();
    assert_eq!((out.files, out.warnings.len()), (2, 0));
}
