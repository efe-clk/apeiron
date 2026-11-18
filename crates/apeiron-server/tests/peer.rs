//! Two devices sync a folder without a server between them (SYNC.md §7):
//! one offers its repository with `serve_peer`, the other clones it while
//! pairing is on and then syncs with it.

use std::fs;

use apeiron::{CloneRequest, Repository, SyncRequest};
use apeiron_server::{PeerIdentity, PeerOptions, serve_peer};

const PASSWORD: &str = "correct horse battery";

#[test]
fn two_devices_sync_without_a_server() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (a, _) = Repository::init_encrypted(root.join("a"), PASSWORD).unwrap();
    let identity = PeerIdentity::for_repo(&root.join("state"), &root.join("a")).unwrap();
    let fingerprint = identity.fingerprint().unwrap();
    // The same identity is found again, so the address stays the same.
    let again = PeerIdentity::for_repo(&root.join("state"), &root.join("a")).unwrap();
    assert_eq!(again.fingerprint().unwrap(), fingerprint);

    let start = |pair: bool| {
        serve_peer(PeerOptions {
            repo: root.join("a"),
            listen: "127.0.0.1:0".parse().unwrap(),
            identity: identity.clone(),
            token: a.peer_token(&fingerprint).unwrap(),
            pair,
        })
        .unwrap()
    };
    let server = start(true);
    let url = format!("peer://127.0.0.1:{}#{fingerprint}", server.address().port());

    // B copies A's repository while pairing is on.
    let clone = |to: &str, url: &str| {
        Repository::clone_from(CloneRequest {
            url: url.to_owned(),
            path: root.join(to),
            secret: Some(PASSWORD.to_owned()),
            rate: None,
        })
        .unwrap()
        .wait()
    };
    clone("b", &url).unwrap();
    let b = Repository::unlock(root.join("b"), PASSWORD).unwrap();
    assert_eq!(b.remotes().unwrap()[0].url, url);

    // B syncs a folder into A's repository: writing needs B to hold the key.
    fs::create_dir_all(root.join("docs-b")).unwrap();
    fs::write(root.join("docs-b/two.txt"), "from b").unwrap();
    let mut sync = SyncRequest::new(root.join("docs-b"), "origin");
    sync.ref_name = Some("sync/docs".to_owned());
    b.sync(sync.clone()).unwrap().wait().unwrap();
    let a = Repository::unlock(root.join("a"), PASSWORD).unwrap();
    assert_eq!(
        a.resolve("sync/docs").unwrap(),
        b.resolve("sync/docs").unwrap()
    );

    // A's repository now holds B's file.
    let listed = a
        .list(&apeiron::ListRequest {
            snapshot: "sync/docs".to_owned(),
            path: apeiron::Name::default(),
        })
        .unwrap();
    assert_eq!(listed.len(), 1);

    // Without pairing, a device without the key reads nothing.
    drop(server);
    let closed = start(false);
    let url = format!("peer://127.0.0.1:{}#{fingerprint}", closed.address().port());
    let refused = clone("c", &url).unwrap_err();
    assert!(refused.message.contains("key"), "{}", refused.message);
    // A copy with the key still syncs.
    sync.remote = "closed".to_owned();
    b.add_remote("closed", &url).unwrap();
    fs::write(root.join("docs-b/three.txt"), "later").unwrap();
    b.sync(sync).unwrap().wait().unwrap();

    // A certificate that is not the one the address names is refused.
    let wrong = format!(
        "peer://127.0.0.1:{}#{}",
        closed.address().port(),
        "0".repeat(64)
    );
    let refused = clone("d", &wrong).unwrap_err();
    assert!(
        refused.message.contains("certificate"),
        "{}",
        refused.message
    );
}
