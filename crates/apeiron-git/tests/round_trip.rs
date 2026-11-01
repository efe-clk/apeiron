//! A Git repository made with `git` goes into Apeiron and back with the same
//! object ids (GIT.md). Skipped when `git` is not installed.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use apeiron_core::{
    ChunkerParams, EntryKind, ObjectId, Repository, Snapshot, Time, Tree, TreeEntry,
};

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Env {
    fn new() -> Option<Env> {
        Command::new("git").arg("--version").output().ok()?;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::write(root.join("gitconfig"), "").unwrap();
        Some(Env { _dir: dir, root })
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        self.git_with(dir, args, None)
    }

    fn git_with(&self, dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> String {
        let mut child = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("HOME", &self.root)
            .env("GIT_AUTHOR_NAME", "Ada Writer")
            .env("GIT_AUTHOR_EMAIL", "ada@example.org")
            .env("GIT_AUTHOR_DATE", "1700000000 +0100")
            .env("GIT_COMMITTER_NAME", "Cem Committer")
            .env("GIT_COMMITTER_EMAIL", "cem@example.org")
            .env("GIT_COMMITTER_DATE", "1700000500 -0230")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(input) = stdin {
            child.stdin.take().unwrap().write_all(input).unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    /// Branches and tags with their ids, as `git for-each-ref` lists them.
    fn refs(&self, dir: &Path) -> String {
        let format = "--format=%(objectname) %(refname)";
        let all = self.git(
            dir,
            &[
                "for-each-ref",
                format,
                "refs/heads",
                "refs/tags",
                "refs/remotes",
            ],
        );
        // A symbolic ref such as origin/HEAD is not imported.
        all.lines()
            .filter(|l| !l.ends_with("/HEAD"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A repository with a merge, a branch with capitals, an executable, a
/// symbolic link, a submodule entry, both kinds of tag, a signature header
/// and a message that is not UTF-8.
fn make_repo(env: &Env) -> PathBuf {
    let src = env.root.join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    env.git(&src, &["init", "-q"]);
    env.git(&src, &["checkout", "-q", "-b", "main"]);
    fs::write(src.join("a.txt"), "first\n").unwrap();
    fs::write(src.join("sub/b.txt"), "second\n").unwrap();
    fs::write(src.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
    env.git(&src, &["add", "."]);
    env.git(&src, &["update-index", "--chmod=+x", "run.sh"]);
    let target = env.git_with(&src, &["hash-object", "-w", "--stdin"], Some(b"a.txt"));
    env.git(
        &src,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{target},link"),
        ],
    );
    let module = "1111111111111111111111111111111111111111";
    env.git(
        &src,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{module},libs/mod"),
        ],
    );
    env.git(&src, &["commit", "-q", "-m", "first"]);

    env.git(&src, &["checkout", "-q", "-b", "Feature/ABC-1"]);
    // Files are added by name: `commit -a` would drop the link and the
    // submodule entry, which have no file in the working folder.
    fs::write(src.join("sub/b.txt"), "second, on a branch\n").unwrap();
    env.git(&src, &["add", "sub/b.txt"]);
    env.git(&src, &["commit", "-q", "-m", "branch work"]);
    env.git(&src, &["checkout", "-q", "main"]);
    fs::write(src.join("a.txt"), "first, changed\n").unwrap();
    env.git(&src, &["add", "a.txt"]);
    env.git(&src, &["commit", "-q", "-m", "main work"]);
    env.git(
        &src,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "merge the branch",
            "Feature/ABC-1",
        ],
    );
    env.git(&src, &["tag", "-a", "v1.0", "-m", "release one"]);
    env.git(&src, &["tag", "light"]);

    // A commit with headers `git commit` writes only when signing, and a
    // Latin-1 message.
    let head = env.git(&src, &["rev-parse", "HEAD"]);
    let tree = env.git(&src, &["rev-parse", "HEAD^{tree}"]);
    let mut raw = format!(
        "tree {tree}\nparent {head}\nauthor Ada Writer <ada@example.org> 1700001000 +0100\n\
         committer Cem Committer <cem@example.org> 1700001000 +0100\nencoding ISO-8859-1\n\
         gpgsig -----BEGIN PGP SIGNATURE-----\n \n iQEzBAABCAAdFiEE\n -----END PGP SIGNATURE-----\n\n"
    )
    .into_bytes();
    raw.extend_from_slice(b"caf\xe9\n");
    let signed = env.git_with(
        &src,
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        Some(&raw),
    );
    env.git(&src, &["update-ref", "refs/heads/signed", &signed]);
    env.git(&src, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    env.git(
        &src,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    src
}

fn init(path: &Path) -> Repository {
    Repository::init_with(path, ChunkerParams::new(64 * 1024).unwrap()).unwrap()
}

#[test]
fn a_repository_goes_in_and_comes_back_with_the_same_ids() {
    let Some(env) = Env::new() else { return };
    let src = make_repo(&env);
    let mut repo = init(&env.root.join("apeiron"));

    let mut seen = 0;
    let report = apeiron_git::import(&mut repo, &src, "git", &mut |n| {
        seen = n;
        Ok(())
    })
    .unwrap();
    assert_eq!((report.commits, report.tags), (5, 1), "{report:?}");
    assert_eq!(seen, 6);
    assert_eq!(report.refs.len(), 6);
    let skipped: Vec<&str> = report.skipped.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(skipped, ["refs/remotes/origin/HEAD"]);
    let names: Vec<String> = repo
        .list_refs()
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        names,
        [
            "git/heads/_46eature/_41_42_43-1",
            "git/heads/main",
            "git/heads/signed",
            "git/remotes/origin/main",
            "git/tags/light",
            "git/tags/v1.0",
        ]
    );
    // The snapshot holds the files as Git had them.
    let main = repo.read_ref("git/heads/main").unwrap().unwrap();
    let snapshot = repo.read_snapshot(&main).unwrap();
    assert_eq!(
        snapshot.author.as_deref(),
        Some("Ada Writer <ada@example.org>")
    );
    assert_eq!(snapshot.time.seconds, 1_700_000_500);
    assert_eq!(snapshot.parents.len(), 2);
    let tree = repo.read_tree(&snapshot.tree).unwrap();
    let names: Vec<&[u8]> = tree.entries.iter().map(|e| e.name.as_slice()).collect();
    assert_eq!(names, [&b"a.txt"[..], b"libs", b"link", b"run.sh", b"sub"]);
    assert_eq!(tree.entries[3].mode, Some(0o755));
    assert!(matches!(&tree.entries[2].kind, EntryKind::Symlink { target } if target == b"a.txt"));

    let out = env.root.join("out.git");
    let exported = apeiron_git::export(&repo, &out, "git", &mut |_| Ok(())).unwrap();
    assert!(exported.inexact.is_empty(), "{exported:?}");
    assert_eq!((exported.commits, exported.tags), (5, 1));
    assert_eq!(env.refs(&out), env.refs(&src));
    env.git(&out, &["fsck", "--no-progress"]);
    assert_eq!(env.git(&out, &["symbolic-ref", "HEAD"]), "refs/heads/main");

    // Only what is new travels the second time, both ways.
    fs::write(src.join("a.txt"), "third\n").unwrap();
    env.git(&src, &["add", "a.txt"]);
    env.git(&src, &["commit", "-q", "-m", "more"]);
    let again = apeiron_git::import(&mut repo, &src, "git", &mut |_| Ok(())).unwrap();
    assert_eq!((again.commits, again.tags, again.refs.len()), (1, 0, 1));
    let exported = apeiron_git::export(&repo, &out, "git", &mut |_| Ok(())).unwrap();
    assert_eq!((exported.commits, exported.inexact.len()), (1, 0));
    assert_eq!(env.refs(&out), env.refs(&src));

    // A deleted branch stays, and is reported.
    env.git(&src, &["branch", "-q", "-D", "Feature/ABC-1"]);
    let again = apeiron_git::import(&mut repo, &src, "git", &mut |_| Ok(())).unwrap();
    assert_eq!(again.stale, ["git/heads/_46eature/_41_42_43-1"]);
}

#[test]
fn a_snapshot_made_in_apeiron_becomes_a_commit() {
    let Some(env) = Env::new() else { return };
    let mut repo = init(&env.root.join("apeiron"));
    let mut w = repo.writer();
    let (size, blob) = w.put_file(&b"backed up\n"[..]).unwrap();
    let tree = w
        .put_tree(&Tree {
            entries: vec![TreeEntry {
                name: b"notes.txt".to_vec(),
                kind: EntryKind::File { size, blob },
                mode: Some(0o600),
                mtime: None,
                uid: Some(1000),
                gid: Some(1000),
                xattrs: vec![],
            }],
        })
        .unwrap();
    let snapshot = w
        .put_snapshot(&Snapshot {
            tree,
            parents: vec![],
            time: Time {
                seconds: 1_700_000_000,
                nanos: 5,
            },
            author: Some("someone".to_owned()),
            host: Some("laptop".to_owned()),
            message: Some("backup".to_owned()),
            meta: vec![],
        })
        .unwrap();
    w.finish().unwrap();
    repo.update_ref("git/heads/backup", None, Some(snapshot))
        .unwrap();
    let id: ObjectId = snapshot;

    let out = env.root.join("out.git");
    let report = apeiron_git::export(&repo, &out, "git", &mut |_| Ok(())).unwrap();
    assert_eq!(report.refs[0].snapshot, id);
    env.git(&out, &["fsck", "--no-progress"]);
    let log = env.git(&out, &["log", "--format=%an <%ae>|%at|%s", "backup"]);
    assert_eq!(log, "someone <apeiron@localhost>|1700000000|backup");
    assert_eq!(
        env.git(&out, &["ls-tree", "backup"])
            .split_whitespace()
            .next(),
        Some("100644")
    );
    assert_eq!(
        env.git(&out, &["symbolic-ref", "HEAD"]),
        "refs/heads/backup"
    );
}

/// Runs on the repository named by `APEIRON_GIT_REAL`, such as a clone of
/// a real project; skipped otherwise.
#[test]
fn a_real_repository_round_trips() {
    let Some(src) = std::env::var_os("APEIRON_GIT_REAL").map(PathBuf::from) else {
        return;
    };
    let env = Env::new().unwrap();
    let mut repo = Repository::init(&env.root.join("apeiron")).unwrap();
    let started = std::time::Instant::now();
    let report = apeiron_git::import(&mut repo, &src, "git", &mut |_| Ok(())).unwrap();
    let imported = started.elapsed();
    let out = env.root.join("out.git");
    let exported = apeiron_git::export(&repo, &out, "git", &mut |_| Ok(())).unwrap();
    eprintln!(
        "{} commits, {} tags in {imported:?}; export in {:?}; skipped {:?}",
        report.commits,
        report.tags,
        started.elapsed() - imported,
        report.skipped
    );
    assert!(exported.inexact.is_empty(), "{:?}", exported.inexact);
    let wanted: Vec<String> = env
        .refs(&src)
        .lines()
        .filter(|line| {
            report
                .refs
                .iter()
                .any(|r| line.ends_with(&format!(" {}", r.git)))
        })
        .map(str::to_owned)
        .collect();
    assert_eq!(env.refs(&out).lines().collect::<Vec<_>>(), wanted);
    env.git(&out, &["fsck", "--no-progress"]);
}

#[test]
fn served_refs_and_packs_are_read_by_git() {
    let Some(env) = Env::new() else { return };
    let src = make_repo(&env);
    let mut repo = init(&env.root.join("apeiron"));
    apeiron_git::import(&mut repo, &src, "git", &mut |_| Ok(())).unwrap();

    let cache = env.root.join("cache.git");
    let refs = apeiron_git::serve_refs(&repo, &cache, "git").unwrap();
    let names: Vec<&str> = refs.iter().map(|r| r.git.as_str()).collect();
    assert_eq!(
        names,
        [
            "refs/heads/Feature/ABC-1",
            "refs/heads/main",
            "refs/heads/signed",
            "refs/tags/light",
            "refs/tags/v1.0",
        ]
    );
    let main = refs.iter().find(|r| r.git == "refs/heads/main").unwrap();
    assert_eq!(main.ref_name, "git/heads/main");

    // A pack of main's parent alone, read into an empty repository by git
    // itself; then only what main adds, for a client that has the parent.
    let parent = env.git(&src, &["rev-parse", "main^"]);
    let empty = env.root.join("empty.git");
    env.git(
        &env.root,
        &["init", "-q", "--bare", empty.to_str().unwrap()],
    );
    let mut pack = Vec::new();
    let all =
        apeiron_git::write_pack(&cache, std::slice::from_ref(&parent), &[], &mut pack).unwrap();
    env.git_with(&empty, &["index-pack", "--stdin", "--strict"], Some(&pack));

    let unknown = "0123456789012345678901234567890123456789".to_owned();
    let haves = [unknown, parent.clone()];
    let common = apeiron_git::common_haves(&cache, std::slice::from_ref(&main.id), &haves).unwrap();
    assert_eq!(common, [parent]);
    let mut pack = Vec::new();
    let new = apeiron_git::write_pack(&cache, std::slice::from_ref(&main.id), &common, &mut pack)
        .unwrap();
    assert!(new < all && new > 0, "{new} of {all}");
    env.git_with(&empty, &["index-pack", "--stdin", "--strict"], Some(&pack));
    env.git(&empty, &["update-ref", "refs/heads/main", &main.id]);
    env.git(&empty, &["fsck", "--no-progress"]);
    assert_eq!(
        env.git(&empty, &["rev-parse", "main"]),
        env.git(&src, &["rev-parse", "main"])
    );
}

#[test]
fn a_changed_file_travels_as_its_changes() {
    let Some(env) = Env::new() else { return };
    let src = env.root.join("big");
    fs::create_dir_all(&src).unwrap();
    env.git(&src, &["init", "-q", "-b", "main"]);
    // Letters zlib cannot squeeze much, changed a little in each commit.
    let mut x = 7u64;
    let mut text: Vec<u8> = (0..100_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            b'a' + (x % 26) as u8
        })
        .collect();
    for round in 0..3 {
        text[round * 1000] = b'#';
        text.extend_from_slice(format!("\nround {round}\n").as_bytes());
        fs::write(src.join("data.txt"), &text).unwrap();
        env.git(&src, &["add", "data.txt"]);
        env.git(&src, &["commit", "-q", "-m", &format!("round {round}")]);
    }
    let mut repo = init(&env.root.join("apeiron"));
    apeiron_git::import(&mut repo, &src, "git", &mut |_| Ok(())).unwrap();
    let cache = env.root.join("cache.git");
    let refs = apeiron_git::serve_refs(&repo, &cache, "git").unwrap();

    let mut pack = Vec::new();
    let count =
        apeiron_git::write_pack(&cache, std::slice::from_ref(&refs[0].id), &[], &mut pack).unwrap();
    assert_eq!(count, 9);
    // Three versions of 100 kB fit in little more than one.
    assert!(pack.len() < 90_000, "{}", pack.len());
    let empty = env.root.join("empty.git");
    env.git(
        &env.root,
        &["init", "-q", "--bare", empty.to_str().unwrap()],
    );
    env.git_with(&empty, &["index-pack", "--stdin", "--strict"], Some(&pack));
    env.git(&empty, &["update-ref", "refs/heads/main", &refs[0].id]);
    env.git(&empty, &["fsck", "--no-progress"]);
    assert_eq!(
        env.git(&empty, &["show", "main:data.txt"]).len(),
        env.git(&src, &["show", "main:data.txt"]).len()
    );
}

#[test]
fn pushed_commits_become_snapshots_and_refs_move_forward() {
    let Some(env) = Env::new() else { return };
    let src = make_repo(&env);
    let mut repo = init(&env.root.join("apeiron"));
    apeiron_git::import(&mut repo, &src, "git", &mut |_| Ok(())).unwrap();
    let cache = env.root.join("cache.git");
    let refs = apeiron_git::serve_refs(&repo, &cache, "git").unwrap();
    let old = refs
        .iter()
        .find(|r| r.git == "refs/heads/main")
        .unwrap()
        .id
        .clone();
    let zero = "0".repeat(40);

    // Two commits on main, sent as Git sends them, with offset deltas.
    for round in 0..2 {
        fs::write(src.join("a.txt"), format!("pushed {round}\n").repeat(50)).unwrap();
        env.git(&src, &["add", "a.txt"]);
        env.git(&src, &["commit", "-q", "-m", &format!("pushed {round}")]);
    }
    let new = env.git(&src, &["rev-parse", "main"]);
    let pack = |revs: &str, offsets: bool| {
        let mut args = vec!["pack-objects", "--stdout", "--revs", "-q"];
        if offsets {
            args.push("--delta-base-offset");
        }
        let out = Command::new("git")
            .args(&args)
            .current_dir(&src)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = out;
        out.stdin
            .take()
            .unwrap()
            .write_all(revs.as_bytes())
            .unwrap();
        out.wait_with_output().unwrap().stdout
    };
    let push = |repo: &mut Repository, git: &str, old: &str, new: &str, pack: &[u8]| {
        let update = apeiron_git::PushedRef {
            git: git.to_owned(),
            old: old.to_owned(),
            new: new.to_owned(),
        };
        let may = |name: &str| name != "git/heads/locked";
        let mut results = apeiron_git::receive(repo, &cache, "git", &[update], pack, &may).unwrap();
        results.remove(0).error
    };
    let sent = pack(&format!("{new}\n^{old}\n"), true);
    assert_eq!(push(&mut repo, "refs/heads/main", &old, &new, &sent), None);
    let main = repo.read_ref("git/heads/main").unwrap().unwrap();
    let snapshot = repo.read_snapshot(&main).unwrap();
    let id = snapshot.meta.iter().find(|(k, _)| k == "git.id").unwrap();
    assert_eq!(id.1, new);
    assert_eq!(snapshot.message.as_deref(), Some("pushed 1\n"));

    // A new branch, sent with deltas against named bases.
    env.git(&src, &["checkout", "-q", "-b", "topic"]);
    fs::write(src.join("a.txt"), "on topic\n".repeat(50)).unwrap();
    env.git(&src, &["commit", "-q", "-am", "topic"]);
    let topic = env.git(&src, &["rev-parse", "topic"]);
    let sent = pack(&format!("{topic}\n^{new}\n"), false);
    assert_eq!(
        push(&mut repo, "refs/heads/topic", &zero, &topic, &sent),
        None
    );
    assert!(repo.read_ref("git/heads/topic").unwrap().is_some());

    // Refused: a stale old id, a ref it may not write, deleting a ref that
    // is not there, and a remote-tracking branch.
    let refused = [
        ("refs/heads/main", old.as_str(), topic.as_str()),
        ("refs/heads/locked", zero.as_str(), topic.as_str()),
        ("refs/heads/gone", zero.as_str(), zero.as_str()),
        ("refs/remotes/origin/x", zero.as_str(), topic.as_str()),
    ];
    for (git, from, to) in refused {
        assert!(
            push(&mut repo, git, from, to, &[]).is_some(),
            "{git} {from} {to}"
        );
    }
    assert_eq!(repo.read_ref("git/heads/locked").unwrap(), None);

    // The server's copy shows the new commits with the ids Git made.
    let served = apeiron_git::serve_refs(&repo, &cache, "git").unwrap();
    let main = served.iter().find(|r| r.git == "refs/heads/main").unwrap();
    assert_eq!(main.id, new);
    env.git(&cache, &["fsck", "--no-progress"]);

    // Deleting a branch and moving one back are taken (GIT.md §7.1), but
    // not past a retention rule.
    assert_eq!(
        push(&mut repo, "refs/heads/topic", &topic, &zero, &[]),
        None
    );
    assert_eq!(repo.read_ref("git/heads/topic").unwrap(), None);
    let served = apeiron_git::serve_refs(&repo, &cache, "git").unwrap();
    assert!(served.iter().all(|r| r.git != "refs/heads/topic"));
    let back = env.git(&src, &["rev-parse", "main~1"]);
    assert_eq!(push(&mut repo, "refs/heads/main", &new, &back, &[]), None);
    repo.add_retention_rule("git/heads/main", 1).unwrap();
    assert!(push(&mut repo, "refs/heads/main", &back, &zero, &[]).is_some());
}
