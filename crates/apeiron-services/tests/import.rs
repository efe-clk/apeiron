//! Bringing tar and zip archives in as snapshots (CLI.md §4.2).

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use apeiron_core::Repository;
use apeiron_services::{
    ArchiveFormat, BackupOptions, Control, Error, ImportFormat, ImportOptions, WarningKind, backup,
    export, import, restore, split_archive,
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
    fn import(&mut self, archive: &Path) -> Result<apeiron_services::ImportOutcome, Error> {
        import(
            &mut self.repo,
            &ImportOptions::new(archive, "imported"),
            &Control::new(),
        )
    }

    /// Restores the ref into a new folder.
    fn restored(&self, name: &str) -> PathBuf {
        let out = self.root.join(name);
        let id = self.repo.read_ref("imported").unwrap().unwrap();
        restore(&self.repo, &id, b"", &out, &Control::new()).unwrap();
        out
    }
}

fn fill(src: &Path) {
    fs::create_dir_all(src.join("sub/deep")).unwrap();
    fs::create_dir_all(src.join("empty")).unwrap();
    fs::write(src.join("a.txt"), "first").unwrap();
    fs::write(src.join("sub/deep/b.bin"), vec![7u8; 3 << 20]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        fs::set_permissions(src.join("a.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        symlink("a.txt", src.join("link")).unwrap();
    }
}

#[test]
fn what_export_writes_comes_back_the_same() {
    for (format, name) in [
        (ArchiveFormat::Tar, "x.tar"),
        (ArchiveFormat::Zip, "x.zip"),
        (ArchiveFormat::SevenZ, "x.7z"),
    ] {
        let mut s = setup();
        let src = s.root.join("src");
        fill(&src);
        let snapshot = backup(
            &mut s.repo,
            &BackupOptions::new(&src, "main"),
            &Control::new(),
        )
        .unwrap()
        .snapshot;
        let file = s.root.join(name);
        export(&s.repo, &snapshot, b"", &file, format, &Control::new()).unwrap();

        let out = s.import(&file).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.files, 2);
        assert_eq!(out.bytes, 5 + (3 << 20));
        // The content was stored by the backup already.
        assert_eq!(out.new_bytes, 0);

        let back = s.restored("back");
        assert_eq!(fs::read(back.join("a.txt")).unwrap(), b"first");
        assert_eq!(
            fs::read(back.join("sub/deep/b.bin")).unwrap(),
            vec![7u8; 3 << 20]
        );
        assert!(back.join("empty").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(back.join("a.txt"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o640);
            assert_eq!(
                fs::read_link(back.join("link")).unwrap(),
                Path::new("a.txt")
            );
        }
        assert!(s.repo.verify().unwrap().is_ok());
    }
}

#[test]
fn a_compressed_tar_is_read() {
    let mut s = setup();
    let file = s.root.join("x.tar.gz");
    let gz =
        flate2::write::GzEncoder::new(File::create(&file).unwrap(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_size(5);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, "dir/a.txt", &b"hello"[..])
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap();

    let out = s.import(&file).unwrap();
    assert_eq!((out.files, out.dirs), (1, 1));
    let back = s.restored("back");
    assert_eq!(fs::read(back.join("dir/a.txt")).unwrap(), b"hello");
}

/// A tar entry with a name the `tar` crate's safe functions would refuse.
fn raw_entry(tar: &mut Vec<u8>, name: &str, kind: u8, link: &str, content: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name.as_bytes());
    header.as_gnu_mut().unwrap().linkname[..link.len()].copy_from_slice(link.as_bytes());
    header.set_entry_type(tar::EntryType::new(kind));
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.extend_from_slice(header.as_bytes());
    tar.extend_from_slice(content);
    tar.resize(tar.len().next_multiple_of(512), 0);
}

#[test]
fn a_hostile_tar_cannot_climb_out_and_loses_nothing_else() {
    let mut s = setup();
    let mut bytes = Vec::new();
    raw_entry(&mut bytes, "../../evil.txt", b'0', "", b"evil");
    raw_entry(&mut bytes, "a/../../evil2.txt", b'0', "", b"evil");
    raw_entry(&mut bytes, "/abs/ok.txt", b'0', "", b"fine");
    raw_entry(&mut bytes, "good.txt", b'0', "", b"good");
    raw_entry(&mut bytes, "hard.txt", b'1', "good.txt", b"");
    raw_entry(&mut bytes, "dangling.txt", b'1', "nothing.txt", b"");
    raw_entry(&mut bytes, "escape", b'2', "../../etc/passwd", b"");
    raw_entry(&mut bytes, "fifo", b'6', "", b"");
    bytes.extend_from_slice(&[0; 1024]);
    let file = s.root.join("x.tar");
    fs::write(&file, &bytes).unwrap();

    let out = s.import(&file).unwrap();
    assert_eq!(out.files, 3, "{:?}", out.warnings);
    assert_eq!(out.warnings.len(), 4, "{:?}", out.warnings);
    assert!(
        out.warnings
            .iter()
            .all(|w| w.kind == WarningKind::Unsupported)
    );

    let back = s.restored("sub/back");
    assert_eq!(fs::read(back.join("good.txt")).unwrap(), b"good");
    assert_eq!(fs::read(back.join("hard.txt")).unwrap(), b"good");
    assert_eq!(fs::read(back.join("abs/ok.txt")).unwrap(), b"fine");
    assert!(!s.root.join("evil.txt").exists());
    assert!(!s.root.join("sub/evil.txt").exists());
    assert!(!back.join("evil2.txt").exists());
    // A link is kept as a link; nothing follows it here.
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(back.join("escape")).unwrap(),
        Path::new("../../etc/passwd")
    );
}

#[test]
fn a_hostile_zip_cannot_climb_out() {
    let mut s = setup();
    let file = s.root.join("x.zip");
    let mut zip = zip::ZipWriter::new(File::create(&file).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("../evil.txt", options).unwrap();
    zip.write_all(b"evil").unwrap();
    zip.start_file("dir\\..\\..\\evil2.txt", options).unwrap();
    zip.write_all(b"evil").unwrap();
    zip.start_file("good.txt", options).unwrap();
    zip.write_all(b"good").unwrap();
    zip.finish().unwrap();

    let out = s.import(&file).unwrap();
    assert_eq!(out.files, 1, "{:?}", out.warnings);
    assert_eq!(out.warnings.len(), 2);
    let back = s.restored("back");
    assert_eq!(fs::read(back.join("good.txt")).unwrap(), b"good");
    assert!(!s.root.join("evil.txt").exists());
}

#[test]
fn a_later_entry_overrides_an_earlier_one() {
    let mut s = setup();
    let mut bytes = Vec::new();
    raw_entry(&mut bytes, "same.txt", b'0', "", b"old");
    raw_entry(&mut bytes, "same.txt", b'0', "", b"new");
    raw_entry(&mut bytes, "turn", b'0', "", b"a file");
    raw_entry(&mut bytes, "turn/into.txt", b'0', "", b"then a folder");
    bytes.extend_from_slice(&[0; 1024]);
    let file = s.root.join("x.tar");
    fs::write(&file, &bytes).unwrap();
    s.import(&file).unwrap();
    let back = s.restored("back");
    assert_eq!(fs::read(back.join("same.txt")).unwrap(), b"new");
    assert_eq!(
        fs::read(back.join("turn/into.txt")).unwrap(),
        b"then a folder"
    );
}

#[test]
fn damage_leaves_the_ref_where_it_was() {
    let mut s = setup();
    let file = s.root.join("x.tar");
    let cut = s.root.join("cut.tar");
    let mut bytes = Vec::new();
    raw_entry(&mut bytes, "a.txt", b'0', "", &vec![1u8; 4096]);
    fs::write(&file, &bytes).unwrap();
    fs::write(&cut, &bytes[..1024]).unwrap();
    assert!(matches!(s.import(&cut), Err(Error::InvalidInput(_))));
    assert_eq!(s.repo.read_ref("imported").unwrap(), None);

    let garbage = s.root.join("garbage.zip");
    fs::write(&garbage, b"this is not a zip file at all").unwrap();
    assert!(matches!(s.import(&garbage), Err(Error::InvalidInput(_))));
    let garbage = s.root.join("garbage.7z");
    fs::write(&garbage, b"this is not a 7z file either").unwrap();
    assert!(matches!(s.import(&garbage), Err(Error::InvalidInput(_))));
    let garbage = s.root.join("garbage.tar");
    fs::write(&garbage, vec![b'x'; 2048]).unwrap();
    assert!(matches!(s.import(&garbage), Err(Error::InvalidInput(_))));
    let empty = s.root.join("empty.tar");
    fs::write(&empty, [0; 1024]).unwrap();
    assert!(matches!(s.import(&empty), Err(Error::InvalidInput(_))));
    let unknown = s.root.join("x.rar");
    fs::write(&unknown, b"").unwrap();
    assert!(matches!(s.import(&unknown), Err(Error::InvalidInput(_))));
    assert_eq!(s.repo.read_ref("imported").unwrap(), None);
    assert!(s.repo.verify().unwrap().is_ok());
}

#[test]
fn the_format_can_be_named_and_a_stop_moves_nothing() {
    let mut s = setup();
    let mut bytes = Vec::new();
    raw_entry(&mut bytes, "a.txt", b'0', "", b"content");
    bytes.extend_from_slice(&[0; 1024]);
    let file = s.root.join("no-extension");
    fs::write(&file, &bytes).unwrap();
    let mut options = ImportOptions::new(&file, "imported");
    options.format = Some(ImportFormat::Tar);

    let ctl = Control::new();
    ctl.cancel();
    assert!(matches!(
        import(&mut s.repo, &options, &ctl),
        Err(Error::Cancelled)
    ));
    assert_eq!(s.repo.read_ref("imported").unwrap(), None);

    let out = import(&mut s.repo, &options, &Control::new()).unwrap();
    assert_eq!(out.files, 1);
    let snapshot = s.repo.read_snapshot(&out.snapshot).unwrap();
    assert_eq!(snapshot.message.as_deref(), Some("import no-extension"));
    // A second import adds to the history.
    let again = import(&mut s.repo, &options, &Control::new()).unwrap();
    assert_eq!(again.parent, Some(out.snapshot));
}

#[test]
fn a_split_archive_joins_back_and_never_overwrites() {
    let mut s = setup();
    let src = s.root.join("src");
    fill(&src);
    let snapshot = backup(
        &mut s.repo,
        &BackupOptions::new(&src, "main"),
        &Control::new(),
    )
    .unwrap()
    .snapshot;
    let file = s.root.join("x.tar");
    export(
        &s.repo,
        &snapshot,
        b"",
        &file,
        ArchiveFormat::Tar,
        &Control::new(),
    )
    .unwrap();
    let whole = fs::read(&file).unwrap();

    // A part in the way stops it, and the archive stays.
    let in_the_way = s.root.join("x.tar.002");
    fs::write(&in_the_way, b"keep").unwrap();
    assert!(matches!(
        split_archive(&file, 1 << 20, &Control::new()),
        Err(Error::AlreadyExists(_))
    ));
    assert_eq!(fs::read(&file).unwrap(), whole);
    assert_eq!(fs::read(&in_the_way).unwrap(), b"keep");
    assert!(!s.root.join("x.tar.001").exists());
    fs::remove_file(&in_the_way).unwrap();

    let parts = split_archive(&file, 1 << 20, &Control::new()).unwrap();
    assert!(parts.len() > 1);
    assert!(!file.exists());
    assert_eq!(fs::metadata(&parts[0]).unwrap().len(), 1 << 20);
    let mut joined = Vec::new();
    for part in &parts {
        joined.extend(fs::read(part).unwrap());
    }
    assert_eq!(joined, whole);
    let back = s.root.join("joined.tar");
    fs::write(&back, joined).unwrap();
    assert_eq!(s.import(&back).unwrap().files, 2);
    assert!(split_archive(&back, 0, &Control::new()).is_err());
}
