//! The web server spoken to over HTTP and HTTPS, as a browser does (WEB.md).

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use apeiron::Repository;
use rustls::pki_types::pem::PemObject;

use apeiron_server::{Audit, Role, Shares, Tls, User, Users, Web, WebOptions, hash_password};
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse";

/// Hashing takes a moment; every test uses the same hash.
fn hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_password(PASSWORD).unwrap())
        .clone()
}

/// A users file in `dir` with the admin `root`.
fn users_file(dir: &Path) -> PathBuf {
    let file = dir.join("config/web.json");
    let mut users = Users::default();
    users
        .add(User {
            name: "root".to_owned(),
            password: hash(),
            role: Role::Admin,
            backups: Vec::new(),
            folders: Vec::new(),
            oidc: None,
        })
        .unwrap();
    users.save(&file).unwrap();
    file
}

struct Reply {
    status: u16,
    headers: String,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.body))
    }

    fn cookie(&self) -> Option<String> {
        let line = self
            .headers
            .lines()
            .find_map(|l| l.strip_prefix("set-cookie: "))?;
        Some(line.split(';').next()?.to_owned())
    }
}

struct Client {
    address: SocketAddr,
    tls: Option<Arc<rustls::ClientConfig>>,
    cookie: Option<String>,
}

impl Client {
    fn send(&self, method: &str, path: &str, body: Option<&Value>, extra: &[&str]) -> Reply {
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.address,
            body.len()
        );
        if let Some(cookie) = &self.cookie {
            head.push_str(&format!("Cookie: other=1; {cookie}\r\n"));
        }
        for line in extra {
            head.push_str(line);
            head.push_str("\r\n");
        }
        if !body.is_empty() {
            head.push_str("Content-Type: application/json\r\n");
        }
        head.push_str("\r\n");
        let tcp = TcpStream::connect(self.address).unwrap();
        let mut raw = Vec::new();
        match &self.tls {
            None => {
                let mut stream = tcp;
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(body.as_bytes()).unwrap();
                stream.read_to_end(&mut raw).unwrap();
            }
            Some(config) => {
                let name = "localhost".try_into().unwrap();
                let conn = rustls::ClientConnection::new(config.clone(), name).unwrap();
                let mut stream = rustls::StreamOwned::new(conn, tcp);
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(body.as_bytes()).unwrap();
                // The server closes without a TLS close message.
                let _ = stream.read_to_end(&mut raw);
            }
        }
        let raw = String::from_utf8(raw).unwrap();
        let (headers, body) = raw.split_once("\r\n\r\n").unwrap();
        Reply {
            status: headers[9..12].parse().unwrap(),
            headers: headers.to_lowercase(),
            body: body.to_owned(),
        }
    }

    fn post(&self, path: &str, body: Value) -> Reply {
        self.send("POST", path, Some(&body), &[])
    }

    fn get(&self, path: &str) -> Reply {
        self.send("GET", path, None, &[])
    }

    fn sign_in(&mut self) {
        self.sign_in_as("root");
    }

    fn sign_in_as(&mut self, name: &str) {
        let reply = self.post("/api/login", json!({"name": name, "password": PASSWORD}));
        assert_eq!(reply.status, 200, "{}", reply.body);
        self.cookie = reply.cookie();
    }

    /// Another browser, signed in as `name`.
    fn other(&self, name: &str) -> Client {
        let mut c = Client {
            address: self.address,
            tls: self.tls.clone(),
            cookie: None,
        };
        c.sign_in_as(name);
        c
    }

    fn kind(&self, path: &str, body: Value) -> (u16, String) {
        let reply = self.post(path, body);
        let kind = reply.json()["kind"].as_str().unwrap_or("").to_owned();
        (reply.status, kind)
    }

    fn run(&self, operation: &str, body: Value) -> Value {
        let started = self.post(&format!("/api/start/{operation}"), body);
        assert_eq!(started.status, 200, "{}", started.body);
        let id = started.json()["job"].as_u64().unwrap();
        loop {
            let polled = self.get(&format!("/api/jobs/{id}")).json();
            if polled["done"] == true {
                return polled;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A plain web server over a new repository that may use `dir/home`.
fn start(dir: &Path) -> (Web, Client, PathBuf) {
    let home = dir.join("home");
    fs::create_dir_all(home.join("src")).unwrap();
    fs::write(home.join("src/a.txt"), "hello").unwrap();
    let server = Web::start(WebOptions {
        repo: Repository::init(dir.join("repo")).unwrap(),
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: None,
        users: users_file(dir),
        audit: Audit::new(dir.join("audit.log")),
        shares: Shares::new(dir.join("shares.json")),
        folders: vec![home.clone()],
    })
    .unwrap();
    let client = Client {
        address: server.address(),
        tls: None,
        cookie: None,
    };
    (server, client, home.canonicalize().unwrap())
}

#[test]
fn nothing_without_signing_in() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, mut c, _) = start(dir.path());

    // The pages and the session state are open to all.
    assert_eq!(c.get("/").status, 200);
    assert_eq!(
        c.get("/api/session").json(),
        json!({"mode": "web", "signed_in": false, "oidc": false})
    );
    let refused = c.get("/api/state");
    assert_eq!(refused.status, 401);
    assert_eq!(refused.json()["sign_in"], true);
    assert_eq!(c.post("/api/call/refs", json!({})).status, 401);

    // A made-up session is no session.
    c.cookie = Some("apeiron_session=00".to_owned());
    assert_eq!(c.get("/api/state").status, 401);

    let wrong = c.post("/api/login", json!({"name": "root", "password": "nope"}));
    assert_eq!(
        (wrong.status, wrong.json()["kind"].as_str()),
        (401, Some("key"))
    );
    assert_eq!(wrong.cookie(), None);

    c.sign_in();
    let reply = c.post("/api/login", json!({"name": "root", "password": PASSWORD}));
    let set = reply.headers.lines().find(|l| l.starts_with("set-cookie:"));
    let set = set.unwrap();
    for part in ["httponly", "secure", "samesite=strict", "path=/"] {
        assert!(set.contains(part), "{set}");
    }
    assert_eq!(c.get("/api/session").json()["signed_in"], true);
    assert_eq!(c.get("/api/state").status, 200);

    // Another site cannot call, whatever cookie it holds.
    let host = c.address.to_string();
    let other = c.send("GET", "/api/state", None, &["Origin: https://evil.example"]);
    assert_eq!(other.status, 403);
    let same = format!("Origin: https://{host}");
    assert_eq!(c.send("GET", "/api/state", None, &[&same]).status, 200);

    // Signing out ends the session.
    let out = c.post("/api/logout", json!({}));
    assert!(out.headers.contains("max-age=0"));
    assert_eq!(c.get("/api/state").status, 401);
}

#[test]
fn guessing_pauses_sign_ins() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, mut c, _) = start(dir.path());
    for _ in 0..5 {
        assert_eq!(
            c.post("/api/login", json!({"name": "root", "password": "guess"}))
                .status,
            401
        );
    }
    // Even the right password waits now.
    let paused = c.post("/api/login", json!({"name": "root", "password": PASSWORD}));
    assert_eq!(
        (paused.status, paused.json()["kind"].as_str()),
        (429, Some("locked"))
    );
    assert!(
        c.post("/api/login", json!({"name": "root", "password": PASSWORD}))
            .cookie()
            .is_none()
    );
    c.cookie = None;
    assert_eq!(c.get("/api/state").status, 401);
}

