//! The audit log's chain under changes, damage and several writers
//! (AUDIT.md §2, §3).

use std::fs;
use std::io::Write;
use std::sync::Arc;
use std::thread;

use apeiron::ErrorKind;
use apeiron_server::{Actor, Audit};
use serde_json::json;

fn actor(name: &str) -> Actor {
    Actor {
        user: Some(name.to_owned()),
        from: None,
    }
}

fn filled(dir: &std::path::Path, n: usize) -> Audit {
    let audit = Audit::new(dir.join("state/audit.log"));
    for i in 0..n {
        audit
            .record(&actor("efe"), "call:list", json!({ "i": i }), "ok")
            .unwrap();
    }
    audit
}

fn lines(audit: &Audit) -> Vec<String> {
    fs::read_to_string(audit.file())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn write_lines(audit: &Audit, lines: &[String]) {
    fs::write(audit.file(), lines.join("\n") + "\n").unwrap();
}

fn broken_at(audit: &Audit) -> String {
    let e = audit.verify().unwrap_err();
    assert_eq!(e.kind, ErrorKind::Corrupt);
    e.message
}

#[test]
fn a_whole_log_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let empty = Audit::new(dir.path().join("none.log"));
    assert_eq!(empty.verify().unwrap().events, 0);
    assert!(empty.read(10, None).unwrap().is_empty());

    let audit = filled(dir.path(), 5);
    let verified = audit.verify().unwrap();
    assert_eq!(verified.events, 5);
    assert_eq!(verified.last.len(), 64);
    let events = audit.read(2, None).unwrap();
    assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), [4, 5]);
    assert!(audit.read(10, Some("someone")).unwrap().is_empty());
    assert!(events[0].time.ends_with('Z'));

    // Long strings are cut.
    audit
        .record(
            &actor("efe"),
            "call:list",
            json!({ "path": "x".repeat(1000) }),
            "ok",
        )
        .unwrap();
    let last = audit.read(1, None).unwrap().remove(0);
    assert_eq!(last.detail["path"].as_str().unwrap().chars().count(), 257);
    assert_eq!(audit.verify().unwrap().events, 6);
}

#[test]
fn changes_removals_and_reordering_are_found() {
    let dir = tempfile::tempdir().unwrap();
    let audit = filled(dir.path(), 5);
    let original = lines(&audit);

    let mut changed = original.clone();
    changed[2] = changed[2].replace("\"i\":2", "\"i\":9");
    write_lines(&audit, &changed);
    assert!(broken_at(&audit).contains("line 4 does not follow"));

    let mut removed = original.clone();
    removed.remove(1);
    write_lines(&audit, &removed);
    assert!(broken_at(&audit).contains("line 2 is out of order"));

    let mut swapped = original.clone();
    swapped.swap(3, 4);
    write_lines(&audit, &swapped);
    assert!(broken_at(&audit).contains("line 4"));

    // Cutting the end is not caught by the chain; the last hash is.
    let before = {
        write_lines(&audit, &original);
        audit.verify().unwrap().last
    };
    write_lines(&audit, &original[..4]);
    assert_ne!(audit.verify().unwrap().last, before);
}

#[test]
fn a_line_cut_short_stays_visible() {
    let dir = tempfile::tempdir().unwrap();
    let audit = filled(dir.path(), 3);
    // A crash in the middle of writing the fourth line.
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(audit.file())
        .unwrap();
    file.write_all(b"{\"seq\":4,\"ti").unwrap();
    drop(file);
    audit
        .record(&actor("efe"), "call:list", json!(null), "ok")
        .unwrap();
    let all = lines(&audit);
    assert_eq!(all.len(), 5);
    assert_eq!(all[3], "{\"seq\":4,\"ti");
    assert!(broken_at(&audit).contains("line 4 is not an event"));
}

#[test]
fn writers_at_the_same_time_keep_one_chain() {
    let dir = tempfile::tempdir().unwrap();
    let audit = Arc::new(Audit::new(dir.path().join("audit.log")));
    let writers: Vec<_> = (0..4)
        .map(|w| {
            let audit = audit.clone();
            thread::spawn(move || {
                for i in 0..25 {
                    audit
                        .record(
                            &actor(&format!("w{w}")),
                            "call:log",
                            json!({ "i": i }),
                            "ok",
                        )
                        .unwrap();
                }
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    assert_eq!(audit.verify().unwrap().events, 100);
}
