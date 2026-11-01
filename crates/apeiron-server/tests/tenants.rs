//! Several tenants served by one web server, each at its own host
//! (TENANTS.md).

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::thread;
use std::time::Duration;

use apeiron::Repository;
use apeiron_server::{Audit, Limits, Role, Shares, TenantOptions, User, Users, Web, hash_password};
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse";

/// Sends a request with `Host: host` and returns the status, the session
/// cookie set and the body.
fn send(
    address: SocketAddr,
    host: &str,
    cookie: Option<&str>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Option<String>, Value) {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n",
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

/// A tenant named `name` at `host`, in `dir`, whose admin `root` has the
/// test password, with a folder holding one file.
fn tenant(dir: &Path, name: &str, host: &str) -> TenantOptions {
    let dir = dir.join(name);
    let home = dir.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("a.txt"), name).unwrap();
    let users = dir.join("web.json");
    let mut list = Users::default();
    list.add(User {
        name: "root".to_owned(),
        password: hash_password(PASSWORD).unwrap(),
        role: Role::Admin,
        backups: Vec::new(),
        folders: Vec::new(),
        oidc: None,
        totp: None,
    })
    .unwrap();
    list.save(&users).unwrap();
    TenantOptions {
        name: name.to_owned(),
        host: host.to_owned(),
        repo: Repository::init(dir.join("repo")).unwrap(),
        users,
        folders: vec![home],
        audit: Audit::new(dir.join("audit.log")),
        shares: Shares::new(dir.join("shares.json")),
        replicas: Vec::new(),
        standby: false,
        limits: Limits::default(),
        operator: false,
    }
}

fn sign_in(address: SocketAddr, host: &str) -> String {
    let (status, cookie, body) = send(
        address,
        host,
        None,
        "POST",
        "/api/login",
        Some(json!({"name": "root", "password": PASSWORD})),
    );
    assert_eq!(status, 200, "{body}");
    cookie.unwrap()
}