#[test]
fn only_the_pages_operations_and_folders() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, mut c, home) = start(dir.path());
    c.sign_in();

    // The repository is fixed.
    let other = dir.path().join("other");
    for path in ["/api/open", "/api/init"] {
        let reply = c.post(path, json!({"path": other}));
        assert_eq!(
            (reply.status, reply.json()["kind"].as_str()),
            (400, Some("unsupported"))
        );
    }
    assert!(!other.exists());

    // Operations beyond the pages are refused.
    for (kind, operation) in [
        ("call", "add_remote"),
        ("call", "keys"),
        ("call", "plugins"),
        ("start", "export"),
        ("start", "gc"),
    ] {
        let reply = c.post(&format!("/api/{kind}/{operation}"), json!({}));
        assert_eq!(reply.json()["kind"], "unsupported", "{operation}");
    }

    // Backing up and restoring inside the allowed folder works.
    let backup = c.run(
        "backup",
        json!({"source": home.join("src"), "ref": "backup/src"}),
    );
    assert_eq!(backup["result"]["files"], 1, "{backup}");
    let restored = c.run(
        "restore",
        json!({"snapshot": "backup/src", "target": home.join("out/new")}),
    );
    assert_eq!(restored["result"]["files"], 1, "{restored}");
    assert_eq!(
        fs::read_to_string(home.join("out/new/a.txt")).unwrap(),
        "hello"
    );

    // Outside it, also by `..`, a link or a relative path, nothing happens.
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, home.join("link")).unwrap();
    let mut tries = vec![
        outside.join("x"),
        home.join("../outside/x"),
        home.join("missing/../../outside/x"),
        PathBuf::from("relative"),
    ];
    if cfg!(unix) {
        tries.push(home.join("link/x"));
    }
    for target in tries {
        let reply = c.post(
            "/api/start/restore",
            json!({"snapshot": "backup/src", "target": target}),
        );
        assert_eq!(
            (reply.status, reply.json()["kind"].as_str()),
            (400, Some("invalid_input")),
            "{}",
            target.display()
        );
    }
    let reply = c.post(
        "/api/start/backup",
        json!({"source": outside, "ref": "backup/out"}),
    );
    assert_eq!(reply.status, 400);
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);

    // The folder browser starts at the allowed folder and stays inside it.
    let listing = c.get("/api/folders").json();
    assert_eq!(listing["path"], json!(home));
    assert_eq!(listing["parent"], Value::Null);
    let encoded = outside.to_str().unwrap().replace('/', "%2F");
    assert_eq!(c.get(&format!("/api/folders?path={encoded}")).status, 400);
}

