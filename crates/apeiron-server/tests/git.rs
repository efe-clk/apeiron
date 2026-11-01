//! `git clone` from the web server (GIT.md §7). Skipped when `git` is not
//! installed.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use apeiron::{BackupRequest, Repository};
use apeiron_server::{Audit, Role, Shares, User, Users, Web, WebOptions, hash_password};

const PASSWORD: &str = "correct horse";

fn git(dir: &Path, args: &[&str]) -> Output {
    git_traced(dir, args, false)
}

/// With `trace`, Git writes the packets it sends and gets to its errors.
fn git_traced(dir: &Path, args: &[&str], trace: bool) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", dir.join("no-gitconfig"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_TRACE_PACKET", if trace { "1" } else { "0" })
        .output()
        .unwrap()
}

fn user(name: &str, role: Role, backups: &[&str], hash: &str) -> User {
    User {
        name: name.to_owned(),
        password: hash.to_owned(),
        role,
        backups: backups.iter().map(|b| (*b).to_owned()).collect(),
        folders: Vec::new(),
        oidc: None,
        totp: None,
    }
}

#[test]
fn git_clones_the_branches_a_user_sees() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let repo = Repository::init(root.join("repo")).unwrap();
    for (folder, file, r) in [
        ("open", "readme.txt", "git/heads/main"),
        ("hidden", "secret.txt", "git/heads/secret"),
    ] {
        fs::create_dir_all(root.join(folder)).unwrap();
        fs::write(root.join(folder).join(file), folder).unwrap();
        repo.backup(BackupRequest::new(root.join(folder), r))
            .unwrap()
            .wait()
            .unwrap();
    }

    let hash = hash_password(PASSWORD).unwrap();
    let file = root.join("config/web.json");
    let mut users = Users::default();
    users.add(user("root", Role::Admin, &[], &hash)).unwrap();
    users
        .add(user("ayse", Role::Reader, &["git/heads/main"], &hash))
        .unwrap();
    let mut with_code = user("cem", Role::Admin, &[], &hash);
    with_code.totp = Some("JBSWY3DPEHPK3PXP".to_owned());
    users.add(with_code).unwrap();
    users.save(&file).unwrap();
    let server = Web::start(WebOptions {
        repo: repo.clone(),
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: None,
        users: file,
        audit: Audit::new(root.join("state/audit.log")),
        shares: Shares::new(root.join("state/shares.json")),
        folders: Vec::new(),
        replicas: Vec::new(),
        standby: false,
    })
    .unwrap();
    let url = |name: &str, password: &str| {
        format!(
            "http://{name}:{}@{}/git/git.git",
            password.replace(' ', "%20"),
            server.address()
        )
    };

    // A reader gets only the branch it may see, checked out.
    let out = git(root, &["clone", "-q", &url("ayse", PASSWORD), "ayse"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.join("ayse/readme.txt")).unwrap(),
        "open"
    );
    let branches = git(&root.join("ayse"), &["branch", "-r"]);
    let branches = String::from_utf8_lossy(&branches.stdout).into_owned();
    assert!(
        branches.contains("origin/main") && !branches.contains("secret"),
        "{branches}"
    );
    let fsck = git(&root.join("ayse"), &["fsck", "--no-progress"]);
    assert!(fsck.status.success());

    // An admin sees both; fetching again works too.
    let out = git(root, &["clone", "-q", &url("root", PASSWORD), "all"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = git(&root.join("all"), &["fetch", "-q", "origin", "secret"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // A new snapshot on the server comes with the next pull, which sends
    // what the client has and gets only what is new.
    fs::write(root.join("open/readme.txt"), "open, changed").unwrap();
    repo.backup(BackupRequest::new(root.join("open"), "git/heads/main"))
        .unwrap()
        .wait()
        .unwrap();
    let out = git_traced(&root.join("ayse"), &["pull", "-q", "--ff-only"], true);
    let trace = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{trace}");
    assert!(trace.contains("< ACK "), "{trace}");
    assert_eq!(
        fs::read_to_string(root.join("ayse/readme.txt")).unwrap(),
        "open, changed"
    );
    let fsck = git(&root.join("ayse"), &["fsck", "--no-progress"]);
    assert!(fsck.status.success());

    // A wrong password, and a user whose second factor Git cannot send,
    // get nothing.
    for (name, password) in [("ayse", "wrong password"), ("cem", PASSWORD)] {
        let out = git(root, &["clone", "-q", &url(name, password), name]);
        assert!(!out.status.success(), "{name}");
    }
    // Pushing is not served.
    let out = git(&root.join("all"), &["push", "-q", "origin", "main:pushed"]);
    assert!(!out.status.success());

    let log = fs::read_to_string(root.join("state/audit.log")).unwrap();
    assert!(log.contains("\"git_fetch\""), "{log}");
    assert!(log.contains("\"via\":\"git\""), "{log}");
}
