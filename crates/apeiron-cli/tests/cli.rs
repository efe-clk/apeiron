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
            .env("APEIRON_SEARCHES", self.root.join("searches.json"))
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
    let found = s.json(&["log", "--grep", "ONE"]);
    assert_eq!(found["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(s.ok(&["duplicates"]), "No files with the same content\n");

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
fn restore_test_checks_a_sample() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    assert!(
        s.ok(&["restore-test"])
            .starts_with("OK: restored and checked 2 files")
    );
    let doc = s.json(&["restore-test", "main", "--sample-mb", "0"]);
    assert_eq!(doc["files"], 1);
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

    // The disk is cleaned from here; it holds nothing dead.
    let out = s.ok(&["gc", "--remote", "disk", "--dry-run"]);
    assert!(out.contains("Would remove 0 objects"), "{out}");
    assert!(
        s.ok(&["gc", "--remote", "disk"])
            .contains("Removed 0 objects")
    );

    // A second device starts from the disk.
    let doc = s.json(&["clone", &s.path("disk"), &s.path("copy")]);
    assert_eq!(doc["refs"][0]["name"], "main");
    let copy = Command::new(env!("CARGO_BIN_EXE_apeiron"))
        .args(["--repo", &s.path("copy"), "pull", "origin"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&copy.stdout).contains("up to date with origin"));
    assert_eq!(
        code(&s.run(&["clone", &s.path("disk"), &s.path("copy")])),
        Some(1)
    );

    let out = s.run(&["--json", "push", "nowhere"]);
    assert_eq!(code(&out), Some(1));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["error"]["kind"], "not_found");
    s.ok(&["remote", "remove", "disk"]);
    assert_eq!(s.ok(&["remote", "list"]), "");
}

