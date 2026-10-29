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