#[test]
fn tls_with_a_kept_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let names = vec!["localhost".to_owned()];
    let tls = Tls::kept_in(dir.path(), names.clone()).unwrap();
    // The second start finds the same certificate.
    let again = Tls::kept_in(dir.path(), names).unwrap();
    assert_eq!(tls.fingerprint().unwrap(), again.fingerprint().unwrap());
    assert_eq!(tls.fingerprint().unwrap().len(), 32 * 3 - 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.path().join("web-key.pem"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let server = Web::start(WebOptions {
        repo: Repository::init(dir.path().join("repo")).unwrap(),
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: Some(tls.clone()),
        users: users_file(dir.path()),
        audit: Audit::new(dir.path().join("audit.log")),
        shares: Shares::new(dir.path().join("shares.json")),
        folders: vec![dir.path().to_path_buf()],
    })
    .unwrap();

    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls::pki_types::CertificateDer::pem_slice_iter(&tls.cert_pem) {
        roots.add(cert.unwrap()).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut c = Client {
        address: server.address(),
        tls: Some(Arc::new(config)),
        cookie: None,
    };
    c.sign_in();
    assert_eq!(c.get("/api/state").json()["open"]["encrypted"], false);

    // Plain HTTP to a TLS port gets nothing.
    let mut tcp = TcpStream::connect(server.address()).unwrap();
    tcp.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut raw = Vec::new();
    let _ = tcp.read_to_end(&mut raw);
    assert!(!String::from_utf8_lossy(&raw).contains("200 OK"));
}

#[test]
fn the_users_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config/web.json");
    assert_eq!(Users::load(&file).unwrap(), Users::default());
    assert!(Users::default().need_admin().is_err());
    assert_eq!(users_file(dir.path()), file);
    let mut users = Users::load(&file).unwrap();
    assert!(
        users
            .get("root")
            .unwrap()
            .password
            .starts_with("$argon2id$")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Names are checked and unique, and the last admin stays.
    let user = |name: &str| User {
        name: name.to_owned(),
        password: hash(),
        role: Role::Member,
        backups: vec!["backup/a-*".to_owned()],
        folders: Vec::new(),
        oidc: None,
    };
    assert!(users.add(user("Bad Name")).is_err());
    assert!(users.add(user("root")).is_err());
    users.add(user("ayse")).unwrap();
    assert!(users.remove("root").is_err());
    assert!(users.remove("nobody").is_err());
    users.remove("ayse").unwrap();
    assert!(hash_password("short").is_err());

    let ayse = user("ayse");
    assert!(ayse.owns("backup/a-photos"));
    assert!(!ayse.owns("backup/b"));
    assert!(ayse.verify(PASSWORD));
    assert!(!ayse.verify("wrong"));

    fs::write(&file, "not json").unwrap();
    assert!(Users::load(&file).is_err());
}

#[test]
fn each_user_sees_and_uses_only_their_own() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, mut root, home) = start(dir.path());
    for folder in ["ayse/docs", "root"] {
        fs::create_dir_all(home.join(folder)).unwrap();
        fs::write(home.join(folder).join("f.txt"), folder).unwrap();
    }
    let file = dir.path().join("config/web.json");
    let mut users = Users::load(&file).unwrap();
    let user = |name: &str, role, backups: &[&str], folders: Vec<PathBuf>| User {
        name: name.to_owned(),
        password: hash(),
        role,
        backups: backups.iter().map(|b| (*b).to_owned()).collect(),
        folders,
        oidc: None,
    };
    users
        .add(user(
            "ayse",
            Role::Member,
            &["backup/ayse-*"],
            vec![home.join("ayse")],
        ))
        .unwrap();
    users
        .add(user(
            "okan",
            Role::Reader,
            &["backup/ayse-docs"],
            Vec::new(),
        ))
        .unwrap();
    users.save(&file).unwrap();

    // The admin backs up both folders, the root one twice.
    root.sign_in();
    let backup = |c: &Client, folder: &str, r: &str| {
        let done = c.run("backup", json!({"source": home.join(folder), "ref": r}));
        assert!(done["result"]["snapshot"].is_string(), "{done}");
        done["result"]["snapshot"].as_str().unwrap().to_owned()
    };
    backup(&root, "ayse/docs", "backup/ayse-docs");
    let first_root = backup(&root, "root", "backup/root");
    fs::write(home.join("root/g.txt"), "more").unwrap();
    backup(&root, "root", "backup/root");

    let ayse = root.other("ayse");
    let session = ayse.get("/api/session").json();
    assert_eq!(session["user"], json!({"name": "ayse", "role": "member"}));

    // Only her backups, by name or by id, also through history.
    let refs = ayse.post("/api/call/refs", json!({})).json();
    assert_eq!(refs.as_array().unwrap().len(), 1);
    assert_eq!(refs[0]["name"], "backup/ayse-docs");
    assert_eq!(
        ayse.kind("/api/call/list", json!({"snapshot": "backup/ayse-docs"}))
            .0,
        200
    );
    for spec in [
        "backup/root",
        first_root.as_str(),
        "backup/root~1",
        "nothing",
    ] {
        assert_eq!(
            ayse.kind("/api/call/list", json!({"snapshot": spec})),
            (404, "not_found".to_owned()),
            "{spec}"
        );
    }
    assert_eq!(
        ayse.kind(
            "/api/call/diff",
            json!({"from": first_root, "to": "backup/ayse-docs"})
        )
        .0,
        404
    );
    assert_eq!(
        ayse.kind("/api/call/reflog", json!({"name": "backup/root"}))
            .0,
        404
    );
    assert_eq!(
        ayse.kind("/api/start/search", json!({"pattern": "*"})).0,
        400
    );
    assert_eq!(
        ayse.kind(
            "/api/start/search",
            json!({"snapshot": "backup/root", "pattern": "*"})
        )
        .0,
        404
    );
    let found = ayse.run(
        "search",
        json!({"snapshot": "backup/ayse-docs", "pattern": "*"}),
    );
    assert!(found["result"]["hits"].is_array(), "{found}");

    // She backs up and restores only within her backups and folders.
    assert_eq!(
        ayse.kind(
            "/api/start/backup",
            json!({"source": home.join("ayse/docs"), "ref": "backup/root"})
        ),
        (400, "invalid_input".to_owned())
    );
    assert_eq!(
        ayse.kind(
            "/api/start/backup",
            json!({"source": home.join("root"), "ref": "backup/ayse-root"})
        ),
        (400, "invalid_input".to_owned())
    );
    backup(&ayse, "ayse/docs", "backup/ayse-copy");
    assert_eq!(
        ayse.kind(
            "/api/start/restore",
            json!({"snapshot": "backup/ayse-docs", "target": home.join("root/out")})
        ),
        (400, "invalid_input".to_owned())
    );
    let restored = ayse.run(
        "restore",
        json!({"snapshot": "backup/ayse-docs", "target": home.join("ayse/out")}),
    );
    assert_eq!(restored["result"]["files"], 1, "{restored}");
    assert_eq!(
        ayse.kind("/api/start/verify", json!({})),
        (400, "unsupported".to_owned())
    );
    assert_eq!(
        ayse.get("/api/folders").json()["path"],
        json!(home.join("ayse"))
    );

    // Nobody follows another user's job.
    let started = root.post("/api/start/verify", json!({}));
    let id = started.json()["job"].as_u64().unwrap();
    assert_eq!(ayse.get(&format!("/api/jobs/{id}")).status, 404);
    assert_eq!(
        ayse.post(&format!("/api/jobs/{id}/cancel"), json!({}))
            .status,
        404
    );

    // A reader only looks.
    let okan = root.other("okan");
    let refs = okan.post("/api/call/refs", json!({})).json();
    assert_eq!(refs.as_array().unwrap().len(), 1);
    assert_eq!(
        okan.kind("/api/call/list", json!({"snapshot": "backup/ayse-docs"}))
            .0,
        200
    );
    for (operation, body) in [
        (
            "backup",
            json!({"source": home.join("ayse/docs"), "ref": "backup/ayse-docs"}),
        ),
        (
            "restore",
            json!({"snapshot": "backup/ayse-docs", "target": home.join("x")}),
        ),
    ] {
        assert_eq!(
            okan.kind(&format!("/api/start/{operation}"), body),
            (400, "unsupported".to_owned())
        );
    }
    assert_eq!(okan.get("/api/folders").status, 400);

    // An unknown name fails like a wrong password.
    let unknown = root.post(
        "/api/login",
        json!({"name": "nobody", "password": PASSWORD}),
    );
    assert_eq!(unknown.status, 401);

    // Changes to the file count at once: a new password and a removal end
    // the sessions.
    let mut users = Users::load(&file).unwrap();
    users.get_mut("okan").unwrap().password = hash_password("another password").unwrap();
    users.remove("ayse").unwrap();
    users.save(&file).unwrap();
    assert_eq!(okan.get("/api/state").status, 401);
    assert_eq!(ayse.get("/api/state").status, 401);
    assert_eq!(root.get("/api/state").status, 200);
}

