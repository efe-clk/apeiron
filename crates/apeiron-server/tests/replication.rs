//! A web server that keeps a replica up to date, and a standby serving
//! that replica (REPLICATION.md).

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use apeiron::Repository;
use apeiron_server::{Audit, Role, Shares, User, Users, Web, WebOptions, hash_password};
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse";

/// Sends a request and returns the status, the session cookie set and the
/// body.
fn send(
    address: SocketAddr,
    cookie: Option<&str>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Option<String>, Value) {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(cookie) = cookie {
        head.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    head.push_str("\r\n");
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let (headers, body) = raw.split_once("\r\n\r\n").unwrap();
    let cookie = headers
        .lines()
        .find_map(|l| {
            l.to_lowercase()
                .strip_prefix("set-cookie: ")
                .map(str::to_owned)
        })
        .and_then(|l| l.split(';').next().map(str::to_owned));
    (
        headers[9..12].parse().unwrap(),
        cookie,
        serde_json::from_str(body).unwrap_or(Value::Null),
    )
}

/// A server over `repo` in `dir`, whose admin `root` may use `dir/home`.
fn serve(dir: &Path, repo: Repository, replicas: &[&str], standby: bool) -> (Web, String) {
    let home = dir.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("a.txt"), "hello").unwrap();
    let users = dir.join("web.json");
    let mut list = Users::default();
    list.add(User {
        name: "root".to_owned(),
        password: hash_password(PASSWORD).unwrap(),
        role: Role::Admin,
        backups: Vec::new(),
        folders: Vec::new(),
        oidc: None,
    })
    .unwrap();
    list.save(&users).unwrap();
    let server = Web::start(WebOptions {
        repo,
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: None,
        users,
        folders: vec![home],
        audit: Audit::new(dir.join("audit.log")),
        shares: Shares::new(dir.join("shares.json")),
        replicas: replicas.iter().map(|r| r.to_string()).collect(),
        standby,
    })
    .unwrap();
    let (status, cookie, body) = send(
        server.address(),
        None,
        "POST",
        "/api/login",
        Some(json!({"name": "root", "password": PASSWORD})),
    );
    assert_eq!(status, 200, "{body}");
    (server, cookie.unwrap())
}

fn backup(address: SocketAddr, cookie: &str, source: &Path) -> Value {
    let (status, _, started) = send(
        address,
        Some(cookie),
        "POST",
        "/api/start/backup",
        Some(json!({"source": source, "ref": "backup/home"})),
    );
    if status != 200 {
        return started;
    }
    let id = started["job"].as_u64().unwrap();
    loop {
        let path = format!("/api/jobs/{id}");
        let (_, _, polled) = send(address, Some(cookie), "GET", &path, None);
        if polled["done"] == true {
            return polled;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until `check` holds, for at most ten seconds.
fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if check() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn a_backup_reaches_the_replica_and_the_standby_serves_it() {
    let dir = tempfile::tempdir().unwrap();
    let (primary_dir, standby_dir) = (dir.path().join("primary"), dir.path().join("standby"));
    let replica: PathBuf = dir.path().join("replica");
    let repo = Repository::init(primary_dir.join("repo")).unwrap();
    repo.add_remote("dr", replica.to_str().unwrap()).unwrap();

    // A replica that is not a remote of the repository is refused.
    let unknown = Web::start(WebOptions {
        repo: repo.clone(),
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: None,
        users: primary_dir.join("none.json"),
        folders: Vec::new(),
        audit: Audit::new(primary_dir.join("x.log")),
        shares: Shares::new(primary_dir.join("x.json")),
        replicas: vec!["nowhere".to_owned()],
        standby: false,
    });
    assert!(unknown.is_err());

    let (primary, on_primary) = serve(&primary_dir, repo, &["dr"], false);
    let a = primary.address();
    let (status, _, health) = send(a, None, "GET", "/api/health", None);
    assert_eq!(
        (status, health),
        (200, json!({"ok": true, "role": "primary"}))
    );

    let done = backup(a, &on_primary, &primary_dir.join("home"));
    assert!(done["error"].is_null(), "{done}");
    let snapshot = done["result"]["snapshot"].as_str().unwrap().to_owned();
    // The backup is pushed to the replica without waiting for a round.
    assert!(eventually(|| {
        Repository::open(&replica)
            .and_then(|r| r.resolve("backup/home"))
            .is_ok_and(|id| id.to_string() == snapshot)
    }));
    let audit = Audit::new(primary_dir.join("audit.log"));
    assert!(eventually(|| {
        audit
            .read(100, None)
            .unwrap()
            .iter()
            .any(|e| e.event == "replicate" && e.outcome == "ok" && e.detail["remote"] == "dr")
    }));

    // A standby serves the replica: it can be browsed and restored from,
    // but takes no backups.
    let (standby, on_standby) = serve(&standby_dir, Repository::open(&replica).unwrap(), &[], true);
    let b = standby.address();
    let (status, _, health) = send(b, None, "GET", "/api/health", None);
    assert_eq!(
        (status, health),
        (200, json!({"ok": true, "role": "standby"}))
    );
    let (_, _, refs) = send(
        b,
        Some(&on_standby),
        "POST",
        "/api/call/refs",
        Some(json!({})),
    );
    assert_eq!(refs[0]["snapshot"], json!(snapshot));
    let refused = backup(b, &on_standby, &standby_dir.join("home"));
    assert_eq!(refused["kind"], "unsupported", "{refused}");
    assert!(
        refused["message"].as_str().unwrap().contains("standby"),
        "{refused}"
    );

    // A later backup on the primary shows on the standby with no restart.
    fs::write(primary_dir.join("home/b.txt"), "more").unwrap();
    let later = backup(a, &on_primary, &primary_dir.join("home"));
    let later = later["result"]["snapshot"].clone();
    assert!(eventually(|| {
        let (_, _, refs) = send(
            b,
            Some(&on_standby),
            "POST",
            "/api/call/refs",
            Some(json!({})),
        );
        refs[0]["snapshot"] == later
    }));
}
