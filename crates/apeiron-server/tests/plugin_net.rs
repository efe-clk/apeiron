//! Plugins that reach the network (PLUGIN.md §2.2, §2.3, §3.1): a storage
//! backend keeps a copy on a small server, and a hook tells it about each
//! backup, only to the hosts the user granted.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use apeiron::{BackupRequest, ErrorKind, Repository, TransferRequest};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};

/// A fixture plugin of apeiron-plugin's tests, built for wasm32-wasip2.
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

#[derive(Default)]
struct Server {
    files: BTreeMap<String, Vec<u8>>,
    refs: BTreeMap<String, String>,
    told: Vec<String>,
}

type Shared = State<Arc<Mutex<Server>>>;

async fn read(State(s): Shared, UrlPath(path): UrlPath<String>) -> Result<Vec<u8>, StatusCode> {
    s.lock()
        .unwrap()
        .files
        .get(&path)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)
}

async fn write(State(s): Shared, UrlPath(path): UrlPath<String>, body: Bytes) -> StatusCode {
    s.lock()
        .unwrap()
        .files
        .entry(path)
        .or_insert_with(|| body.to_vec());
    StatusCode::OK
}

async fn remove(State(s): Shared, UrlPath(path): UrlPath<String>) -> StatusCode {
    s.lock().unwrap().files.remove(&path);
    StatusCode::OK
}

async fn list(State(s): Shared, UrlPath(dir): UrlPath<String>) -> String {
    let prefix = format!("{dir}/");
    let mut names: Vec<String> = s
        .lock()
        .unwrap()
        .files
        .keys()
        .filter_map(|k| k.strip_prefix(&prefix))
        .map(|rest| rest.split('/').next().unwrap().to_owned())
        .collect();
    names.dedup();
    names.join("\n")
}

async fn read_ref(State(s): Shared, UrlPath(name): UrlPath<String>) -> Result<String, StatusCode> {
    s.lock()
        .unwrap()
        .refs
        .get(&name)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)
}

async fn refs(State(s): Shared) -> String {
    let names: Vec<String> = s.lock().unwrap().refs.keys().cloned().collect();
    names.join("\n")
}

async fn swap(
    State(s): Shared,
    UrlPath(name): UrlPath<String>,
    body: String,
) -> (StatusCode, String) {
    let (expected, new) = body.split_once('\n').unwrap();
    let mut s = s.lock().unwrap();
    let actual = s.refs.get(&name).cloned().unwrap_or_default();
    if actual != expected {
        return (StatusCode::CONFLICT, actual);
    }
    s.refs.insert(name, new.to_owned());
    (StatusCode::OK, String::new())
}

async fn tell(State(s): Shared, body: String) -> StatusCode {
    s.lock().unwrap().told.push(body);
    StatusCode::OK
}

/// The test server, on a thread of its own; its address.
fn start(state: Arc<Mutex<Server>>) -> String {
    let app = Router::new()
        .route("/x/files/{*path}", get(read).put(write).delete(remove))
        .route("/x/list/{*dir}", get(list))
        .route("/x/refs/{*name}", get(read_ref))
        .route("/x/refs-list", get(refs))
        .route("/x/swap/{*name}", post(swap))
        .route("/tell", post(tell))
        .with_state(state);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    format!("http://{address}")
}

#[test]
fn plugins_reach_only_the_hosts_they_were_granted() {
    let dir = tempfile::tempdir().unwrap();
    let plugins = dir.path().join("plugins");
    let state = Arc::new(Mutex::new(Server::default()));
    let server = start(state.clone());
    let allow = ["net:127.0.0.1".to_owned()];

    // A plugin that asks to reach the network is installed only if allowed.
    let storage = fixture("fixture-storage", "test_storage");
    let refused = apeiron::add_plugin(&storage, Some(&plugins)).unwrap_err();
    assert!(
        refused.message.contains("--allow net:127.0.0.1"),
        "{}",
        refused.message
    );
    let added = apeiron::add_plugin_allowing(&storage, &allow, Some(&plugins)).unwrap();
    assert_eq!(added.info.granted, allow);

    let (repo, _) =
        Repository::init_encrypted(dir.path().join("repo"), "correct horse battery").unwrap();
    repo.use_plugin_dir(&plugins);
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/a.txt"), "one").unwrap();
    let backup = || {
        repo.backup(BackupRequest::new(dir.path().join("src"), "main"))
            .unwrap()
            .wait()
            .unwrap()
    };
    let first = backup();

    // The storage backend keeps a whole copy on the server.
    repo.add_remote("store", &format!("plugin://test-storage/{server}/x"))
        .unwrap();
    let push = |r: &Repository| {
        r.push(TransferRequest {
            remote: "store".to_owned(),
            ref_name: "main".to_owned(),
            rate: None,
            replace: false,
        })
        .unwrap()
        .wait()
    };
    push(&repo).unwrap();
    assert_eq!(
        state.lock().unwrap().refs.get("main"),
        Some(&first.snapshot.to_hex())
    );
    assert!(state.lock().unwrap().files.contains_key("config"));

    // A ref someone else moved there is a conflict, as on any remote.
    state
        .lock()
        .unwrap()
        .refs
        .insert("main".to_owned(), "0".repeat(64));
    fs::write(dir.path().join("src/a.txt"), "two").unwrap();
    backup();
    assert_eq!(push(&repo).unwrap_err().kind, ErrorKind::Conflict);

    // A hook tells the address in its settings after each backup.
    let notify = fixture("fixture-notify", "test_notify");
    apeiron::add_plugin_allowing(&notify, &allow, Some(&plugins)).unwrap();
    apeiron::plugin_settings(
        "test-notify",
        Some(&format!("{server}/tell")),
        Some(&plugins),
    )
    .unwrap();
    let repo = Repository::unlock(dir.path().join("repo"), "correct horse battery").unwrap();
    repo.use_plugin_dir(&plugins);
    fs::write(dir.path().join("src/b.txt"), "new").unwrap();
    let done = repo
        .backup(BackupRequest::new(dir.path().join("src"), "main"))
        .unwrap()
        .wait()
        .unwrap();
    assert!(done.warnings.is_empty(), "{:?}", done.warnings);
    assert_eq!(state.lock().unwrap().told, ["main: b.txt"]);

    // A host that was not granted is refused; the backup stands, warned.
    apeiron::plugin_settings(
        "test-notify",
        Some("https://example.com/tell"),
        Some(&plugins),
    )
    .unwrap();
    let repo = Repository::unlock(dir.path().join("repo"), "correct horse battery").unwrap();
    repo.use_plugin_dir(&plugins);
    fs::write(dir.path().join("src/c.txt"), "more").unwrap();
    let warned = repo
        .backup(BackupRequest::new(dir.path().join("src"), "main"))
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(warned.warnings.len(), 1, "{:?}", warned.warnings);
    assert!(
        warned.warnings[0].message.contains("Denied"),
        "{}",
        warned.warnings[0].message
    );
    assert_eq!(repo.resolve("main").unwrap(), warned.snapshot);
    assert_eq!(state.lock().unwrap().told.len(), 1);
}