#[test]
fn each_tenant_is_apart() {
    let dir = tempfile::tempdir().unwrap();
    let acme = tenant(dir.path(), "acme", "acme.test");
    let globex = tenant(dir.path(), "globex", "globex.test");
    let acme_home = acme.folders[0].canonicalize().unwrap();
    let server = Web::start_tenants(
        "127.0.0.1:0".parse().unwrap(),
        None,
        vec![acme, globex],
        None,
    )
    .unwrap();
    let a = server.address();

    // No site answers at another host, not even the pages.
    for host in [a.to_string(), "other.test".to_owned()] {
        assert_eq!(send(a, &host, None, "GET", "/", None).0, 421);
        assert_eq!(send(a, &host, None, "GET", "/api/session", None).0, 421);
    }
    assert_eq!(send(a, "acme.test", None, "GET", "/", None).0, 200);
    // The port and the case of the host do not matter.
    assert_eq!(send(a, "ACME.test:8443", None, "GET", "/", None).0, 200);

    // A session of one tenant is no session of the other.
    let acme_cookie = sign_in(a, "acme.test");
    let ok = send(
        a,
        "acme.test",
        Some(&acme_cookie),
        "GET",
        "/api/state",
        None,
    );
    assert_eq!(ok.0, 200);
    let other = send(
        a,
        "globex.test",
        Some(&acme_cookie),
        "GET",
        "/api/state",
        None,
    );
    assert_eq!(other.0, 401);
    let globex_cookie = sign_in(a, "globex.test");

    // A backup in one tenant's repository is not in the other's.
    let (status, _, started) = send(
        a,
        "acme.test",
        Some(&acme_cookie),
        "POST",
        "/api/start/backup",
        Some(json!({"source": acme_home, "ref": "backup/acme"})),
    );
    assert_eq!(status, 200, "{started}");
    let id = started["job"].as_u64().unwrap();
    loop {
        let path = format!("/api/jobs/{id}");
        let (_, _, polled) = send(a, "acme.test", Some(&acme_cookie), "GET", &path, None);
        if polled["done"] == true {
            assert!(polled["error"].is_null(), "{polled}");
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let refs = |host: &str, cookie: &str| {
        let (_, _, r) = send(
            a,
            host,
            Some(cookie),
            "POST",
            "/api/call/refs",
            Some(json!({})),
        );
        r.as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(refs("acme.test", &acme_cookie), ["backup/acme"]);
    assert!(refs("globex.test", &globex_cookie).is_empty());

    // The folders of one tenant are not the other's.
    let (status, _, refused) = send(
        a,
        "globex.test",
        Some(&globex_cookie),
        "POST",
        "/api/start/backup",
        Some(json!({"source": acme_home, "ref": "backup/x"})),
    );
    assert_eq!(status, 400, "{refused}");

    // Each tenant's audit log holds only its own events.
    let read = |name: &str| {
        Audit::new(dir.path().join(name).join("audit.log"))
            .read(100, None)
            .unwrap()
    };
    let backups = |name: &str| {
        read(name)
            .into_iter()
            .filter(|e| e.event == "start:backup")
            .map(|e| e.outcome)
            .collect::<Vec<_>>()
    };
    assert_eq!(backups("acme"), ["ok"]);
    assert_eq!(backups("globex"), ["invalid_input"]);
}

#[test]
fn tenants_are_checked_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    assert!(Web::start_tenants(listen, None, Vec::new(), None).is_err());
    let one = tenant(dir.path(), "one", "same.test");
    let two = tenant(dir.path(), "two", "SAME.test");
    assert!(Web::start_tenants(listen, None, vec![one, two], None).is_err());
    // Every tenant needs an admin.
    let lonely = tenant(dir.path(), "lonely", "lonely.test");
    Users::default().save(&lonely.users).unwrap();
    let e = Web::start_tenants(listen, None, vec![lonely], None)
        .err()
        .unwrap();
    assert!(e.message.starts_with("tenant lonely: "), "{}", e.message);
}

/// A tenant whose repository holds its limit takes no new backup
/// (TENANTS.md §2.1).
#[test]
fn a_full_tenant_takes_no_backup() {
    let dir = tempfile::tempdir().unwrap();
    let mut acme = tenant(dir.path(), "acme", "acme.test");
    acme.limits.max_bytes = Some(1);
    let home = acme.folders[0].canonicalize().unwrap();
    let server =
        Web::start_tenants("127.0.0.1:0".parse().unwrap(), None, vec![acme], None).unwrap();
    let a = server.address();
    let cookie = sign_in(a, "acme.test");
    let (status, _, refused) = send(
        a,
        "acme.test",
        Some(&cookie),
        "POST",
        "/api/start/backup",
        Some(json!({"source": home, "ref": "backup/acme"})),
    );
    assert_eq!(status, 400, "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("its limit is 1"),
        "{refused}"
    );
    let log = fs::read_to_string(dir.path().join("acme/audit.log")).unwrap();
    assert!(
        log.contains("\"start:backup\"") && log.contains("\"unsupported\""),
        "{log}"
    );
}

/// An admin of the operator tenant adds, limits and removes tenants while
/// the server runs (TENANTS.md §4.1).
#[test]
fn the_operator_manages_tenants_in_the_pages() {
    let dir = tempfile::tempdir().unwrap();
    let mut ops = tenant(dir.path(), "ops", "ops.test");
    ops.operator = true;
    let acme = tenant(dir.path(), "acme", "acme.test");
    let file = dir.path().join("tenants.json");
    let listed = json!({"tenants": [
        {"name": "ops", "host": "ops.test", "repo": dir.path().join("ops/repo"), "operator": true},
        {"name": "acme", "host": "acme.test", "repo": dir.path().join("acme/repo")},
    ]});
    fs::write(&file, listed.to_string()).unwrap();
    let manage = apeiron_server::Manage {
        file: file.clone(),
        config: dir.path().join("config"),
        state: dir.path().join("state"),
        standby: false,
        syslog: None,
    };
    let server = Web::start_tenants(
        "127.0.0.1:0".parse().unwrap(),
        None,
        vec![ops, acme],
        Some(manage),
    )
    .unwrap();
    let a = server.address();
    let ops_cookie = sign_in(a, "ops.test");
    let acme_cookie = sign_in(a, "acme.test");

    // Only the operator's admin sees the tenants.
    let (status, _, session) = send(
        a,
        "ops.test",
        Some(&ops_cookie),
        "GET",
        "/api/session",
        None,
    );
    assert_eq!((status, &session["operator"]), (200, &json!(true)));
    let (status, _, _) = send(
        a,
        "acme.test",
        Some(&acme_cookie),
        "GET",
        "/api/tenants",
        None,
    );
    assert_eq!(status, 400);
    let (status, _, list) = send(
        a,
        "ops.test",
        Some(&ops_cookie),
        "GET",
        "/api/tenants",
        None,
    );
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["tenants"].as_array().unwrap().len(), 2);

    // A new tenant with a new repository is served at once.
    let new = json!({
        "name": "globex", "host": "globex.test", "repo": dir.path().join("globex/repo"),
        "admin": "root", "password": PASSWORD, "max_jobs": 2,
    });
    let (status, _, body) = send(
        a,
        "ops.test",
        Some(&ops_cookie),
        "POST",
        "/api/tenants",
        Some(new),
    );
    assert_eq!(status, 200, "{body}");
    let globex_cookie = sign_in(a, "globex.test");
    let (status, _, _) = send(
        a,
        "globex.test",
        Some(&globex_cookie),
        "GET",
        "/api/state",
        None,
    );
    assert_eq!(status, 200);
    assert!(dir.path().join("config/tenants/globex/web.json").exists());
    let saved: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(saved["tenants"][2]["max_jobs"], 2, "{saved}");

    // Limits change at once and in the file.
    let (status, _, body) = send(
        a,
        "ops.test",
        Some(&ops_cookie),
        "POST",
        "/api/tenants/globex/limits",
        Some(json!({"max_bytes": 1, "max_jobs": 0})),
    );
    assert_eq!(status, 200, "{body}");
    let (status, _, refused) = send(
        a,
        "globex.test",
        Some(&globex_cookie),
        "POST",
        "/api/start/backup",
        Some(json!({"source": dir.path(), "ref": "backup/x"})),
    );
    assert_eq!(status, 400, "{refused}");
    let saved: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(saved["tenants"][2]["max_bytes"], 1, "{saved}");
    assert!(saved["tenants"][2].get("max_jobs").is_none(), "{saved}");

    // Removing stops serving it; the operator cannot remove itself.
    let (status, _, _) = send(
        a,
        "ops.test",
        Some(&ops_cookie),
        "POST",
        "/api/tenants/ops/remove",
        None,
    );
    assert_eq!(status, 400);
    let (status, _, _) = send(
        a,
        "ops.test",
        Some(&ops_cookie),
        "POST",
        "/api/tenants/globex/remove",
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(send(a, "globex.test", None, "GET", "/", None).0, 421);
    assert!(!fs::read_to_string(&file).unwrap().contains("globex"));

    let log = fs::read_to_string(dir.path().join("ops/audit.log")).unwrap();
    for event in ["tenant_add", "tenant_limit", "tenant_remove"] {
        assert!(log.contains(event), "{event}: {log}");
    }
}
