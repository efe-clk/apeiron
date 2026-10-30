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

use apeiron_server::{Tls, Web, WebOptions, hash_password, load_password, save_password};
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse";

/// Hashing takes a moment; every test uses the same hash.
fn hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_password(PASSWORD).unwrap())
        .clone()
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
        let reply = self.post("/api/login", json!({"password": PASSWORD}));
        assert_eq!(reply.status, 200, "{}", reply.body);
        self.cookie = reply.cookie();
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
        password_hash: hash(),
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
        json!({"mode": "web", "signed_in": false})
    );
    let refused = c.get("/api/state");
    assert_eq!(refused.status, 401);
    assert_eq!(refused.json()["sign_in"], true);
    assert_eq!(c.post("/api/call/refs", json!({})).status, 401);

    // A made-up session is no session.
    c.cookie = Some("apeiron_session=00".to_owned());
    assert_eq!(c.get("/api/state").status, 401);

    let wrong = c.post("/api/login", json!({"password": "nope"}));
    assert_eq!(
        (wrong.status, wrong.json()["kind"].as_str()),
        (401, Some("key"))
    );
    assert_eq!(wrong.cookie(), None);

    c.sign_in();
    let reply = c.post("/api/login", json!({"password": PASSWORD}));
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
            c.post("/api/login", json!({"password": "guess"})).status,
            401
        );
    }
    // Even the right password waits now.
    let paused = c.post("/api/login", json!({"password": PASSWORD}));
    assert_eq!(
        (paused.status, paused.json()["kind"].as_str()),
        (429, Some("locked"))
    );
    assert!(
        c.post("/api/login", json!({"password": PASSWORD}))
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
        password_hash: hash(),
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
fn the_password_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config/web.json");
    assert_eq!(load_password(&file).unwrap(), None);
    save_password(&file, &hash()).unwrap();
    assert_eq!(load_password(&file).unwrap(), Some(hash()));
    assert!(hash().starts_with("$argon2id$"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    fs::write(&file, "not json").unwrap();
    assert!(load_password(&file).is_err());
}
