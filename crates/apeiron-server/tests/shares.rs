//! The shares file (SHARE.md §3).

use std::fs;
use std::thread;

use apeiron::{ErrorKind, ObjectId};
use apeiron_server::Shares;

fn snapshot() -> ObjectId {
    ObjectId::from_hex(&"b".repeat(64)).unwrap()
}

#[test]
fn writers_at_the_same_time_lose_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state/shares.json");
    let threads: Vec<_> = (0..8)
        .map(|n| {
            let shares = Shares::new(&file);
            thread::spawn(move || {
                (0..5)
                    .map(|_| {
                        shares
                            .add(&format!("u{n}"), snapshot(), "a", 1, None)
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let made: Vec<_> = threads
        .into_iter()
        .flat_map(|t| t.join().unwrap())
        .collect();
    let shares = Shares::new(&file);
    assert_eq!(shares.list().unwrap().len(), 40);
    for (share, secret) in &made {
        assert_eq!(shares.find(secret).unwrap().id, share.id);
    }

    // Only hashes are kept, and only the owner can read the file.
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        made.iter()
            .all(|(_, secret)| !text.contains(secret.as_str()))
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Removing one leaves the rest.
    let (first, secret) = &made[0];
    assert_eq!(shares.remove(&first.id, |_| true).unwrap().id, first.id);
    assert_eq!(shares.find(secret).unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(shares.list().unwrap().len(), 39);
}

#[test]
fn a_damaged_file_is_reported_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shares.json");
    fs::write(&file, "{\"shares\": [").unwrap();
    let shares = Shares::new(&file);
    assert_eq!(shares.list().unwrap_err().kind, ErrorKind::Corrupt);
    assert_eq!(
        shares.add("u", snapshot(), "", 1, None).unwrap_err().kind,
        ErrorKind::Corrupt
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "{\"shares\": [");
}
