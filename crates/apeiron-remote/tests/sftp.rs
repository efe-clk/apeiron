//! SFTP remotes against a real OpenSSH server, started for each test as the
//! current user. Without `sshd` the tests are skipped, unless
//! `APEIRON_SFTP_TEST` is set, as it is in CI.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use apeiron_core::{
    ChunkerParams, EntryKind, Error, ObjectId, Repository, SftpUrl, Snapshot, Store, Time, Tree,
    TreeEntry,
};
use apeiron_remote::{SftpStore, SshOptions};

const PASSWORD: &str = "pw";

struct Server {
    dir: tempfile::TempDir,
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn find_sshd() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([PathBuf::from("/usr/sbin"), PathBuf::from("/usr/local/sbin")])
        .map(|dir| dir.join("sshd"))
        .find(|p| p.is_file())
}

fn keygen(path: &Path) {
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

/// Starts `sshd` on a free port, or returns `None` to skip the test.
fn server() -> Option<Server> {
    let Some(sshd) = find_sshd() else {
        assert!(
            std::env::var_os("APEIRON_SFTP_TEST").is_none_or(|v| v.is_empty()),
            "APEIRON_SFTP_TEST is set but sshd was not found"
        );
        eprintln!("skipped: sshd not found");
        return None;
    };
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    keygen(&d.join("host_key"));
    keygen(&d.join("user_key"));
    fs::copy(d.join("user_key.pub"), d.join("authorized_keys")).unwrap();
    fs::create_dir(d.join("data")).unwrap();
    // Tests run in parallel and may pick the same free port; an sshd that
    // cannot listen exits, and the next port is tried. sshd writes its pid
    // file only once it listens.
    let (child, port) = (0..10)
        .find_map(|_| start_sshd(&sshd, d))
        .expect("sshd did not start");
    let host_key = fs::read_to_string(d.join("host_key.pub")).unwrap();
    fs::write(
        d.join("known_hosts"),
        format!("[127.0.0.1]:{port} {host_key}"),
    )
    .unwrap();
    Some(Server { dir, child, port })
}

/// Starts sshd on a port that was free a moment ago; `None` if it exited.
fn start_sshd(sshd: &Path, d: &Path) -> Option<(Child, u16)> {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = format!(
        "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/host_key\n\
         AuthorizedKeysFile {d}/authorized_keys\nPidFile {d}/sshd.pid\n\
         PasswordAuthentication no\nKbdInteractiveAuthentication no\n\
         StrictModes no\nUsePAM no\nSubsystem sftp internal-sftp\n",
        d = d.display()
    );
    fs::write(d.join("sshd_config"), config).unwrap();
    let _ = fs::remove_file(d.join("sshd.pid"));
    let mut child = Command::new(sshd)
        .arg("-D")
        .arg("-e")
        .arg("-f")
        .arg(d.join("sshd_config"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !d.join("sshd.pid").exists() {
        if child.try_wait().unwrap().is_some() || start.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Some((child, port))
}

impl Server {
    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn url(&self, folder: &str) -> SftpUrl {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap();
        SftpUrl {
            user: Some(user),
            host: "127.0.0.1".to_owned(),
            port: self.port,
            path: self.path(folder).to_str().unwrap().to_owned(),
        }
    }

    fn options(&self) -> SshOptions {
        SshOptions {
            known_hosts: self.path("known_hosts"),
            identity_files: vec![self.path("user_key")],
            use_agent: false,
        }
    }

    fn store(&self, folder: &str) -> SftpStore {
        SftpStore::connect(&self.url(folder), &self.options()).unwrap()
    }
}

/// 300 KiB that do not repeat (xorshift).
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

fn init(path: &Path) -> Repository {
    let params = ChunkerParams::new(64 * 1024).unwrap();
    Repository::init_encrypted(path, params, PASSWORD)
        .unwrap()
        .0
}

fn copy_dir(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn push_and_pull_over_sftp() {
    let Some(server) = server() else { return };
    let mut a = init(&server.path("a"));
    let first = commit(&mut a, 1, None);
    let store = server.store("data/remote");
    let report = a.push(&store, "main", &mut |_| Ok(())).unwrap();
    assert!(report.objects > 5);
    assert_eq!(a.push(&store, "main", &mut |_| Ok(())).unwrap().objects, 0);

    // The server folder is a complete repository.
    let remote = server.path("data/remote");
    let copy = Repository::unlock(&remote, PASSWORD).unwrap();
    assert_eq!(content(&copy, &first), data(1));
    assert!(copy.verify().unwrap().is_ok());
    assert!(fs::read_dir(remote.join("tmp")).unwrap().next().is_none());

    copy_dir(&server.path("a"), &server.path("b"));
    let mut b = Repository::unlock(&server.path("b"), PASSWORD).unwrap();
    let second = commit(&mut a, 2, Some(first));
    a.push(&store, "main", &mut |_| Ok(())).unwrap();
    let report = b.pull(&store, "main", &mut |_| Ok(())).unwrap();
    assert_eq!((report.snapshot, report.previous), (second, Some(first)));
    assert_eq!(content(&b, &second), data(2));
    assert!(b.verify().unwrap().is_ok());
}

#[test]
fn sftp_refs_are_compare_and_swap_with_locks() {
    let Some(server) = server() else { return };
    let mut a = init(&server.path("a"));
    let first = commit(&mut a, 1, None);
    let store = server.store("data/remote");
    a.push(&store, "main", &mut |_| Ok(())).unwrap();
    let other = ObjectId::from_bytes([7; 32]);

    assert!(matches!(
        store.swap_ref("main", None, other),
        Err(Error::RefConflict { .. })
    ));
    let lock = server.path("data/remote/refs/main.lock");
    fs::write(&lock, b"").unwrap();
    assert!(matches!(
        store.swap_ref("main", Some(first), other),
        Err(Error::RefLocked { .. })
    ));
    // A lock left behind long ago by a crash is stale.
    let old = SystemTime::now() - Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_modified(old)
        .unwrap();
    store.swap_ref("main", Some(first), other).unwrap();
    assert_eq!(store.read_ref("main").unwrap(), Some(other));
    assert!(!lock.exists());
    assert_eq!(store.read_ref("other").unwrap(), None);
    store.swap_ref("sync/docs", None, other).unwrap();
    fs::write(server.path("data/remote/refs/sync/docs.lock"), b"").unwrap();
    assert_eq!(store.list_refs().unwrap(), ["main", "sync/docs"]);

    store.write_new("keys/x.key", b"one").unwrap();
    store.write_new("keys/x.key", b"two").unwrap();
    assert_eq!(store.read("keys/x.key").unwrap().unwrap(), b"one");
    assert_eq!(store.read_range("keys/x.key", 1, 2).unwrap(), b"ne");
    store.delete("keys/x.key").unwrap();
    store.delete("keys/x.key").unwrap();
    assert_eq!(store.read("keys/x.key").unwrap(), None);
    assert!(store.list("nothing").unwrap().is_empty());
}

#[test]
fn unknown_or_changed_servers_and_wrong_keys_are_refused() {
    let Some(server) = server() else { return };
    let url = server.url("data/remote");
    let message = |options: &SshOptions| SftpStore::connect(&url, options).unwrap_err().to_string();

    let mut options = server.options();
    options.known_hosts = server.path("missing_known_hosts");
    fs::write(&options.known_hosts, "").unwrap();
    assert!(
        message(&options).contains("is not in"),
        "{}",
        message(&options)
    );

    keygen(&server.path("other_key"));
    let other = fs::read_to_string(server.path("other_key.pub")).unwrap();
    fs::write(
        &options.known_hosts,
        format!("[127.0.0.1]:{} {other}", server.port),
    )
    .unwrap();
    assert!(
        message(&options).contains("differs"),
        "{}",
        message(&options)
    );

    let mut options = server.options();
    options.identity_files = vec![server.path("other_key")];
    assert!(message(&options).contains("accepted no SSH key"));
}

#[test]
fn an_interrupted_push_over_sftp_is_run_again() {
    let Some(server) = server() else { return };
    let mut a = init(&server.path("a"));
    let first = commit(&mut a, 1, None);
    let second = commit(&mut a, 2, Some(first));
    let remote = server.path("data/remote");
    for stop_after in [1, 4, 9] {
        let store = server.store("data/remote");
        let mut copied = 0;
        let result = a.push(&store, "main", &mut |_| {
            copied += 1;
            if copied == stop_after {
                return Err(Error::Cancelled);
            }
            Ok(())
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        drop(store);
        let copy = Repository::unlock(&remote, PASSWORD).unwrap();
        assert_eq!(copy.read_ref("main").unwrap(), None);
    }
    let store = server.store("data/remote");
    a.push(&store, "main", &mut |_| Ok(())).unwrap();
    let copy = Repository::unlock(&remote, PASSWORD).unwrap();
    assert_eq!(content(&copy, &second), data(2));
    assert_eq!(content(&copy, &first), data(1));
    assert!(copy.verify().unwrap().is_ok());
}
