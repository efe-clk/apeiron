//! The plugin host with a real component (PLUGIN.md §3). The fixture plugin
//! is built for wasm32-wasip2 on first use.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use apeiron_plugin::{Error, Plugins};
use apeiron_services::ContentMerger;

fn fixture() -> &'static Path {
    static WASM: OnceLock<PathBuf> = OnceLock::new();
    WASM.get_or_init(|| {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = here.join("../../target/plugin-fixture");
        let status = Command::new(env!("CARGO"))
            .args(["build", "--release", "--target", "wasm32-wasip2", "--manifest-path"])
            .arg(here.join("tests/fixture/Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "building the fixture plugin failed; is the target installed? rustup target add wasm32-wasip2"
        );
        target.join("wasm32-wasip2/release/test_driver.wasm")
    })
}

fn installed() -> (tempfile::TempDir, Plugins) {
    let dir = tempfile::tempdir().unwrap();
    let mut plugins = Plugins::open(dir.path().join("plugins"))
        .unwrap()
        .with_time_limit(Duration::from_secs(1));
    let added = plugins.add(fixture()).unwrap();
    assert_eq!(added.info.name, "test-driver");
    assert!(!added.replaced);
    (dir, plugins)
}

#[test]
fn install_list_reopen_and_remove() {
    let (dir, mut plugins) = installed();
    assert!(plugins.add(fixture()).unwrap().replaced);
    let names: Vec<&str> = plugins.list().iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["test-driver"]);
    assert!(dir.path().join("plugins/test-driver.wasm").exists());

    let reopened = Plugins::open(dir.path().join("plugins")).unwrap();
    assert_eq!(reopened.list(), plugins.list());
    assert!(reopened.handles(b"A.MERGE"));
    assert!(!reopened.handles(b"a.txt"));
    let merged = reopened
        .merge_file(b"a.merge", Some(b"b"), b"o", b"t")
        .unwrap()
        .unwrap();
    assert_eq!(
        (merged.data.as_slice(), merged.plugin.as_str()),
        (&b"b|o|t"[..], "test-driver")
    );

    assert_eq!(plugins.remove("test-driver").unwrap().name, "test-driver");
    assert!(plugins.list().is_empty());
    assert!(!dir.path().join("plugins/test-driver.wasm").exists());
    assert!(matches!(
        plugins.remove("test-driver"),
        Err(Error::NotFound(_))
    ));
    assert!(
        Plugins::open(dir.path().join("plugins"))
            .unwrap()
            .list()
            .is_empty()
    );
}

#[test]
fn files_the_plugin_does_not_handle_are_left_alone() {
    let (_dir, plugins) = installed();
    assert_eq!(
        plugins.merge_file(b"a.txt", None, b"o", b"t").unwrap(),
        None
    );
    let merged = plugins
        .merge_file(b"x.merge", None, b"o", b"t")
        .unwrap()
        .unwrap();
    assert_eq!(merged.data, b"-|o|t");
    assert_eq!(
        plugins
            .merge_file(b"x.merge", None, b"conflict", b"t")
            .unwrap(),
        None
    );
    assert_eq!(
        plugins
            .diff_file(b"x.merge", b"ab", b"abc")
            .unwrap()
            .as_deref(),
        Some("2 -> 3 bytes")
    );
}

#[test]
fn the_plugin_sees_nothing_of_the_system() {
    let (_dir, plugins) = installed();
    let merged = plugins
        .merge_file(b"x.merge", None, b"sandbox", b"t")
        .unwrap()
        .unwrap();
    assert_eq!(
        String::from_utf8(merged.data).unwrap(),
        "env=0 args=0 files=false"
    );
}

#[test]
fn crashes_timeouts_memory_and_refusals_are_errors() {
    let (_dir, plugins) = installed();
    let fail = |ours: &[u8]| match plugins.merge(b"x.merge", None, ours, b"t") {
        Err(message) => message,
        Ok(m) => panic!("{ours:?} merged: {m:?}"),
    };
    assert!(fail(b"trap").contains("crashed"), "{}", fail(b"trap"));
    let start = Instant::now();
    let message = fail(b"loop");
    assert!(message.contains("took longer than 1 s"), "{message}");
    assert!(start.elapsed() < Duration::from_secs(10));
    let message = fail(b"alloc");
    assert!(message.contains("more than 512 MiB"), "{message}");
    assert!(fail(b"refuse").contains("refused on purpose"));
    // The plugin still works after each failure.
    assert!(
        plugins
            .merge(b"x.merge", None, b"o", b"t")
            .unwrap()
            .is_some()
    );
}

#[test]
fn files_that_are_not_plugins_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.wasm");
    std::fs::write(&file, b"\0asm not really").unwrap();
    let mut plugins = Plugins::open(dir.path().join("plugins")).unwrap();
    let err = plugins.add(&file).unwrap_err().to_string();
    assert!(err.contains("not a WebAssembly component"), "{err}");
    assert!(plugins.list().is_empty());
    assert!(!dir.path().join("plugins").exists());
}
