//! Mounts a repository through FUSE and reads it like any folder. Skipped
//! where FUSE is missing, as in many containers.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;

use apeiron::{BackupRequest, Repository};

fn fuse_here() -> bool {
    Path::new("/dev/fuse").exists()
        && ["/usr/bin/fusermount3", "/bin/fusermount3"]
            .iter()
            .any(|p| Path::new(p).exists())
}

#[test]
fn snapshots_read_like_folders() {
    if !fuse_here() {
        eprintln!("skipped: no /dev/fuse or fusermount3");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.txt"), "one").unwrap();
    let big: Vec<u8> = (0..3_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    fs::write(src.join("sub/big.bin"), &big).unwrap();
    let repo = Repository::init(dir.path().join("repo")).unwrap();
    repo.backup(BackupRequest::new(&src, "backup/docs"))
        .unwrap()
        .wait()
        .unwrap();
    fs::write(src.join("a.txt"), "two").unwrap();
    repo.backup(BackupRequest::new(&src, "backup/docs"))
        .unwrap()
        .wait()
        .unwrap();

    let at = dir.path().join("mnt");
    fs::create_dir(&at).unwrap();
    let mounted = match apeiron_mount::mount(repo, &at) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("skipped: cannot mount here: {e}");
            return;
        }
    };
    let docs = at.join("backup/docs");
    let mut names: Vec<String> = fs::read_dir(&docs)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names.len(), 3, "{names:?}");
    assert_eq!(names[2], "latest");
    assert_eq!(
        fs::read_to_string(docs.join("latest/a.txt")).unwrap(),
        "two"
    );
    let contents: Vec<String> = names[..2]
        .iter()
        .map(|n| fs::read_to_string(docs.join(n).join("a.txt")).unwrap())
        .collect();
    assert!(contents.contains(&"one".to_owned()), "{contents:?}");
    assert_eq!(fs::read(docs.join("latest/sub/big.bin")).unwrap(), big);
    assert!(fs::write(docs.join("latest/new.txt"), "x").is_err());
    mounted.unmount().unwrap();
    assert!(fs::read_dir(&at).unwrap().next().is_none());
}