#[test]
fn every_request_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, mut root, home) = start(dir.path());
    let audit = Audit::new(dir.path().join("audit.log"));

    assert_eq!(
        root.post(
            "/api/login",
            json!({"name": "root", "password": "wrong one"})
        )
        .status,
        401
    );
    root.sign_in();
    root.post("/api/call/refs", json!({}));
    let done = root.run(
        "backup",
        json!({"source": home.join("src"), "ref": "backup/src"}),
    );
    assert!(done["result"].is_object());
    let outside = dir.path().join("elsewhere");
    let refused = root.post(
        "/api/start/restore",
        json!({"snapshot": "backup/src", "target": outside}),
    );
    assert_eq!(refused.status, 400);
    root.post("/api/logout", json!({}));

    let events = audit.read(100, None).unwrap();
    let seen: Vec<(Option<&str>, &str, &str)> = events
        .iter()
        .map(|e| (e.user.as_deref(), e.event.as_str(), e.outcome.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            (None, "sign_in", "key"),
            (Some("root"), "sign_in", "ok"),
            (Some("root"), "call:refs", "ok"),
            (Some("root"), "start:backup", "ok"),
            (Some("root"), "done:backup", "ok"),
            (Some("root"), "start:restore", "invalid_input"),
            (Some("root"), "sign_out", "ok"),
        ]
    );
    assert_eq!(events[0].detail, json!({"name": "root"}));
    assert_eq!(events[3].detail["ref"], "backup/src");
    assert!(events.iter().all(|e| {
        e.from
            .as_deref()
            .is_some_and(|f| f.starts_with("127.0.0.1:"))
    }));
    let text = fs::read_to_string(audit.file()).unwrap();
    assert!(!text.contains(PASSWORD) && !text.contains("wrong one"));
    assert_eq!(audit.verify().unwrap().events, 7);

    // Only an admin reads it, newest first.
    root.sign_in();
    let read = root.get("/api/audit?limit=2").json();
    assert_eq!(read[0]["event"], "sign_in");
    assert_eq!(read[1]["event"], "sign_out");
    let mut users = Users::load(&dir.path().join("config/web.json")).unwrap();
    users
        .add(User {
            name: "ayse".to_owned(),
            password: hash(),
            role: Role::Member,
            backups: vec!["backup/src".to_owned()],
            folders: Vec::new(),
            oidc: None,
        })
        .unwrap();
    users.save(&dir.path().join("config/web.json")).unwrap();
    let ayse = root.other("ayse");
    let reply = ayse.get("/api/audit");
    assert_eq!(reply.json()["kind"], "unsupported");

    // Nothing happens unrecorded: with the log unwritable, requests fail.
    fs::remove_file(audit.file()).unwrap();
    fs::create_dir(audit.file()).unwrap();
    let failed = root.post("/api/call/refs", json!({}));
    assert_eq!(failed.status, 500, "{}", failed.body);
    let failed = root.post(
        "/api/start/backup",
        json!({"source": home.join("src"), "ref": "backup/src"}),
    );
    assert_eq!(failed.status, 500);
    assert_eq!(
        root.post("/api/login", json!({"name": "root", "password": PASSWORD}))
            .status,
        500
    );
}

