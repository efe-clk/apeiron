//! S3 remotes against a real S3-compatible service. `APEIRON_S3_TEST_URL`
//! names an existing bucket, such as
//! `s3://apeiron-test?endpoint=http://127.0.0.1:4566&region=us-east-1`, and
//! the `AWS_*` variables hold its credentials; without it the tests are
//! skipped. CI runs them against LocalStack.

use std::time::{SystemTime, UNIX_EPOCH};

use apeiron_core::{
    ChunkerParams, EntryKind, Error, ObjectId, RemoteUrl, Repository, S3Url, Snapshot, Store, Time,
    Tree, TreeEntry,
};
use apeiron_remote::S3Store;

/// A fresh folder in the test bucket, or `None` to skip.
fn url() -> Option<S3Url> {
    let Some(url) = std::env::var("APEIRON_S3_TEST_URL")
        .ok()
        .filter(|u| !u.is_empty())
    else {
        eprintln!("skipped: APEIRON_S3_TEST_URL is not set");
        return None;
    };
    let RemoteUrl::S3(mut url) = RemoteUrl::parse(&url).unwrap() else {
        panic!("APEIRON_S3_TEST_URL is not an s3:// URL");
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    url.prefix = format!("test-{nanos}");
    Some(url)
}

fn data(seed: u8) -> Vec<u8> {
    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ u64::from(seed);
    (0..300 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn commit(repo: &mut Repository, seed: u8, parent: Option<ObjectId>) -> ObjectId {
    let mut w = repo.writer();
    let (size, blob) = w.put_file(data(seed).as_slice()).unwrap();
    let tree = w
        .put_tree(&Tree {
            entries: vec![TreeEntry {
                name: b"file".to_vec(),
                kind: EntryKind::File { size, blob },
                mode: None,
                mtime: None,
                uid: None,
                gid: None,
                xattrs: vec![],
            }],
        })
        .unwrap();
    let snapshot = w
        .put_snapshot(&Snapshot {
            tree,
            parents: parent.into_iter().collect(),
            time: Time {
                seconds: 1_700_000_000,
                nanos: 0,
            },
            author: None,
            host: None,
            message: None,
            meta: vec![],
        })
        .unwrap();
    w.finish().unwrap();
    repo.update_ref("main", parent, Some(snapshot)).unwrap();
    snapshot
}

fn content(repo: &Repository, snapshot: &ObjectId) -> Vec<u8> {
    let tree = repo
        .read_tree(&repo.read_snapshot(snapshot).unwrap().tree)
        .unwrap();
    let EntryKind::File {
        blob: Some(blob), ..
    } = tree.entries[0].kind
    else {
        panic!("expected a file");
    };
    let mut out = Vec::new();
    repo.read_file(&blob, &mut out).unwrap();
    out
}

fn copy_dir(src: &std::path::Path, dest: &std::path::Path) {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn push_and_pull_through_s3() {
    let Some(url) = url() else { return };
    let dir = tempfile::tempdir().unwrap();
    let params = ChunkerParams::new(64 * 1024).unwrap();
    let (mut a, _) = Repository::init_encrypted(&dir.path().join("a"), params, "pw").unwrap();
    let first = commit(&mut a, 1, None);
    copy_dir(&dir.path().join("a"), &dir.path().join("b"));
    let mut b = Repository::unlock(&dir.path().join("b"), "pw").unwrap();

    // An interrupted push leaves the bucket without the ref, and runs again.
    let store = S3Store::connect(&url).unwrap();
    let result = a.push(&store, "main", &mut |r| {
        if r.objects == 3 {
            return Err(Error::Cancelled);
        }
        Ok(())
    });
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(store.read_ref("main").unwrap(), None);
    let report = a.push(&store, "main", &mut |_| Ok(())).unwrap();
    assert!(report.objects > 5);
    assert_eq!(a.push(&store, "main", &mut |_| Ok(())).unwrap().objects, 0);

    let second = commit(&mut a, 2, Some(first));
    a.push(&store, "main", &mut |_| Ok(())).unwrap();
    let report = b.pull(&store, "main", &mut |_| Ok(())).unwrap();
    assert_eq!((report.snapshot, report.previous), (second, Some(first)));
    assert_eq!(content(&b, &second), data(2));
    assert!(b.verify().unwrap().is_ok());

    // A stale writer cannot move the ref back.
    assert!(matches!(
        store.swap_ref("main", Some(first), first),
        Err(Error::RefConflict { actual: Some(x), .. }) if x == second
    ));
    store.write_new("keys/x.key", b"one").unwrap();
    store.write_new("keys/x.key", b"two").unwrap();
    assert_eq!(store.read("keys/x.key").unwrap().unwrap(), b"one");
}