#[test]
fn daemon_runs_due_jobs_and_shows_their_status() {
    let s = Setup::new();
    let jobs = s.root.join("jobs.toml");
    let state = s.path("state");
    let toml_path = |p: String| p.replace('\\', "/");
    fs::write(
        &jobs,
        format!(
            "[job.docs]\nrepo = \"{}\"\ndo = \"backup\"\nfolder = \"{}\"\nevery = \"day\"\n\n\
             [job.test]\nrepo = \"{}\"\ndo = \"restore-test\"\nevery = \"week\"\n",
            toml_path(s.path("repo")),
            toml_path(s.path("src")),
            toml_path(s.path("repo")),
        ),
    )
    .unwrap();
    let daemon = |command: &str| {
        let args = [
            "daemon",
            command,
            "--jobs",
            jobs.to_str().unwrap(),
            "--state-dir",
            &state,
        ];
        s.ok(&args)
    };
    let first = daemon("once");
    assert!(first.contains("docs: ok: snapshot"), "{first}");
    assert!(
        first.contains("test: ok: restored and checked 2 files"),
        "{first}"
    );
    assert_eq!(daemon("once"), "No job is due\n");
    let status = daemon("status");
    assert!(status.starts_with("docs  backup  ok "), "{status}");
    assert!(status.contains("next "));

    let doc = s.json(&[
        "daemon",
        "status",
        "--jobs",
        jobs.to_str().unwrap(),
        "--state-dir",
        &state,
    ]);
    assert_eq!(doc[1]["do"], "restore-test");
    assert_eq!(doc[1]["last"]["result"], "ok");

    fs::write(
        &jobs,
        "[job.bad]\nrepo = \"/r\"\ndo = \"nothing\"\nevery = \"day\"\n",
    )
    .unwrap();
    let out = s.run(&[
        "daemon",
        "once",
        "--jobs",
        jobs.to_str().unwrap(),
        "--state-dir",
        &state,
    ]);
    assert_eq!(code(&out), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown `do"));
}

#[cfg(unix)]
#[test]
fn daemon_run_stops_cleanly_on_sigterm() {
    let s = Setup::new();
    let state = s.root.join("state");
    let mut child = Command::new(env!("CARGO_BIN_EXE_apeiron"))
        .args(["daemon", "run", "--jobs"])
        .arg(s.root.join("none.toml"))
        .arg("--state-dir")
        .arg(&state)
        .spawn()
        .unwrap();
    let log = state.join("daemon.log");
    let start = std::time::Instant::now();
    while !fs::read_to_string(&log).is_ok_and(|l| l.contains("started")) {
        assert!(start.elapsed().as_secs() < 10, "the daemon did not start");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Command::new("kill")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert!(fs::read_to_string(&log).unwrap().contains("stopped"));
}

#[test]
fn git_import_and_export_keep_commit_ids() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let s = Setup::new();
    fs::write(s.root.join("gitconfig"), "").unwrap();
    let git = |dir: &str, args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(s.root.join(dir))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", s.root.join("gitconfig"))
            .env("GIT_AUTHOR_NAME", "A")
            .env("GIT_AUTHOR_EMAIL", "a@example.org")
            .env("GIT_COMMITTER_NAME", "A")
            .env("GIT_COMMITTER_EMAIL", "a@example.org")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    git("src", &["init", "-q"]);
    git("src", &["add", "."]);
    git("src", &["commit", "-q", "-m", "first"]);
    git("src", &["tag", "-a", "v1", "-m", "one"]);

    let doc = s.json(&["git", "import", &s.path("src")]);
    assert_eq!(
        (doc["commits"].as_u64(), doc["tags"].as_u64()),
        (Some(1), Some(1))
    );
    assert!(s.ok(&["ls", "git/tags/v1"]).contains("a.txt"));
    let out = s.ok(&["git", "export", &s.path("out.git")]);
    assert!(out.contains("Wrote 1 commit and 1 tag"), "{out}");
    let refs = |dir: &str| git(dir, &["for-each-ref", "refs/heads", "refs/tags"]);
    assert_eq!(refs("out.git"), refs("src"));
    let missing = s.run(&["--json", "git", "import", &s.path("nothing")]);
    assert_eq!(code(&missing), Some(1));
}

#[test]
fn branches_are_created_merged_and_deleted() {
    let s = Setup::new();
    let src = s.path("src");
    s.ok(&["backup", &src]);
    assert!(
        s.ok(&["ref", "create", "feature"])
            .starts_with("Created feature at ")
    );
    assert_eq!(code(&s.run(&["ref", "create", "feature"])), Some(1));

    fs::write(s.root.join("src/a.txt"), b"on feature").unwrap();
    s.ok(&["backup", &src, "--ref", "feature"]);
    let doc = s.json(&["merge", "feature", "--dry-run"]);
    assert_eq!(doc["action"], "fast_forward");
    let out = s.ok(&["merge", "feature"]);
    assert!(
        out.contains("~ a.txt") && out.contains("main moved forward to feature"),
        "{out}"
    );

    // Both sides change a.txt: a conflict, exit code 3.
    fs::write(s.root.join("src/a.txt"), b"feature again").unwrap();
    s.ok(&["backup", &src, "--ref", "feature"]);
    fs::write(s.root.join("src/a.txt"), b"main again").unwrap();
    s.ok(&["backup", &src]);
    let out = s.run(&["merge", "feature"]);
    assert_eq!(code(&out), Some(3));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("a (conflict feature).txt"), "{text}");
    assert!(s.ok(&["ls", "main"]).contains("a (conflict feature).txt"));

    // A new file on feature is copied over on its own.
    fs::write(s.root.join("src/new.txt"), b"picked").unwrap();
    s.ok(&["backup", &src, "--ref", "feature"]);
    let doc = s.json(&["cherry-pick", "feature", "--dry-run"]);
    assert_eq!(doc["action"], "picked");
    let out = s.ok(&["cherry-pick", "feature"]);
    assert!(
        out.contains("+ new.txt") && out.contains("the changes of feature copied into main"),
        "{out}"
    );
    assert!(s.ok(&["ls", "main"]).contains("new.txt"));
    assert!(
        s.ok(&["cherry-pick", "feature"])
            .contains("main already has everything in feature")
    );

    // feature and main both move on; a rebase puts feature on top of main.
    fs::write(s.root.join("src/feature.txt"), b"only on feature").unwrap();
    s.ok(&["backup", &src, "--ref", "feature"]);
    fs::remove_file(s.root.join("src/feature.txt")).unwrap();
    fs::write(s.root.join("src/main.txt"), b"only on main").unwrap();
    s.ok(&["backup", &src]);
    let out = s.ok(&["rebase", "feature"]);
    assert!(out.contains("feature rebased onto main"), "{out}");
    let listing = s.ok(&["ls", "feature"]);
    assert!(
        listing.contains("feature.txt") && listing.contains("main.txt"),
        "{listing}"
    );
    assert!(
        s.ok(&["merge", "feature"])
            .contains("main moved forward to feature")
    );

    // A working folder follows main, then feature.
    let work = s.root.join("work");
    let work_arg = work.to_str().unwrap();
    assert!(
        s.ok(&["switch", "main", work_arg])
            .contains("switched to main")
    );
    assert!(work.join("main.txt").exists());
    fs::write(work.join("draft.txt"), b"not backed up").unwrap();
    let out = s.ok(&["switch", "feature~1", work_arg]);
    assert!(out.contains("the folder was recorded first"), "{out}");
    assert!(!work.join("draft.txt").exists());

    assert!(
        s.ok(&["ref", "delete", "feature"])
            .contains("Deleted feature")
    );
    assert!(!s.ok(&["refs"]).contains("feature"));
}

#[test]
fn search_by_name_text_size_and_history() {
    let s = Setup::new();
    let src = s.path("src");
    s.ok(&["backup", &src]);
    fs::remove_file(s.root.join("src/a.txt")).unwrap();
    s.ok(&["backup", &src]);

    assert_eq!(s.ok(&["search", "*.txt"]), "sub/b.txt\n");
    assert_eq!(
        s.ok(&["search", "--text", "SECOND"]),
        "sub/b.txt:1: second\n"
    );
    assert_eq!(s.ok(&["search", "--larger", "6"]), "sub/b.txt\n");
    assert!(
        s.ok(&["search", "--smaller", "1K", "--after", "2100-01-01"])
            .starts_with("No matches")
    );
    let old = s.ok(&["search", "a.txt", "--history"]);
    assert!(
        old.ends_with("  a.txt\n") && old.lines().count() == 1,
        "{old}"
    );

    let doc = s.json(&["search", "*", "--in", "main~1", "--limit", "1"]);
    assert_eq!(doc["hits"].as_array().unwrap().len(), 1);
    assert_eq!(doc["truncated"], true);

    assert_eq!(code(&s.run(&["search"])), Some(1));
    assert_eq!(code(&s.run(&["search", "--larger", "ten"])), Some(2));
    assert_eq!(
        code(&s.run(&["search", "x", "--before", "29.10.2026"])),
        Some(2)
    );

    assert_eq!(s.ok(&["search", "--list-saved"]), "No saved searches\n");
    assert_eq!(s.ok(&["search", "*.txt", "--save", "texts"]), "sub/b.txt\n");
    assert_eq!(s.ok(&["search", "--saved", "texts"]), "sub/b.txt\n");
    assert_eq!(
        s.ok(&["search", "--saved", "texts", "--text", "nothing here"]),
        "No matches in 1 snapshot\n"
    );
    assert_eq!(s.ok(&["search", "--list-saved"]), "texts  \"*.txt\"\n");
    assert_eq!(code(&s.run(&["search", "--saved", "other"])), Some(1));
    s.ok(&["search", "--forget", "texts"]);
    assert_eq!(code(&s.run(&["search", "--forget", "texts"])), Some(1));
}

/// The fixture plugin of apeiron-plugin's tests, built for wasm32-wasip2.
fn fixture_plugin() -> PathBuf {
    fixture("fixture", "test_driver")
}

/// A fixture plugin of apeiron-plugin's tests in `tests/<folder>`.
fn fixture(folder: &str, file: &str) -> PathBuf {
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../apeiron-plugin");
    let target = plugin.join("../../target/plugin-fixture");
    let status = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
        ])
        .arg(plugin.join("tests").join(folder).join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success(), "rustup target add wasm32-wasip2");
    target.join(format!("wasm32-wasip2/release/{file}.wasm"))
}

/// A store's index lists plugins; one installs from it only when its file
/// matches the index (PLUGIN.md §6).
#[test]
fn plugins_install_from_a_store() {
    let s = Setup::new();
    let plugins = s.root.join("plugins");
    let store = s.root.join("store");
    fs::create_dir_all(&store).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_apeiron"))
            .args(args)
            .env("APEIRON_REPO", s.repo())
            .env("APEIRON_PLUGINS", &plugins)
            .env("APEIRON_PLUGIN_STORE", store.join("plugins.json"))
            .output()
            .unwrap()
    };
    let ok = |args: &[&str]| {
        let out = run(args);
        assert_eq!(
            code(&out),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let hook = fixture("fixture-hook", "test_hook");
    let wasm = store.join("test_hook.wasm");
    fs::copy(&hook, &wasm).unwrap();
    let index = ok(&["plugin", "index", wasm.to_str().unwrap()]);
    fs::write(store.join("plugins.json"), &index).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&index).unwrap();
    assert_eq!(parsed["plugins"][0]["file"], "test_hook.wasm");
    assert_eq!(parsed["plugins"][0]["kind"], "hook");

    assert!(ok(&["plugin", "search", "secret"]).starts_with("test-hook 1.0  hook"));
    assert!(ok(&["plugin", "search", "nothing"]).contains("no such plugin"));
    let out = ok(&["plugin", "install", "test-hook"]);
    assert!(out.starts_with("Installed test-hook 1.0"), "{out}");
    assert!(ok(&["plugin", "list"]).starts_with("test-hook"));
    assert_ne!(code(&run(&["plugin", "install", "nothing"])), Some(0));

    // A file that is not the one the index lists is refused.
    ok(&["plugin", "remove", "test-hook"]);
    let mut data = fs::read(&wasm).unwrap();
    data.push(0);
    fs::write(&wasm, data).unwrap();
    let out = run(&["plugin", "install", "test-hook"]);
    assert_ne!(code(&out), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not the file the store lists"));
    assert!(ok(&["plugin", "list"]).starts_with("No plugins"));
}

/// An importer plugin reads an archive format for `import`, and a hook
/// plugin warns about a backup or stops it (PLUGIN.md §2.1, §2.2).
#[test]
fn importer_and_hook_plugins_work_in_import_and_backup() {
    let s = Setup::new();
    let plugins = s.root.join("plugins");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_apeiron"))
            .args(args)
            .env("APEIRON_REPO", s.repo())
            .env("APEIRON_PLUGINS", &plugins)
            .output()
            .unwrap()
    };
    let ok = |args: &[&str]| {
        let out = run(args);
        assert_eq!(
            code(&out),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let importer = fixture("fixture-importer", "test_importer");
    let out = ok(&["plugin", "add", importer.to_str().unwrap()]);
    assert!(out.contains("apeiron import reads *.lines"), "{out}");
    let archive = s.root.join("mail.lines");
    fs::write(&archive, "inbox/1.txt=hello\ninbox/2.txt=again\n").unwrap();
    let imported: serde_json::Value = serde_json::from_slice(
        &run(&[
            "--json",
            "import",
            archive.to_str().unwrap(),
            "--ref",
            "mail",
        ])
        .stdout,
    )
    .unwrap();
    assert_eq!(imported["plugin"], "test-importer", "{imported}");
    assert_eq!(imported["files"], 2);
    assert!(ok(&["ls", "mail", "inbox"]).contains("2.txt"));

    let hook = fixture("fixture-hook", "test_hook");
    let out = ok(&["plugin", "add", hook.to_str().unwrap()]);
    assert!(out.contains("It is called for: backup"), "{out}");
    assert!(ok(&["plugin", "list"]).contains("test-hook 1.0  hook  backup"));
    let src = s.path("src");
    ok(&["backup", &src]);
    fs::write(s.root.join("src/run.log"), "fine").unwrap();
    let out = run(&["backup", &src]);
    assert_eq!(code(&out), Some(3), "warnings give exit code 3");
    let said =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("test-hook: backup of main holds run.log"),
        "{said}"
    );
    let before = ok(&["log", "main"]);
    fs::write(s.root.join("src/.env"), "KEY=SECRET").unwrap();
    let out = run(&["backup", &src]);
    assert_eq!(code(&out), Some(1));
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("the plugin test-hook stopped the backup: .env holds a secret"),
        "{said}"
    );
    assert_eq!(ok(&["log", "main"]), before, "the ref did not move");
}

#[test]
fn a_plugin_merges_what_would_be_a_conflict() {
    let s = Setup::new();
    let plugins = s.root.join("plugins");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_apeiron"))
            .args(args)
            .env("APEIRON_REPO", s.repo())
            .env("APEIRON_PLUGINS", &plugins)
            .output()
            .unwrap()
    };
    let ok = |args: &[&str]| {
        let out = run(args);
        assert_eq!(
            code(&out),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };

    assert!(ok(&["plugin", "list"]).starts_with("No plugins"));
    let wasm = fixture_plugin();
    let out = ok(&["plugin", "add", wasm.to_str().unwrap()]);
    assert!(out.starts_with("Installed test-driver 1.0"), "{out}");
    assert!(ok(&["plugin", "list"]).starts_with("test-driver 1.0  type driver  *.merge"));

    // The repository carries a plugin to its other copies (PLUGIN.md §5).
    ok(&["plugin", "share", "test-driver"]);
    ok(&["plugin", "remove", "test-driver"]);
    assert!(ok(&["plugin", "list", "--shared"]).starts_with("test-driver 1.0"));
    let out = ok(&["plugin", "add", "--from-repo", "test-driver"]);
    assert!(out.starts_with("Installed test-driver 1.0"), "{out}");
    ok(&["plugin", "unshare", "test-driver"]);
    assert!(ok(&["plugin", "list", "--shared"]).starts_with("No plugins"));

    let src = s.path("src");
    fs::write(s.root.join("src/a.merge"), b"base").unwrap();
    ok(&["backup", &src]);
    ok(&["ref", "create", "feature"]);
    fs::write(s.root.join("src/a.merge"), b"theirs").unwrap();
    ok(&["backup", &src, "--ref", "feature"]);
    fs::write(s.root.join("src/a.merge"), b"ours").unwrap();
    ok(&["backup", &src]);

    let out = ok(&["merge", "feature"]);
    assert!(
        out.contains("merged by plugin test-driver: a.merge"),
        "{out}"
    );
    assert!(!ok(&["ls", "main"]).contains("conflict"));
    ok(&["restore", "main", &s.path("out"), "--path", "a.merge"]);
    let restored = fs::read_dir(s.root.join("out"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read(restored).unwrap(), b"base|ours|theirs");

    assert!(ok(&["plugin", "remove", "test-driver"]).starts_with("Removed test-driver"));
    assert_eq!(code(&run(&["plugin", "remove", "test-driver"])), Some(1));
}

#[test]
fn the_text_plugin_merges_lines_and_shows_differences() {
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/text");
    let target = plugin.join("../../target/plugin-text");
    let status = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
        ])
        .arg(plugin.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success(), "rustup target add wasm32-wasip2");
    let wasm = target.join("wasm32-wasip2/release/apeiron_text.wasm");

    let s = Setup::new();
    let plugins = s.root.join("plugins");
    let ok = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_apeiron"))
            .args(args)
            .env("APEIRON_REPO", s.repo())
            .env("APEIRON_PLUGINS", &plugins)
            .output()
            .unwrap();
        assert_eq!(
            code(&out),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    assert!(ok(&["plugin", "add", wasm.to_str().unwrap()]).starts_with("Installed text 0.1.0"));

    let src = s.path("src");
    let notes = s.root.join("src/notes.md");
    fs::write(&notes, "# Notes\n\none\ntwo\nthree\nfour\nfive\n").unwrap();
    ok(&["backup", &src]);
    ok(&["ref", "create", "feature"]);
    fs::write(&notes, "# Notes\n\none\ntwo\nthree\nfour\nFIVE\n").unwrap();
    ok(&["backup", &src, "--ref", "feature"]);
    fs::write(&notes, "# Notes\n\nONE\ntwo\nthree\nfour\nfive\n").unwrap();
    ok(&["backup", &src]);

    let diff = ok(&["diff", "main~1", "main", "--text"]);
    assert!(diff.starts_with("M notes.md\n@@ "), "{diff}");
    assert!(diff.contains("\n-one\n+ONE\n"), "{diff}");

    let out = ok(&["merge", "feature"]);
    assert!(out.contains("merged by plugin text: notes.md"), "{out}");
    ok(&["restore", "main", &s.path("out")]);
    assert_eq!(
        fs::read_to_string(s.root.join("out/notes.md")).unwrap(),
        "# Notes\n\nONE\ntwo\nthree\nfour\nFIVE\n"
    );

    // Two changes to neighbouring lines of a JSON file join by key.
    let settings = s.root.join("src/settings.json");
    fs::write(&settings, "{\n  \"port\": 80,\n  \"name\": \"a\"\n}\n").unwrap();
    ok(&["backup", &src]);
    ok(&["ref", "delete", "feature"]);
    ok(&["ref", "create", "feature"]);
    fs::write(&settings, "{\n  \"port\": 80,\n  \"name\": \"b\"\n}\n").unwrap();
    ok(&["backup", &src, "--ref", "feature"]);
    fs::write(&settings, "{\n  \"port\": 8080,\n  \"name\": \"a\"\n}\n").unwrap();
    ok(&["backup", &src]);
    let out = ok(&["merge", "feature"]);
    assert!(
        out.contains("merged by plugin text: settings.json"),
        "{out}"
    );
    ok(&["restore", "main", &s.path("out2")]);
    assert_eq!(
        fs::read_to_string(s.root.join("out2/settings.json")).unwrap(),
        "{\n  \"port\": 8080,\n  \"name\": \"b\"\n}\n"
    );
}

/// Backups and restores of the CLI go into the web server's audit log when
/// there is one (AUDIT.md §1).
#[cfg(target_os = "linux")]
#[test]
fn cli_backups_and_restores_are_recorded() {
    let s = Setup::new();
    let state = s.root.join("state");
    let run = |args: &[&str], with_log: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_apeiron"));
        command
            .args(args)
            .env("APEIRON_REPO", s.repo())
            .env("XDG_CONFIG_HOME", s.root.join("config"))
            .env("XDG_STATE_HOME", &state);
        let out = command.output().unwrap();
        assert_eq!(code(&out), Some(0), "{args:?} {with_log}");
        out.stdout
    };
    // Without a log nothing is written.
    run(&["backup", &s.path("src")], false);
    assert!(!state.join("apeiron/audit.log").exists());

    fs::create_dir_all(state.join("apeiron")).unwrap();
    fs::write(state.join("apeiron/audit.log"), b"").unwrap();
    run(&["backup", &s.path("src")], true);
    run(&["restore", "main", &s.path("out")], true);
    let shown: serde_json::Value =
        serde_json::from_slice(&run(&["--json", "web", "audit", "show"], true)).unwrap();
    let events: Vec<&str> = shown
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(
        events,
        [
            "start:backup",
            "done:backup",
            "start:restore",
            "done:restore"
        ]
    );
    assert_eq!(shown[0]["from"], "cli");
    assert_eq!(shown[1]["detail"]["via"], "cli");
    assert!(shown[1]["detail"]["snapshot"].is_string());
    run(&["web", "audit", "verify"], true);
}

/// A report maps its facts to a standard's controls and lists legal holds
/// (REPORTS.md §3.1).
#[test]
fn a_report_maps_to_a_standard() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    s.ok(&["hold", "add", "main", "--reason", "Case 1"]);
    let report = s.json(&["report", "--standard", "iso27001"]);
    assert_eq!(report["holds"][0]["reason"], "Case 1");
    let controls = report["standard"]["controls"].as_array().unwrap();
    let status = |id: &str| {
        controls
            .iter()
            .find(|c| c["id"] == id)
            .map(|c| c["status"].as_str().unwrap().to_owned())
            .unwrap()
    };
    assert_eq!(status("A.8.24"), "gap");
    assert_eq!(status("A.5.33"), "evidence");
    assert_eq!(status("A.5.15"), "not_covered");
    assert_eq!(status("A.5.30"), "gap");
    let text = s.ok(&["report", "--standard", "iso27001"]);
    assert!(text.contains("## ISO/IEC 27001:2022 Annex A"), "{text}");
    assert!(text.contains("## Legal holds"), "{text}");
}

/// SOC 2 shows the same facts as ISO/IEC 27001, under its own criteria
/// (REPORTS.md §3.1).
#[test]
fn a_report_maps_to_soc2_with_the_same_facts() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    let iso = s.json(&["report", "--standard", "iso27001"]);
    let soc2 = s.json(&["report", "--standard", "soc2"]);
    let of = |report: &serde_json::Value, id: &str| {
        report["standard"]["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no {id}"))
    };
    assert_eq!(soc2["standard"]["controls"].as_array().unwrap().len(), 7);
    for (from, to) in [
        ("A.5.17", "CC6.1"),
        ("A.5.15", "CC6.3"),
        ("A.8.24", "CC6.7"),
        ("A.8.15", "CC7.2"),
        ("A.5.30", "CC9.1"),
        ("A.8.13", "A1.2"),
        ("A.5.33", "C1.1"),
    ] {
        let (a, b) = (of(&iso, from), of(&soc2, to));
        assert_eq!(a["status"], b["status"], "{to}");
        assert_eq!(a["facts"], b["facts"], "{to}");
    }
    assert_eq!(of(&soc2, "CC6.7")["status"], "gap");
    assert_eq!(of(&soc2, "CC6.3")["status"], "not_covered");
    let text = s.ok(&["report", "--standard", "soc2"]);
    assert!(text.contains("## SOC 2 Trust Services Criteria"), "{text}");
    assert!(
        text.contains("| CC6.7 Protecting data that moves | gap |"),
        "{text}"
    );
}

/// Operations that change the repository are recorded, newest first, with
/// how they ended; dry runs are not (FORMAT.md §9.5).
#[test]
fn the_operation_log_records_changes() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    s.ok(&["gc", "--dry-run"]);
    assert_ne!(code(&s.run(&["ref", "create", "main", "main"])), Some(0));
    s.ok(&["ref", "delete", "main"]);
    let ops = s.json(&["oplog"]);
    let seen: Vec<(String, String, String)> = ops
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["op"].as_str().unwrap().to_owned(),
                o["outcome"].as_str().unwrap().to_owned(),
                o["detail"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    let want = |op: &str, outcome: &str, detail: &str| {
        (op.to_owned(), outcome.to_owned(), detail.to_owned())
    };
    assert_eq!(
        seen,
        [
            want("delete-ref", "ok", "main"),
            want("create-ref", "failed", "main"),
            want("backup", "ok", "main"),
            want("init", "ok", ""),
        ]
    );
    assert_eq!(s.json(&["oplog", "-n", "1"]).as_array().unwrap().len(), 1);
    let text = s.ok(&["oplog"]);
    assert!(
        text.lines().next().unwrap().ends_with("delete-ref  main"),
        "{text}"
    );
}

/// A review goes from opening to merging on the command line
/// (REVIEW.md §3).
#[test]
fn a_review_is_opened_approved_and_merged() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    s.ok(&["ref", "create", "draft", "main"]);
    fs::write(s.root.join("src/a.txt"), b"changed").unwrap();
    s.ok(&["backup", &s.path("src"), "--ref", "draft"]);
    let opened = s.json(&[
        "review", "open", "draft", "--title", "Change a", "--author", "ayse",
    ]);
    let id = opened["id"].as_str().unwrap().to_owned();
    let listed = s.ok(&["review", "list"]);
    assert!(
        listed.contains(&id) && listed.contains("Change a"),
        "{listed}"
    );
    let merge = ["review", "merge", &id, "--author", "ayse"];
    assert_eq!(code(&s.run(&merge)), Some(1));
    s.ok(&[
        "review",
        "comment",
        &id,
        "Looks right",
        "--path",
        "a.txt",
        "--author",
        "mehmet",
    ]);
    let approved = s.json(&["review", "approve", &id, "--author", "mehmet"]);
    assert_eq!(approved["ready"], true);
    let shown = s.ok(&["review", "show", &id]);
    assert!(shown.contains("mehmet commented on a.txt"), "{shown}");
    assert!(shown.contains("M a.txt"), "{shown}");
    s.ok(&merge);
    assert!(s.ok(&["review", "list"]).contains("No open reviews"));
    let ops = s.ok(&["oplog"]);
    assert!(
        ops.lines().next().unwrap().contains("merge-review"),
        "{ops}"
    );
}

/// `apeiron call` runs any operation with a JSON request, short or long
/// (API.md §2.1).
#[test]
fn any_operation_runs_with_a_json_request() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    let page: serde_json::Value =
        serde_json::from_str(&s.ok(&["call", "log", r#"{"start": "main", "limit": 1}"#])).unwrap();
    assert_eq!(page["snapshots"].as_array().unwrap().len(), 1);
    let verified: serde_json::Value = serde_json::from_str(&s.ok(&["call", "verify"])).unwrap();
    assert!(
        verified["objects_checked"].as_u64().unwrap() > 0,
        "{verified}"
    );
    assert_ne!(code(&s.run(&["call", "nonsense"])), Some(0));
    assert_eq!(code(&s.run(&["call", "log", "not json"])), Some(2));
}

/// The GDPR shows the same facts as ISO/IEC 27001, under its articles
/// (REPORTS.md §3.1).
#[test]
fn a_report_maps_to_the_gdpr_with_the_same_facts() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    let iso = s.json(&["report", "--standard", "iso27001"]);
    let gdpr = s.json(&["report", "--standard", "gdpr"]);
    let of = |report: &serde_json::Value, id: &str| {
        report["standard"]["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no {id}"))
    };
    assert_eq!(gdpr["standard"]["controls"].as_array().unwrap().len(), 6);
    for (from, to) in [
        ("A.8.24", "Art. 32(1)(a)"),
        ("A.5.30", "Art. 32(1)(b)"),
        ("A.8.13", "Art. 32(1)(c)"),
        ("A.5.15", "Art. 32(4)"),
        ("A.5.33", "Art. 5(1)(f)"),
        ("A.8.15", "Art. 5(2)"),
    ] {
        let (a, b) = (of(&iso, from), of(&gdpr, to));
        assert_eq!(a["status"], b["status"], "{to}");
        assert_eq!(a["facts"], b["facts"], "{to}");
    }
    let text = s.ok(&["report", "--standard", "gdpr"]);
    assert!(text.contains("## GDPR Articles 5 and 32"), "{text}");
    assert!(
        text.contains("| Art. 32(1)(a) Encryption of personal data | gap |"),
        "{text}"
    );
}

/// A report written to a file can be signed and checked (REPORTS.md §5.1).
#[test]
fn a_signed_report_is_checked() {
    let s = Setup::new();
    s.ok(&["backup", &s.path("src")]);
    let made = s.json(&["report", "key", &s.path("report.key")]);
    let public = made["public_key"].as_str().unwrap().to_owned();
    let report = s.path("report.md");
    s.ok(&["report", "--out", &report, "--sign", &s.path("report.key")]);
    assert!(
        fs::read_to_string(&report)
            .unwrap()
            .starts_with("# Compliance report")
    );
    s.ok(&["report", "check", &report, "--public-key", &public]);
    let other = s.run(&["report", "check", &report, "--public-key", "00"]);
    assert_eq!(code(&other), Some(4));
    fs::write(&report, "# Compliance report\n\nAll is well.\n").unwrap();
    assert_eq!(code(&s.run(&["report", "check", &report])), Some(4));
}

/// Tags set on files are kept by backups, found by search and written back
/// by restore (SEARCH.md §1, FORMAT.md §6.3).
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn tags_are_kept_found_and_restored() {
    let s = Setup::new();
    let file = s.path("src/a.txt");
    let tagged = s.run(&["tag", &file, "--add", "Fatura", "--add", "2026"]);
    if code(&tagged) != Some(0) {
        // A file system without user attributes cannot hold tags.
        eprintln!("skipped: {}", String::from_utf8_lossy(&tagged.stderr));
        return;
    }
    s.ok(&["backup", &s.path("src")]);
    let found = s.json(&["search", "--tag", "fatura", "--tag", "2026"]);
    let paths: Vec<&str> = found["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["a.txt"]);
    assert_eq!(found["hits"][0]["entry"]["tags"][0], "Fatura");
    assert!(
        s.json(&["search", "--tag", "other"])["hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    s.ok(&["restore", "main", &s.path("out")]);
    let shown = s.json(&["tag", &s.path("out/a.txt")]);
    assert_eq!(shown[0]["tags"], serde_json::json!(["Fatura", "2026"]));
    s.ok(&["tag", &file, "--remove", "fatura", "--remove", "2026"]);
    assert_eq!(s.json(&["tag", &file])[0]["tags"], serde_json::json!([]));
}

#[test]
fn damaged_data_is_replaced_from_a_copy() {
    let s = Setup::new();
    let copy = s.root.join("copy");
    s.ok(&["backup", s.root.join("src").to_str().unwrap()]);
    s.ok(&["remote", "add", "disk", copy.to_str().unwrap()]);
    s.ok(&["push", "disk"]);
    assert_eq!(code(&s.run(&["repair", "--from", "disk"])), Some(0));

    let shard = fs::read_dir(s.repo().join("packs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let pack = fs::read_dir(shard).unwrap().next().unwrap().unwrap().path();
    let mut bytes = fs::read(&pack).unwrap();
    for byte in bytes.iter_mut().skip(16) {
        *byte ^= 1;
    }
    fs::write(&pack, bytes).unwrap();
    assert_eq!(code(&s.run(&["verify"])), Some(4));
    // A pull finds nothing new and changes nothing.
    s.ok(&["pull", "disk"]);
    assert_eq!(code(&s.run(&["verify"])), Some(4));

    let repaired = s.ok(&["repair", "--from", "disk"]);
    assert!(repaired.contains("1 pack damaged"), "{repaired}");
    assert!(!pack.exists());
    s.ok(&["verify"]);
    let restored = s.root.join("restored");
    s.ok(&["restore", "main", restored.to_str().unwrap()]);
    assert_eq!(fs::read(restored.join("a.txt")).unwrap(), b"first");
}

#[test]
fn old_backups_are_pruned_and_a_copy_follows() {
    let s = Setup::new();
    let copy = s.root.join("copy");
    let src = s.root.join("src");
    for text in ["one", "two", "three"] {
        fs::write(src.join("a.txt"), text).unwrap();
        s.ok(&["backup", src.to_str().unwrap(), "--message", text]);
    }
    s.ok(&["remote", "add", "disk", copy.to_str().unwrap()]);
    s.ok(&["push", "disk"]);

    let trial = s.ok(&["prune", "main", "--keep-last", "1", "--dry-run"]);
    assert!(trial.contains("Would remove 2 snapshots"), "{trial}");
    assert_eq!(s.ok(&["log"]).lines().count(), 3);
    assert_eq!(code(&s.run(&["prune", "main"])), Some(1));

    s.ok(&["prune", "main", "--keep-last", "1"]);
    let log = s.ok(&["log"]);
    assert_eq!(log.lines().count(), 1, "{log}");
    assert!(log.contains("three"), "{log}");
    assert_eq!(code(&s.run(&["push", "disk"])), Some(1));
    s.ok(&["push", "disk", "--replace"]);
    s.ok(&["gc", "--keep-days", "0"]);
    s.ok(&["verify"]);
    let restored = s.root.join("restored");
    s.ok(&["restore", "main", restored.to_str().unwrap()]);
    assert_eq!(fs::read(restored.join("a.txt")).unwrap(), b"three");
}

/// A small PDF with one line of text, with a correct table of its objects.
fn pdf_with(line: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 50 700 Td ({line}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_owned(),
    ];
    let mut out = String::from("%PDF-1.5\n");
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let start = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

/// The documents plugin, built for WebAssembly and installed, gives search
/// the text inside a PDF and an office file (SEARCH.md §3).
#[test]
fn the_documents_plugin_finds_text_inside_files() {
    use std::io::Write;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target/documents-plugin");
    let status = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
        ])
        .arg(root.join("plugins/documents/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success(), "rustup target add wasm32-wasip2");
    let wasm = target.join("wasm32-wasip2/release/apeiron_documents.wasm");

    let s = Setup::new();
    let plugins = s.root.join("plugins");
    let run = |args: &[&str], with_plugin: bool| {
        let dir = if with_plugin {
            plugins.clone()
        } else {
            s.root.join("none")
        };
        Command::new(env!("CARGO_BIN_EXE_apeiron"))
            .args(args)
            .env("APEIRON_REPO", s.repo())
            .env("APEIRON_PLUGINS", dir)
            .output()
            .unwrap()
    };
    let ok = |args: &[&str], with_plugin: bool| {
        let out = run(args, with_plugin);
        assert_eq!(
            code(&out),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };

    fs::write(
        s.root.join("src/report.pdf"),
        pdf_with("Annual invoice summary"),
    )
    .unwrap();
    let docx = fs::File::create(s.root.join("src/letter.docx")).unwrap();
    let mut zip = zip::ZipWriter::new(docx);
    zip.start_file(
        "word/document.xml",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"<w:document><w:p><w:t>Rent contract</w:t></w:p></w:document>")
        .unwrap();
    zip.finish().unwrap();
    // A damaged one is counted and stops nothing.
    fs::write(s.root.join("src/broken.pdf"), b"%PDF-1.5 not really").unwrap();
    ok(&["backup", &s.path("src")], true);

    // Without the plugin an office file holds no text.
    assert!(ok(&["search", "--text", "contract"], false).starts_with("No matches"));

    ok(&["plugin", "add", wasm.to_str().unwrap()], true);
    let found = ok(&["search", "--text", "invoice"], true);
    assert!(
        found.contains("report.pdf:") && found.contains("Annual invoice summary"),
        "{found}"
    );
    assert!(found.contains("1 file a plugin could not read"), "{found}");
    let found = ok(&["search", "--text", "rent contract"], true);
    assert!(found.contains("letter.docx:1: Rent contract"), "{found}");
    // Plain text is searched as before.
    let found = ok(&["search", "--text", "first"], true);
    assert!(found.contains("a.txt:1: first"), "{found}");
}

/// Git clones and pushes over SSH as a user of the web server: the SSH
/// server runs `apeiron git ssh` for the user's key (GIT.md §7.2). A small
/// script stands in for ssh and runs it the way the forced command would.
#[cfg(target_os = "linux")]
#[test]
fn git_clones_and_pushes_over_ssh() {
    let s = Setup::new();
    let config = s.root.join("config");
    let state = s.root.join("state");
    fs::create_dir_all(config.join("apeiron")).unwrap();
    fs::write(
        config.join("apeiron/web.json"),
        r#"{"users": [{"name": "ayse", "role": "member", "backups": ["git/*"]}, {"name": "okan", "role": "reader", "backups": ["git/*"]}]}"#,
    )
    .unwrap();
    let ssh = s.root.join("fake-ssh");
    fs::write(
        &ssh,
        format!(
            "#!/bin/sh\n# Git calls: <ssh> [options] <host> <command>\nfor last; do :; done\nSSH_ORIGINAL_COMMAND=\"$last\" SSH_CONNECTION='192.0.2.1 5555 192.0.2.2 22' XDG_CONFIG_HOME='{}' XDG_STATE_HOME='{}' exec '{}' --repo '{}' git ssh --user \"$APEIRON_TEST_USER\"\n",
            config.display(),
            state.display(),
            env!("CARGO_BIN_EXE_apeiron"),
            s.repo().display()
        ),
    )
    .unwrap();
    fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let git = |dir: &Path, user: &str, args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_SSH_COMMAND", &ssh)
            .env("APEIRON_TEST_USER", user)
            .env("GIT_AUTHOR_NAME", "Ayse")
            .env("GIT_AUTHOR_EMAIL", "ayse@example.com")
            .env("GIT_COMMITTER_NAME", "Ayse")
            .env("GIT_COMMITTER_EMAIL", "ayse@example.com")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    // A Git project goes in through `git import`, then comes out over SSH.
    let project = s.root.join("project");
    fs::create_dir_all(&project).unwrap();
    assert!(git(&project, "ayse", &["init", "-q", "-b", "main"]).0);
    fs::write(project.join("a.txt"), "one\n").unwrap();
    assert!(git(&project, "ayse", &["add", "a.txt"]).0);
    assert!(git(&project, "ayse", &["commit", "-q", "-m", "first"]).0);
    s.ok(&["git", "import", &project.to_string_lossy()]);

    let (ok, err) = git(&s.root, "ayse", &["clone", "-q", "ssh://nas/git", "copy"]);
    assert!(ok, "{err}");
    let copy = s.root.join("copy");
    assert_eq!(fs::read_to_string(copy.join("a.txt")).unwrap(), "one\n");

    fs::write(copy.join("b.txt"), "two\n").unwrap();
    assert!(git(&copy, "ayse", &["add", "b.txt"]).0);
    assert!(git(&copy, "ayse", &["commit", "-q", "-m", "second"]).0);
    let (ok, err) = git(&copy, "ayse", &["push", "-q", "origin", "main"]);
    assert!(ok, "{err}");
    let log = s.ok(&["log", "git/heads/main"]);
    assert!(log.contains("second"), "{log}");

    // A fetch after the push sends only what is new.
    let (ok, err) = git(&project, "ayse", &["pull", "-q", "ssh://nas/git", "main"]);
    assert!(ok, "{err}");
    assert!(project.join("b.txt").exists());

    // A reader may clone but not push.
    fs::write(copy.join("c.txt"), "three\n").unwrap();
    assert!(git(&copy, "okan", &["add", "c.txt"]).0);
    assert!(git(&copy, "okan", &["commit", "-q", "-m", "third"]).0);
    let (ok, err) = git(&copy, "okan", &["push", "-q", "origin", "main"]);
    assert!(!ok);
    assert!(err.contains("reader"), "{err}");

    let audit = fs::read_to_string(state.join("apeiron/audit.log")).unwrap();
    assert!(
        audit.contains("\"git_push\"") && audit.contains("192.0.2.1:5555 ssh"),
        "{audit}"
    );

    // The line for authorized_keys runs this program for the key.
    let key = s.root.join("id.pub");
    fs::write(
        &key,
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample ayse@laptop\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_apeiron"))
        .args(["git", "ssh-key", "ayse", &key.to_string_lossy()])
        .env("APEIRON_REPO", s.repo())
        .env("XDG_CONFIG_HOME", &config)
        .output()
        .unwrap();
    let line = String::from_utf8(out.stdout).unwrap();
    assert!(
        line.starts_with("command=\"")
            && line.contains(" git ssh --user ayse --repo ")
            && line.contains("\",restrict ssh-ed25519 AAAA"),
        "{line}"
    );
}

#[test]
fn ape_is_the_same_command() {
    let run = |program: &str, args: &[&str]| Command::new(program).args(args).output().unwrap();
    let apeiron = env!("CARGO_BIN_EXE_apeiron");
    let ape = env!("CARGO_BIN_EXE_ape");
    for args in [&["--version"][..], &["no-such-command"][..]] {
        let (long, short) = (run(apeiron, args), run(ape, args));
        assert_eq!(short.status.code(), long.status.code(), "{args:?}");
        assert_eq!(short.stdout, long.stdout, "{args:?}");
        assert_eq!(short.stderr, long.stderr, "{args:?}");
    }
}
