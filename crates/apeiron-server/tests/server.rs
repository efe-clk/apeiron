//! The local server spoken to over HTTP, as the pages do (DESKTOP.md §3, §4).

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use apeiron_server::{Local, Options};
use serde_json::{Value, json};

struct Client {
    host: String,
    token: String,
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
}

impl Client {
    fn new(server: &Local) -> Self {
        let rest = server.url().strip_prefix("http://").unwrap();
        let (host, fragment) = rest.split_once("/#token=").unwrap();
        Client {
            host: host.to_owned(),
            token: fragment.to_owned(),
        }
    }

    /// Sends a request with `extra` header lines; the token unless `extra`
    /// replaces the defaults.
    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        extra: &[&str],
        defaults: bool,
    ) -> Reply {
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        if defaults {
            head.push_str(&format!(
                "Host: {}\r\nAuthorization: Bearer {}\r\n",
                self.host, self.token
            ));
        }
        for line in extra {
            head.push_str(line);
            head.push_str("\r\n");
        }
        if !body.is_empty() {
            head.push_str("Content-Type: application/json\r\n");
        }
        head.push_str("\r\n");
        let mut stream = TcpStream::connect(&self.host).unwrap();
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body.as_bytes()).unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let (headers, body) = raw.split_once("\r\n\r\n").unwrap();
        let status = headers[9..12].parse().unwrap();
        Reply {
            status,
            headers: headers.to_lowercase(),
            body: body.to_owned(),
        }
    }

    fn post(&self, path: &str, body: Value) -> Reply {
        self.send("POST", path, Some(&body), &[], true)
    }

    fn get(&self, path: &str) -> Reply {
        self.send("GET", path, None, &[], true)
    }

    /// Starts a job and polls it until it ends.
    fn run(&self, operation: &str, body: Value) -> Value {
        let started = self.post(&format!("/api/start/{operation}"), body);
        assert_eq!(started.status, 200, "{}", started.body);
        let id = started.json()["job"].as_u64().unwrap();
        loop {
            let polled = self.get(&format!("/api/jobs/{id}")).json();
            if polled["done"] == true {
                assert_eq!(self.get(&format!("/api/jobs/{id}")).status, 404);
                return polled;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

fn start(dir: &std::path::Path) -> (Local, Client) {
    let server = Local::start(Options {
        repo: None,
        memory: Some(dir.join("memory.json")),
    })
    .unwrap();
    let client = Client::new(&server);
    (server, client)
}

#[test]
fn only_the_pages_get_in() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, c) = start(dir.path());
    let host = format!("Host: {}", c.host);

    // The pages need no token and carry a strict content policy.
    let page = c.send("GET", "/", None, &[&host], false);
    assert_eq!(page.status, 200);
    assert!(
        page.headers
            .contains("content-security-policy: default-src 'self'")
    );
    assert_eq!(c.send("GET", "/missing", None, &[&host], false).status, 404);

    // The API needs the token.
    let reply = c.send("GET", "/api/state", None, &[&host], false);
    assert_eq!(
        (reply.status, reply.json()["kind"].as_str()),
        (401, Some("invalid_input"))
    );
    let wrong = c.send(
        "GET",
        "/api/state",
        None,
        &[&host, "Authorization: Bearer 00"],
        false,
    );
    assert_eq!(wrong.status, 401);

    // Another Host (DNS rebinding) or another Origin is refused.
    let auth = format!("Authorization: Bearer {}", c.token);
    let other_host = c.send(
        "GET",
        "/api/state",
        None,
        &["Host: evil.example", &auth],
        false,
    );
    assert_eq!(other_host.status, 403);
    let other_origin = c.get_with("/api/state", "Origin: http://evil.example");
    assert_eq!(other_origin.status, 403);
    let same_origin = c.get_with("/api/state", &format!("Origin: http://{}", c.host));
    assert_eq!(same_origin.status, 200);
}

impl Client {
    fn get_with(&self, path: &str, line: &str) -> Reply {
        self.send("GET", path, None, &[line], true)
    }
}

#[test]
fn create_back_up_list_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, c) = start(dir.path());
    let src = dir.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("a.txt"), "hello").unwrap();

    assert_eq!(
        c.get("/api/state").json(),
        json!({"open": null, "last": null})
    );
    let none = c.post("/api/call/refs", json!({}));
    assert_eq!(
        (none.status, none.json()["kind"].as_str()),
        (404, Some("not_found"))
    );

    let repo = dir.path().join("repo");
    let made = c.post("/api/init", json!({"path": repo}));
    assert_eq!(made.status, 200, "{}", made.body);
    assert_eq!(made.json()["recovery_key"], Value::Null);
    let state = c.get("/api/state").json();
    assert_eq!(state["last"], json!(repo));
    assert_eq!(state["open"]["encrypted"], false);

    let backup = c.run("backup", json!({"source": src, "ref": "backup/src"}));
    assert_eq!(backup["result"]["files"], 1, "{backup}");
    let list = c
        .post("/api/call/list", json!({"snapshot": "backup/src"}))
        .json();
    assert_eq!(list[0]["name"], "a.txt");

    let restored = c.run(
        "restore",
        json!({"snapshot": "backup/src", "target": dir.path().join("out")}),
    );
    assert_eq!(restored["result"]["files"], 1);
    assert_eq!(
        fs::read_to_string(dir.path().join("out/a.txt")).unwrap(),
        "hello"
    );

    // Errors keep their kind, in a call and in a job.
    let missing = c.post("/api/call/resolve", json!({"spec": "nope"}));
    assert_eq!(
        (missing.status, missing.json()["kind"].as_str()),
        (404, Some("not_found"))
    );
    let unknown = c.post("/api/call/nonsense", json!({}));
    assert_eq!(unknown.status, 400);
    let failed = c.run("search", json!({"snapshot": "backup/src"}));
    assert_eq!(failed["error"]["kind"], "invalid_input");
    assert_eq!(c.get("/api/jobs/999").status, 404);
    assert_eq!(c.post("/api/call/list", json!("not an object")).status, 400);
}

