//! Runs the `apeiron` program the way a user or a script would.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("src/sub")).unwrap();
        fs::write(root.join("src/a.txt"), b"first").unwrap();
        fs::write(root.join("src/sub/b.txt"), b"second").unwrap();
        let s = Setup { _dir: dir, root };
        s.ok(&["init", s.repo().to_str().unwrap()]);
        s
    }

    fn repo(&self) -> PathBuf {
        self.root.join("repo")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_apeiron"))
            .args(args)
            .env("APEIRON_REPO", self.repo())
            .output()
            .unwrap()
    }

    /// Runs and expects exit code 0; returns standard output.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        serde_json::from_str(&self.ok(&all)).unwrap()
    }

    fn path(&self, rel: &str) -> String {
        self.root.join(rel).to_str().unwrap().to_owned()
    }
}

fn code(out: &Output) -> Option<i32> {
    out.status.code()
}

#[test]
fn backup_log_diff_ls_restore() {
    let s = Setup::new();
    let first = s.json(&["backup", &s.path("src"), "-m", "one"]);
    assert_eq!(first["files"], 2);
    fs::write(s.root.join("src/a.txt"), b"changed").unwrap();
    let text = s.ok(&["backup", &s.path("src"), "-m", "two"]);
    assert!(text.starts_with("Snapshot "), "{text}");

    let log = s.json(&["log"]);
    let snapshots = log["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[1]["id"], first["snapshot"]);
    assert!(s.ok(&["log"]).contains("two"));

    assert_eq!(s.ok(&["diff", "main~1", "main"]), "M a.txt\n");
    let listing = s.ok(&["ls", "main", "sub"]);
    assert!(listing.contains("b.txt"), "{listing}");

    let target = s.path("restored");
    s.ok(&["restore", "main~1", &target]);
    assert_eq!(
        fs::read(Path::new(&target).join("a.txt")).unwrap(),
        b"first"
    );

    let one = s.path("one");
    s.ok(&["restore", "main", &one, "--path", "sub/b.txt"]);
    assert_eq!(fs::read(Path::new(&one).join("b.txt")).unwrap(), b"second");

    let archive = s.path("snap.zip");
    let exported = s.json(&["export", "main", &archive]);
    assert_eq!(exported["files"], 2);
    assert!(Path::new(&archive).is_file());
    let tar = s.path("snap.archive");
    s.ok(&["export", "main", &tar, "--format", "tar"]);
    s.ok(&["export", "main", &s.path("only.7z"), "--path", "sub"]);
    // Unknown extension and no --format: an invalid request.
    assert_eq!(code(&s.run(&["export", "main", &s.path("x.rar")])), Some(1));

    let refs = s.json(&["refs"]);
    assert_eq!(refs[0]["name"], "main");
    assert!(s.ok(&["verify"]).starts_with("OK: "));
    assert_eq!(s.ok(&["repair"]), "Nothing to repair\n");

    // Both snapshots are reachable from main, so nothing is removed.
    let dry = s.json(&["gc", "--keep-days", "0", "--dry-run"]);
    assert_eq!(
        (dry["dry_run"].as_bool(), dry["removed_objects"].as_u64()),
        (Some(true), Some(0))
    );
    assert!(s.ok(&["gc"]).starts_with("Removed 0 objects"));
    assert!(s.ok(&["verify"]).starts_with("OK: "));
    assert_eq!(s.ok(&["diff", "main~1", "main"]), "M a.txt\n");
}

#[test]
fn exit_codes() {
    let s = Setup::new();
    // Usage errors.
    assert_eq!(code(&s.run(&["no-such-command"])), Some(2));
    let out = Command::new(env!("CARGO_BIN_EXE_apeiron"))
        .args(["log"])
        .env_remove("APEIRON_REPO")
        .output()
        .unwrap();
    assert_eq!(code(&out), Some(2));

    // An error from the operation, also as JSON.
    s.ok(&["backup", &s.path("src")]);
    let busy = s.path("src");
    let out = s.run(&["--json", "restore", "main", &busy]);
    assert_eq!(code(&out), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["error"]["kind"], "already_exists");
    assert_eq!(doc["error"]["code"], 3);

    // Damage found by verify.
    let packs = s.repo().join("packs");
    let shard = fs::read_dir(&packs)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let pack = fs::read_dir(&shard)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = fs::read(&pack).unwrap();
    bytes[20] ^= 0xff;
    fs::write(&pack, bytes).unwrap();
    assert_eq!(code(&s.run(&["verify"])), Some(4));
}

#[test]
fn damaged_index_is_reported_and_repaired() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    let index_dir = s.repo().join("index");
    let index = fs::read_dir(&index_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = fs::read(&index).unwrap();
    bytes[30] ^= 0xff;
    fs::write(&index, bytes).unwrap();

    let out = s.run(&["log"]);
    assert_eq!(code(&out), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning: damaged file"));
    assert_eq!(code(&s.run(&["verify"])), Some(4));
    let repaired = s.ok(&["repair"]);
    assert!(
        repaired.contains("wrote a new index for 1 pack"),
        "{repaired}"
    );
    let out = s.run(&["verify"]);
    assert_eq!(code(&out), Some(0));
    assert!(out.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn warnings_give_exit_code_3() {
    let s = Setup::new();
    let _socket = std::os::unix::net::UnixListener::bind(s.root.join("src/sock")).unwrap();
    let out = s.run(&["backup", &s.path("src")]);
    assert_eq!(code(&out), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning: sock"));
}

#[test]
fn encrypted_repository_with_password_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/a.txt"), b"private").unwrap();
    fs::write(root.join("pw"), "pass word\n").unwrap();
    fs::write(root.join("wrong"), "other\n").unwrap();
    let repo = root.join("repo");
    let run = |args: &[&str], password_file: &str| {
        Command::new(env!("CARGO_BIN_EXE_apeiron"))
            .args(args)
            .env("APEIRON_REPO", &repo)
            .env("APEIRON_PASSWORD_FILE", root.join(password_file))
            .output()
            .unwrap()
    };

    let out = run(
        &["--json", "init", "--encrypt", repo.to_str().unwrap()],
        "pw",
    );
    assert_eq!(code(&out), Some(0));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["encrypted"], true);
    let recovery = doc["recovery_key"].as_str().unwrap().to_owned();
    assert_eq!(recovery.len(), 64);

    let src = root.join("src");
    assert_eq!(
        code(&run(&["backup", src.to_str().unwrap()], "pw")),
        Some(0)
    );

    let out = run(&["--json", "ls", "main"], "wrong");
    assert_eq!(code(&out), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["error"]["kind"], "key");

    fs::write(root.join("recovery"), &recovery).unwrap();
    let out = run(&["ls", "main"], "recovery");
    assert_eq!(code(&out), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("a.txt"));

    // Change the password: add the new one, then remove the old one.
    let json = |args: &[&str], password_file: &str| {
        let out = run(&[&["--json"], args].concat(), password_file);
        assert_eq!(
            code(&out),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let keys = json(&["key", "list"], "pw");
    let old = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["current"] == true)
        .unwrap();
    assert_eq!(old["kind"], "password");
    let old = old["id"].as_str().unwrap().to_owned();
    let new_file = root.join("wrong").to_str().unwrap().to_owned();
    json(&["key", "add", "--new-password-file", &new_file], "pw");
    let doc = json(&["key", "remove", &old[..8]], "wrong");
    assert_eq!(doc["removed"], old.as_str());
    assert_eq!(code(&run(&["ls", "main"], "pw")), Some(1));
    let out = run(&["key", "list"], "wrong");
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.contains("(in use)"));

    let doc = json(&["key", "add", "--recovery"], "wrong");
    assert_eq!(doc["recovery_key"].as_str().unwrap().len(), 64);
}

#[test]
fn push_and_pull_with_a_folder_remote() {
    let s = Setup::new();
    let src = s.path("src");
    s.ok(&["backup", &src]);
    let doc = s.json(&["remote", "add", "disk", &s.path("disk")]);
    assert_eq!(doc["url"], s.path("disk").as_str());
    assert!(s.ok(&["remote", "list"]).starts_with("disk  "));

    let doc = s.json(&["push", "disk"]);
    assert!(doc["previous"].is_null());
    assert!(doc["objects"].as_u64().unwrap() > 0);
    assert!(
        s.ok(&["push", "disk"])
            .contains("main is up to date with disk")
    );

    // The disk is a complete repository on its own.
    let disk = Command::new(env!("CARGO_BIN_EXE_apeiron"))
        .args(["--repo", &s.path("disk"), "ls", "main"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&disk.stdout).contains("a.txt"));
    assert!(s.ok(&["pull", "disk"]).contains("up to date"));

    let out = s.run(&["--json", "push", "nowhere"]);
    assert_eq!(code(&out), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["error"]["kind"], "not_found");
    s.ok(&["remote", "remove", "disk"]);
    assert_eq!(s.ok(&["remote", "list"]), "");
}