#[test]
fn share_links() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, mut root, home) = start(dir.path());
    for folder in ["ayse/docs/sub", "root"] {
        fs::create_dir_all(home.join(folder)).unwrap();
    }
    fs::write(home.join("ayse/docs/f.txt"), "first").unwrap();
    fs::write(home.join("ayse/docs/sub/ş ğ.txt"), "second").unwrap();
    let big: String = (0..300_000).map(|i| format!("{i:07}\n")).collect();
    fs::write(home.join("ayse/docs/sub/big.txt"), &big).unwrap();
    fs::write(home.join("root/r.txt"), "root").unwrap();
    let file = dir.path().join("config/web.json");
    let mut users = Users::load(&file).unwrap();
    for (name, role) in [("ayse", Role::Member), ("okan", Role::Reader)] {
        users
            .add(User {
                name: name.to_owned(),
                password: hash(),
                role,
                backups: vec!["backup/ayse-*".to_owned()],
                folders: Vec::new(),
                oidc: None,
            })
            .unwrap();
    }
    users.save(&file).unwrap();
    root.sign_in();
    for (folder, r) in [("ayse/docs", "backup/ayse-docs"), ("root", "backup/root")] {
        let done = root.run("backup", json!({"source": home.join(folder), "ref": r}));
        assert!(done["result"]["snapshot"].is_string(), "{done}");
    }
    let ayse = root.other("ayse");
    let okan = root.other("okan");

    // Only what the user sees, for 1 to 30 days, and not by a reader.
    let share = |c: &Client, body: Value| c.post("/api/shares", body);
    let refused = |c: &Client, body: Value| {
        let reply = share(c, body);
        (
            reply.status,
            reply.json()["kind"].as_str().unwrap().to_owned(),
        )
    };
    let not_found = (404, "not_found".to_owned());
    let invalid = (400, "invalid_input".to_owned());
    assert_eq!(
        refused(&ayse, json!({"snapshot": "backup/root"})),
        not_found
    );
    assert_eq!(
        refused(
            &ayse,
            json!({"snapshot": "backup/ayse-docs", "path": "missing"})
        ),
        not_found
    );
    assert_eq!(
        refused(
            &ayse,
            json!({"snapshot": "backup/ayse-docs", "path": "sub/.."})
        ),
        invalid
    );
    assert_eq!(
        refused(&ayse, json!({"snapshot": "backup/ayse-docs", "days": 31})),
        invalid
    );
    assert_eq!(
        refused(&okan, json!({"snapshot": "backup/ayse-docs"})),
        (400, "unsupported".to_owned())
    );
    let made = share(&ayse, json!({"snapshot": "backup/ayse-docs", "days": 1})).json();
    let folder = made["secret"].as_str().unwrap().to_owned();
    assert_eq!(folder.len(), 64);
    assert!(made["share"].get("hash").is_none(), "{made}");
    let made = share(
        &ayse,
        json!({"snapshot": "backup/ayse-docs", "path": "/f.txt"}),
    )
    .json();
    let one = made["secret"].as_str().unwrap().to_owned();
    let one_id = made["share"]["id"].as_str().unwrap().to_owned();

    // Anyone with the link reads what it names and nothing else.
    let stranger = Client {
        address: root.address,
        tls: None,
        cookie: None,
    };
    let page = stranger.get(&format!("/s/{folder}"));
    assert_eq!(page.status, 200);
    assert!(page.headers.contains("referrer-policy: no-referrer"));
    let listed = stranger.get(&format!("/api/shared/{folder}")).json();
    let names: Vec<&str> = listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["f.txt", "sub"]);
    assert!(listed["time"].is_object(), "{listed}");
    let sub = stranger
        .get(&format!("/api/shared/{folder}?path=sub"))
        .json();
    assert_eq!(sub["entries"][1]["name"], "ş ğ.txt");
    // A large file comes whole, as it is read.
    let got = stranger.get(&format!("/api/shared/{folder}/file?path=sub/big.txt"));
    assert!(got.body == big, "{} of {} bytes", got.body.len(), big.len());
    let got = stranger.get(&format!(
        "/api/shared/{folder}/file?path=sub/%C5%9F%20%C4%9F.txt"
    ));
    assert_eq!((got.status, got.body.as_str()), (200, "second"));
    assert!(
        got.headers
            .contains("content-disposition: attachment; filename*=utf-8''%c5%9f%20%c4%9f.txt"),
        "{}",
        got.headers
    );
    for path in ["..", "sub/../..", "sub/../../root"] {
        let reply = stranger.get(&format!("/api/shared/{folder}?path={path}"));
        assert_eq!(reply.status, 404, "{path}");
    }
    let folder_file = stranger.get(&format!("/api/shared/{folder}/file?path=sub"));
    assert_eq!(folder_file.status, 400);
    let single = stranger.get(&format!("/api/shared/{one}")).json();
    assert_eq!(single["entry"]["kind"], "file");
    assert_eq!(single["entries"], json!([]));
    assert_eq!(
        stranger.get(&format!("/api/shared/{one}/file")).body,
        "first"
    );
    assert_eq!(
        stranger
            .get(&format!("/api/shared/{}", "0".repeat(64)))
            .status,
        404
    );
    // The signed-in API stays closed to the link holder.
    assert_eq!(stranger.get("/api/shares").status, 401);

    // Each user sees and removes their own links; an admin all.
    let ids = |c: &Client| -> Vec<String> {
        c.get("/api/shares")
            .json()
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(ids(&ayse).len(), 2);
    assert_eq!(ids(&root).len(), 2);
    assert!(ids(&okan).is_empty());
    let remove = |c: &Client, id: &str| {
        c.post(&format!("/api/shares/{id}/remove"), json!({}))
            .status
    };
    assert_eq!(remove(&okan, &one_id), 404);
    assert_eq!(remove(&ayse, &one_id), 200);
    assert_eq!(stranger.get(&format!("/api/shared/{one}")).status, 404);

    // A link ends when its time is up.
    let shares_file = dir.path().join("shares.json");
    let mut kept: Value = serde_json::from_slice(&fs::read(&shares_file).unwrap()).unwrap();
    assert!(!fs::read_to_string(&shares_file).unwrap().contains(&folder));
    let expires = kept["shares"][0]["expires"].clone();
    kept["shares"][0]["expires"] = json!("2020-01-01T00:00:00Z");
    fs::write(&shares_file, kept.to_string()).unwrap();
    assert_eq!(stranger.get(&format!("/api/shared/{folder}")).status, 404);
    kept["shares"][0]["expires"] = expires;
    fs::write(&shares_file, kept.to_string()).unwrap();
    assert_eq!(stranger.get(&format!("/api/shared/{folder}")).status, 200);

    // And when its owner may no longer share what it names.
    let mut users = Users::load(&file).unwrap();
    users.get_mut("ayse").unwrap().role = Role::Reader;
    users.save(&file).unwrap();
    assert_eq!(stranger.get(&format!("/api/shared/{folder}")).status, 404);

    // Every use of a link is recorded, without the secret.
    let log = fs::read_to_string(dir.path().join("audit.log")).unwrap();
    assert!(!log.contains(&folder) && !log.contains(&one));
    for event in ["share_add", "share_remove", "shared:list", "shared:read"] {
        assert!(log.contains(&format!("\"event\":\"{event}\"")), "{event}");
    }
}
