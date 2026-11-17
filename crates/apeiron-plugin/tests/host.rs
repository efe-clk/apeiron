//! The plugin host with a real component (PLUGIN.md §3). The fixture plugin
//! is built for wasm32-wasip2 on first use.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use apeiron_plugin::{Error, Plugins};
use apeiron_services::ContentMerger;
use apeiron_services::TextExtractor;

fn fixture() -> &'static Path {
    static WASM: OnceLock<PathBuf> = OnceLock::new();
    WASM.get_or_init(|| build("fixture", "test_driver"))
}

/// Builds the fixture plugin in `tests/<folder>` and gives its file.
fn build(folder: &str, file: &str) -> PathBuf {
    {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = here.join("../../target/plugin-fixture");
        let status = Command::new(env!("CARGO"))
            .args([
                "build",
                "--release",
                "--target",
                "wasm32-wasip2",
                "--manifest-path",
            ])
            .arg(here.join("tests").join(folder).join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "building the fixture plugin failed; is the target installed? rustup target add wasm32-wasip2"
        );
        target.join(format!("wasm32-wasip2/release/{file}.wasm"))
    }
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

#[test]
fn text_comes_from_the_plugin_that_reads_the_file() {
    let (_dir, plugins) = installed();
    assert!(plugins.reads_text(b"a.extract"));
    assert!(!plugins.reads_text(b"a.merge"));
    assert_eq!(
        plugins.extract_file(b"a.extract", b"inside").unwrap(),
        Some("INSIDE".to_owned())
    );
    assert_eq!(plugins.extract_file(b"a.extract", b"none").unwrap(), None);
    assert_eq!(plugins.extract_file(b"a.other", b"inside").unwrap(), None);
    assert!(matches!(
        plugins.extract_file(b"a.extract", b"refuse"),
        Err(Error::Failed { .. })
    ));
    assert!(matches!(
        plugins.extract_file(b"a.extract", b"trap"),
        Err(Error::Failed { .. })
    ));
    // A crash leaves the next call as it would be.
    assert_eq!(
        plugins.extract_file(b"a.extract", b"again").unwrap(),
        Some("AGAIN".to_owned())
    );
}

/// An importer reads its archives; a crash or a refusal is an error
/// (PLUGIN.md §2.1).
#[test]
fn an_importer_reads_its_archives() {
    use apeiron_plugin::Kind;
    use apeiron_services::ArchiveReader;
    let dir = tempfile::tempdir().unwrap();
    let mut plugins = Plugins::open(dir.path().join("plugins"))
        .unwrap()
        .with_time_limit(Duration::from_secs(1));
    let added = plugins
        .add(&build("fixture-importer", "test_importer"))
        .unwrap();
    assert_eq!(added.info.kind, Kind::Importer);
    assert!(plugins.reads(b"mail.lines"));
    assert!(!plugins.reads(b"a.merge"));
    // An importer merges nothing.
    assert!(!plugins.handles(b"a.lines"));
    let (by, files) = plugins
        .read(b"a.lines", b"a.txt=one\nsub/b.txt=two")
        .unwrap();
    assert_eq!(by, "test-importer");
    assert_eq!(
        files,
        [
            (b"a.txt".to_vec(), b"one".to_vec()),
            (b"sub/b.txt".to_vec(), b"two".to_vec())
        ]
    );
    assert!(
        plugins
            .read(b"a.lines", b"refuse")
            .unwrap_err()
            .contains("refused")
    );
    assert!(
        plugins
            .read(b"a.lines", b"trap")
            .unwrap_err()
            .contains("crashed")
    );
    let reopened = Plugins::open(dir.path().join("plugins")).unwrap();
    assert_eq!(reopened.list()[0].kind, Kind::Importer);
}

/// A hook warns, refuses, and is taken as a refusal when it runs too long
/// (PLUGIN.md §2.2).
#[test]
fn a_hook_warns_or_stops_a_backup() {
    use apeiron_services::{ChangeKind, HookChange, Hooks, Name, Verdict};
    let dir = tempfile::tempdir().unwrap();
    let mut plugins = Plugins::open(dir.path().join("plugins"))
        .unwrap()
        .with_time_limit(Duration::from_secs(1));
    plugins.add(&build("fixture-hook", "test_hook")).unwrap();
    assert!(plugins.has_hooks("backup"));
    assert!(!plugins.has_hooks("push"));
    let change = |path: &str, content: &[u8]| HookChange {
        path: Name::from(path),
        kind: ChangeKind::Added,
        content: Some(content.to_vec()),
    };
    assert_eq!(
        plugins.check("backup", "main", &[change("a.txt", b"fine")]),
        Verdict::Allow(Vec::new())
    );
    assert_eq!(
        plugins.check("backup", "main", &[change("run.log", b"fine")]),
        Verdict::Allow(vec![(
            "test-hook".to_owned(),
            "backup of main holds run.log".to_owned()
        )])
    );
    assert_eq!(
        plugins.check("backup", "main", &[change(".env", b"KEY=SECRET")]),
        Verdict::Refuse("test-hook".to_owned(), ".env holds a secret".to_owned())
    );
    let start = Instant::now();
    match plugins.check("backup", "backup/loop", &[]) {
        Verdict::Refuse(name, why) => {
            assert_eq!(name, "test-hook");
            assert!(why.contains("longer"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(start.elapsed() < Duration::from_secs(10));
}