#[test]
fn encrypted_repositories_need_their_password() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, c) = start(dir.path());
    let repo = dir.path().join("repo");
    let made = c
        .post("/api/init", json!({"path": repo, "password": "pw"}))
        .json();
    assert!(made["recovery_key"].as_str().unwrap().len() > 20);

    let (_again, c2) = start(dir.path());
    let locked = c2.post("/api/open", json!({"path": repo}));
    assert_eq!(
        (locked.status, locked.json()["kind"].as_str()),
        (401, Some("key"))
    );
    assert_eq!(
        c2.post("/api/open", json!({"path": repo, "password": "no"}))
            .status,
        401
    );
    let opened = c2.post("/api/open", json!({"path": repo, "password": "pw"}));
    assert_eq!(
        (opened.status, opened.json()["encrypted"].as_bool()),
        (200, Some(true))
    );
    // The last repository is remembered for the next start.
    assert_eq!(c2.get("/api/state").json()["last"], json!(repo));
}

#[test]
fn folders_can_be_browsed() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, c) = start(dir.path());
    for name in ["b", "A", ".hidden"] {
        fs::create_dir(dir.path().join(name)).unwrap();
    }
    fs::write(dir.path().join("file.txt"), "x").unwrap();
    let path = dir.path().to_str().unwrap();
    let encoded: String = path
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let listing = c.get(&format!("/api/folders?path={encoded}")).json();
    assert_eq!(listing["folders"], json!(["A", "b"]));
    assert_eq!(listing["parent"], json!(dir.path().parent().unwrap()));
    assert_eq!(c.get("/api/folders?path=%2Fno%2Fsuch%2Ffolder").status, 404);
}

#[test]
fn conflict_copies_are_found_and_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, c) = start(dir.path());
    let made = c.post("/api/init", json!({"path": dir.path().join("repo")}));
    assert_eq!(made.status, 200, "{}", made.body);
    let folder = dir.path().join("docs");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("a.txt"), "here").unwrap();
    fs::write(folder.join("a (conflict pc 2026-10-31 10-08).txt"), "there").unwrap();

    let found = c
        .post("/api/call/conflicts", json!({"folder": folder}))
        .json();
    assert_eq!(
        found,
        json!([{"path": "a.txt", "copy": "a (conflict pc 2026-10-31 10-08).txt", "exists": true}])
    );
    let resolved = c.post(
        "/api/call/resolve_conflict",
        json!({"folder": folder, "copy": found[0]["copy"], "keep": "copy"}),
    );
    assert_eq!(resolved.status, 200, "{}", resolved.body);
    assert_eq!(fs::read_to_string(folder.join("a.txt")).unwrap(), "there");
    assert_eq!(
        c.post("/api/call/conflicts", json!({"folder": folder}))
            .json(),
        json!([])
    );
}
