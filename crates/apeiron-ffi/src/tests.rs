//! The C ABI called the way a C program calls it, the shared conformance
//! scenarios (SDK.md §6) and the check that `apeiron.h` is up to date.

use std::collections::BTreeSet;
use std::ffi::{CStr, CString, c_char};
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;

use serde_json::Value;

use super::*;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn last_error() -> Value {
    let e = apeiron_last_error();
    assert!(!e.is_null());
    serde_json::from_str(unsafe { CStr::from_ptr(e) }.to_str().unwrap()).unwrap()
}

/// Takes a string returned by the library.
fn take(s: *mut c_char) -> String {
    assert!(!s.is_null());
    let out = unsafe { CStr::from_ptr(s) }.to_str().unwrap().to_owned();
    unsafe { apeiron_string_free(s) };
    out
}

/// An open handle and the folder it lives in.
struct Repo(*mut apeiron_repo);

impl Drop for Repo {
    fn drop(&mut self) {
        unsafe { apeiron_repo_free(self.0) };
    }
}

impl Repo {
    /// Runs a short operation: the result, or the error kind.
    fn call(&self, op: &str, request: &Value) -> std::result::Result<Value, String> {
        let mut out = ptr::null_mut();
        let request = c(&request.to_string());
        let code = unsafe { apeiron_call(self.0, c(op).as_ptr(), request.as_ptr(), &mut out) };
        finish(code, out)
    }

    /// Starts a long operation and waits for it: the result, or the error kind.
    fn start(&self, op: &str, request: &Value) -> std::result::Result<Value, String> {
        start(self.0, op, request)
    }
}

fn start(
    repo: *const apeiron_repo,
    op: &str,
    request: &Value,
) -> std::result::Result<Value, String> {
    let mut job = ptr::null_mut();
    let request = c(&request.to_string());
    let code = unsafe { apeiron_start(repo, c(op).as_ptr(), request.as_ptr(), &mut job) };
    if code != 0 {
        assert!(job.is_null());
        return Err(error_kind(code));
    }
    let mut out = ptr::null_mut();
    let code = unsafe { apeiron_job_wait(job, -1, &mut out) };
    unsafe { apeiron_job_free(job) };
    finish(code, out)
}

fn finish(code: i32, out: *mut c_char) -> std::result::Result<Value, String> {
    if code == 0 {
        Ok(serde_json::from_str(&take(out)).unwrap())
    } else {
        assert!(out.is_null());
        Err(error_kind(code))
    }
}

fn error_kind(code: i32) -> String {
    let e = last_error();
    assert_eq!(e["code"], code);
    e["kind"].as_str().unwrap().to_owned()
}

fn open(path: &Path, password: Option<&str>) -> std::result::Result<Repo, String> {
    let mut repo = ptr::null_mut();
    let p = c(path.to_str().unwrap());
    let code = match password {
        None => unsafe { apeiron_repo_open(p.as_ptr(), &mut repo) },
        Some(s) => unsafe { apeiron_repo_unlock(p.as_ptr(), c(s).as_ptr(), &mut repo) },
    };
    if code == 0 {
        Ok(Repo(repo))
    } else {
        assert!(repo.is_null());
        Err(error_kind(code))
    }
}

fn init(path: &Path, password: Option<&str>) -> Repo {
    let mut repo = ptr::null_mut();
    let p = c(path.to_str().unwrap());
    let code = match password {
        None => unsafe { apeiron_repo_init(p.as_ptr(), &mut repo) },
        Some(s) => {
            let mut key = ptr::null_mut();
            let code = unsafe {
                apeiron_repo_init_encrypted(p.as_ptr(), c(s).as_ptr(), &mut repo, &mut key)
            };
            assert!(!take(key).is_empty());
            code
        }
    };
    assert_eq!(code, 0);
    Repo(repo)
}

/// Whether `actual` holds everything in `expected`: objects by key, arrays
/// element by element with the same length, other values by equality.
fn holds(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            e.iter().all(|(k, v)| a.get(k).is_some_and(|x| holds(x, v)))
        }
        (Value::Array(a), Value::Array(e)) => {
            a.len() == e.len() && a.iter().zip(e).all(|(x, v)| holds(x, v))
        }
        _ => actual == expected,
    }
}

/// Replaces `$TMP` in every string with the scenario's folder.
fn expand(value: &Value, tmp: &str) -> Value {
    match value {
        Value::String(s) => Value::String(s.replace("$TMP", tmp)),
        Value::Array(a) => Value::Array(a.iter().map(|v| expand(v, tmp)).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), expand(v, tmp))).collect())
        }
        other => other.clone(),
    }
}

fn run_scenario(file: &Path) {
    let scenario: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
    let schema = api::json::schema();
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path().to_str().unwrap().to_owned();
    let mut repo: Option<Repo> = None;
    for (i, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
        let step = expand(step, &tmp);
        let at = format!("{} step {}: {step}", file.display(), i + 1);
        let request = step
            .get("request")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));
        let password = step.get("password").and_then(Value::as_str);
        let result = if let Some(rel) = step.get("write").and_then(Value::as_str) {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, step["text"].as_str().unwrap()).unwrap();
            continue;
        } else if let Some(rel) = step.get("remove").and_then(Value::as_str) {
            fs::remove_file(dir.path().join(rel)).unwrap();
            continue;
        } else if let Some(rel) = step.get("read").and_then(Value::as_str) {
            let text = fs::read_to_string(dir.path().join(rel)).unwrap();
            assert_eq!(text, step["text"].as_str().unwrap(), "{at}");
            continue;
        } else if let Some(rel) = step.get("init").and_then(Value::as_str) {
            repo = Some(init(&dir.path().join(rel), password));
            continue;
        } else if let Some(rel) = step.get("open").and_then(Value::as_str) {
            match open(&dir.path().join(rel), password) {
                Ok(r) => {
                    repo = Some(r);
                    Ok(Value::Null)
                }
                Err(e) => Err(e),
            }
        } else if let Some(op) = step.get("call").and_then(Value::as_str) {
            repo.as_ref().unwrap().call(op, &request)
        } else if let Some(op) = step.get("start").and_then(Value::as_str) {
            repo.as_ref().unwrap().start(op, &request)
        } else {
            panic!("{at}: unknown step");
        };
        match (step.get("error").and_then(Value::as_str), result) {
            (Some(kind), Err(got)) => {
                assert_eq!(got, kind, "{at}");
                check_against(&schema, &schema["error"], &last_error(), &at);
            }
            (Some(kind), Ok(v)) => panic!("{at}: expected error {kind}, got {v}"),
            (None, Err(got)) => panic!("{at}: failed with {got}: {}", last_error()),
            (None, Ok(v)) => {
                if let Some(expected) = step.get("expect") {
                    assert!(holds(&v, expected), "{at}\ngot {v}");
                }
                let op = step
                    .get("call")
                    .or(step.get("start"))
                    .and_then(Value::as_str);
                if let Some(op) = op {
                    let entry = &schema["operations"][op];
                    assert!(entry.is_object(), "{at}: not in the schema");
                    check_against(&schema, &entry["request"], &request, &at);
                    check_against(&schema, &entry["result"], &v, &at);
                }
            }
        }
    }
}

fn sdk_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk")
}

#[test]
fn conformance_scenarios() {
    let mut files: Vec<PathBuf> = fs::read_dir(sdk_dir().join("conformance"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    assert!(files.len() >= 4);
    let mut used = BTreeSet::new();
    for file in &files {
        run_scenario(file);
        let scenario: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        for step in scenario["steps"].as_array().unwrap() {
            let op = step.get("call").or(step.get("start"));
            if let (Some(op), None) = (op.and_then(Value::as_str), step.get("error")) {
                used.insert(op.to_owned());
            }
        }
    }
    // These need what a scenario cannot make: an id it cannot know, a plugin
    // file, or no repository (`clone_needs_no_repository_handle`).
    let elsewhere = [
        "clone",
        "delete_ref",
        "lift_hold",
        "plugin_add",
        "plugin_remove",
        "remove_key",
    ];
    let schema = api::json::schema();
    let missing: Vec<&String> = schema["operations"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|op| !used.contains(*op) && !elsewhere.contains(&op.as_str()))
        .collect();
    assert!(missing.is_empty(), "no scenario runs {missing:?}");
}

#[test]
fn null_pointers_and_bad_strings_are_errors() {
    let mut repo = ptr::null_mut();
    assert_eq!(unsafe { apeiron_repo_open(ptr::null(), &mut repo) }, 4);
    assert_eq!(last_error()["message"], "path is NULL");
    assert_eq!(
        unsafe { apeiron_repo_open(c("x").as_ptr(), ptr::null_mut()) },
        4
    );
    let bad = [0xffu8, 0];
    assert_eq!(
        unsafe { apeiron_repo_open(bad.as_ptr().cast(), &mut repo) },
        4
    );
    assert!(repo.is_null());
    let mut out = ptr::null_mut();
    assert_eq!(
        unsafe { apeiron_call(ptr::null(), c("info").as_ptr(), ptr::null(), &mut out) },
        4
    );
    let dir = tempfile::tempdir().unwrap();
    let request = c(&serde_json::json!({"dir": dir.path().join("plugins")}).to_string());
    assert_eq!(
        unsafe {
            apeiron_call(
                ptr::null(),
                c("plugins").as_ptr(),
                request.as_ptr(),
                &mut out,
            )
        },
        0
    );
    assert_eq!(take(out), "[]");
    let request = c(&serde_json::json!({"name": "x", "dir": dir.path()}).to_string());
    assert_eq!(
        unsafe {
            apeiron_call(
                ptr::null(),
                c("plugin_remove").as_ptr(),
                request.as_ptr(),
                &mut out,
            )
        },
        2
    );
    assert_eq!(unsafe { apeiron_job_cancel(ptr::null_mut()) }, 4);
    // Freeing NULL is allowed.
    unsafe {
        apeiron_repo_free(ptr::null_mut());
        apeiron_job_free(ptr::null_mut());
        apeiron_string_free(ptr::null_mut());
    }
    assert_eq!(apeiron_abi_version(), APEIRON_ABI_VERSION);
}

#[test]
fn a_job_reports_progress_can_be_polled_and_is_given_once() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/a.txt"), b"a").unwrap();
    let repo = init(&dir.path().join("repo"), None);
    let request = c(&serde_json::json!({
        "source": dir.path().join("src"),
        "ref": "main",
    })
    .to_string());
    let mut job = ptr::null_mut();
    assert_eq!(
        unsafe { apeiron_start(repo.0, c("backup").as_ptr(), request.as_ptr(), &mut job) },
        0
    );
    let mut out = ptr::null_mut();
    loop {
        let mut progress = ptr::null_mut();
        assert_eq!(unsafe { apeiron_job_progress(job, &mut progress) }, 0);
        let progress: Value = serde_json::from_str(&take(progress)).unwrap();
        assert!(progress["phase"].is_string());
        match unsafe { apeiron_job_wait(job, 0, &mut out) } {
            APEIRON_PENDING => assert!(out.is_null()),
            0 => break,
            code => panic!("failed: {code}"),
        }
    }
    let report: Value = serde_json::from_str(&take(out)).unwrap();
    assert_eq!(report["files"], 1);
    // The result is given only once.
    assert_eq!(unsafe { apeiron_job_wait(job, -1, &mut out) }, 11);
    unsafe { apeiron_job_free(job) };
}

#[test]
fn clone_needs_no_repository_handle() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/a.txt"), b"a").unwrap();
    let repo = init(&dir.path().join("repo"), None);
    repo.start(
        "backup",
        &serde_json::json!({"source": dir.path().join("src"), "ref": "main"}),
    )
    .unwrap();
    let report = start(
        ptr::null(),
        "clone",
        &serde_json::json!({"url": dir.path().join("repo"), "path": dir.path().join("copy")}),
    )
    .unwrap();
    assert_eq!(report["refs"][0]["name"], "main");
    let copy = open(&dir.path().join("copy"), None).unwrap();
    assert_eq!(
        copy.call("remotes", &Value::Null).unwrap()[0]["name"],
        "origin"
    );
}

fn schema_file() -> PathBuf {
    sdk_dir().join("schema/apeiron.schema.json")
}

/// `sdk/schema/apeiron.schema.json` matches the Rust types. Run with
/// `APEIRON_BLESS=1` to write it anew after a change.
#[test]
fn the_schema_is_up_to_date() {
    let path = schema_file();
    let generated = serde_json::to_string_pretty(&api::json::schema()).unwrap() + "\n";
    if std::env::var_os("APEIRON_BLESS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &generated).unwrap();
    }
    let current = fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        current == generated,
        "the schema is out of date; run APEIRON_BLESS=1 cargo test -p apeiron-ffi-rs"
    );
}

/// Every operation the dispatch knows has an entry, and only those.
#[test]
fn the_schema_names_every_operation() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../apeiron/src/json.rs"))
            .unwrap();
    let mut known: Vec<String> = source
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix('"')?;
            let (name, tail) = rest.split_once('"')?;
            tail.trim_start().starts_with("=>").then(|| name.to_owned())
        })
        .chain(["clone".to_owned()])
        .collect();
    known.sort();
    known.dedup();
    let schema = api::json::schema();
    let mut named: Vec<String> = schema["operations"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    named.sort();
    assert_eq!(named, known);
}

/// Checks a value against one reference of the schema.
fn check_against(schema: &Value, reference: &Value, value: &Value, at: &str) {
    let mut whole = reference.clone();
    whole["$defs"] = schema["$defs"].clone();
    whole["$schema"] = schema["$schema"].clone();
    let validator = jsonschema::validator_for(&whole).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{at}: {errors:?}\nvalue: {value}");
}

/// `apeiron.h` matches the functions above. Run with `APEIRON_BLESS=1` to
/// write it anew after a change.
#[test]
fn the_header_is_up_to_date() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let config = cbindgen::Config::from_file(Path::new(crate_dir).join("cbindgen.toml")).unwrap();
    let mut generated = Vec::new();
    cbindgen::generate_with_config(crate_dir, config)
        .unwrap()
        .write(&mut generated);
    let generated = String::from_utf8(generated).unwrap();
    let path = Path::new(crate_dir).join("include/apeiron.h");
    if std::env::var_os("APEIRON_BLESS").is_some() {
        fs::write(&path, &generated).unwrap();
    }
    let current = fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        current == generated,
        "include/apeiron.h is out of date; run APEIRON_BLESS=1 cargo test -p apeiron-ffi-rs"
    );
}

/// Bytes that do not repeat, so a chunk out of place shows.
fn noise(len: usize) -> Vec<u8> {
    let mut x = 0x9E37_79B9_7F4A_7C15_u64;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn backup_of(dir: &Path, repo: &Repo, data: &[u8]) {
    let source = dir.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("big.bin"), data).unwrap();
    fs::write(source.join("empty"), b"").unwrap();
    repo.start(
        "backup",
        &serde_json::json!({"source": source, "ref": "main"}),
    )
    .unwrap();
}

struct Reader(*mut apeiron_reader);

impl Drop for Reader {
    fn drop(&mut self) {
        unsafe { apeiron_reader_free(self.0) };
    }
}

impl Reader {
    fn open(repo: &Repo, request: &Value) -> std::result::Result<Reader, String> {
        let mut out = ptr::null_mut();
        let request = c(&request.to_string());
        match unsafe { apeiron_reader_open(repo.0, request.as_ptr(), &mut out) } {
            0 => Ok(Reader(out)),
            code => {
                assert!(out.is_null());
                Err(error_kind(code))
            }
        }
    }

    fn read(&self, buffer: &mut [u8]) -> std::result::Result<usize, String> {
        let mut read = 99;
        let code =
            unsafe { apeiron_reader_read(self.0, buffer.as_mut_ptr(), buffer.len(), &mut read) };
        if code == 0 {
            Ok(read)
        } else {
            assert_eq!(read, 0);
            Err(error_kind(code))
        }
    }

    fn all(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            match self.read(&mut buffer).unwrap() {
                0 => return out,
                n => out.extend_from_slice(&buffer[..n]),
            }
        }
    }
}

struct Writer(*mut apeiron_writer);

impl Drop for Writer {
    fn drop(&mut self) {
        unsafe { apeiron_writer_free(self.0) };
    }
}

impl Writer {
    fn open(repo: &Repo, request: &Value) -> std::result::Result<Writer, String> {
        let mut out = ptr::null_mut();
        let request = c(&request.to_string());
        match unsafe { apeiron_writer_open(repo.0, request.as_ptr(), &mut out) } {
            0 => Ok(Writer(out)),
            code => {
                assert!(out.is_null());
                Err(error_kind(code))
            }
        }
    }

    fn write(&self, data: &[u8]) -> std::result::Result<(), String> {
        match unsafe { apeiron_writer_write(self.0, data.as_ptr(), data.len()) } {
            0 => Ok(()),
            code => Err(error_kind(code)),
        }
    }

    fn finish(&self) -> std::result::Result<Value, String> {
        let mut out = ptr::null_mut();
        let code = unsafe { apeiron_writer_finish(self.0, &mut out) };
        finish(code, out)
    }
}

fn files_under(dir: &Path) -> BTreeSet<PathBuf> {
    let mut found = BTreeSet::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.insert(path);
        }
    }
    found
}

#[test]
fn a_file_is_read_in_pieces() {
    let dir = tempfile::tempdir().unwrap();
    let repo = init(&dir.path().join("repo"), None);
    let data = noise(5 << 20);
    backup_of(dir.path(), &repo, &data);

    let reader = Reader::open(
        &repo,
        &serde_json::json!({"snapshot": "main", "path": "big.bin"}),
    )
    .unwrap();
    assert!(reader.all() == data, "the bytes read differ");
    // The end stays the end.
    assert_eq!(reader.read(&mut [0; 16]), Ok(0));
    let empty = Reader::open(
        &repo,
        &serde_json::json!({"snapshot": "main", "path": "empty"}),
    )
    .unwrap();
    assert!(empty.all().is_empty());

    // What cannot be read fails when opening, not later.
    let open = |path: &str, snapshot: &str| {
        Reader::open(
            &repo,
            &serde_json::json!({"snapshot": snapshot, "path": path}),
        )
        .map(|_| ())
    };
    assert_eq!(open("missing", "main"), Err("not_found".to_owned()));
    assert_eq!(open("", "main"), Err("invalid_input".to_owned()));
    assert_eq!(open("big.bin", "no-such"), Err("not_found".to_owned()));
    // A buffer that cannot take anything is an error, not the end of the file.
    let mut read = 0;
    assert_eq!(
        unsafe { apeiron_reader_read(reader.0, [0_u8; 1].as_mut_ptr(), 0, &mut read) },
        4
    );
    let schema: Value = serde_json::from_str(&fs::read_to_string(schema_file()).unwrap()).unwrap();
    check_against(
        &schema,
        &schema["streams"]["reader"]["request"],
        &serde_json::json!({"snapshot": "main", "path": "big.bin"}),
        "reader request",
    );
}

/// A reader that waits holds nothing: the same thread can read and write the
/// repository meanwhile. If it held a lock, this test would never end.
#[test]
fn a_waiting_reader_leaves_the_repository_free() {
    let dir = tempfile::tempdir().unwrap();
    let repo = init(&dir.path().join("repo"), None);
    let data = noise(3 << 20);
    backup_of(dir.path(), &repo, &data);
    let reader = Reader::open(
        &repo,
        &serde_json::json!({"snapshot": "main", "path": "big.bin"}),
    )
    .unwrap();
    let mut start = vec![0_u8; 1000];
    let first = reader.read(&mut start).unwrap();

    repo.call("list", &serde_json::json!({"snapshot": "main"}))
        .unwrap();
    repo.start(
        "backup",
        &serde_json::json!({"source": dir.path().join("source"), "ref": "other"}),
    )
    .unwrap();

    let mut got = start[..first].to_vec();
    got.extend(reader.all());
    assert!(got == data, "the bytes read differ");
}

/// Damage found while reading fails every later read too, so a program that
/// goes on cannot get a file with a hole in it that looks whole.
#[test]
fn damage_stops_a_reader_for_good() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repo");
    let repo = init(&path, None);
    let data = noise(5 << 20);
    backup_of(dir.path(), &repo, &data);
    let reader = Reader::open(
        &repo,
        &serde_json::json!({"snapshot": "main", "path": "big.bin"}),
    )
    .unwrap();
    let mut damaged = 0;
    for file in files_under(&path) {
        if file.extension().is_some_and(|e| e == "pack") {
            let mut bytes = fs::read(&file).unwrap();
            // One chunk only: the ones after it are whole, and a reader that
            // went on past the damage would give them, with a hole before.
            for byte in bytes.iter_mut().skip(64).take(256) {
                *byte ^= 0xFF;
            }
            let mut permissions = fs::metadata(&file).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(&file, permissions).unwrap();
            fs::write(&file, bytes).unwrap();
            damaged += 1;
        }
    }
    assert!(damaged > 0, "no pack file was found");

    let mut buffer = [0_u8; 4096];
    let mut given = 0;
    let first = loop {
        match reader.read(&mut buffer) {
            Ok(0) => panic!("the damage was not found"),
            Ok(n) => given += n,
            Err(e) => break e,
        }
    };
    assert_eq!(first, "corrupt");
    assert!(given < data.len() / 2, "the damage came late: {given}");
    for _ in 0..10 {
        assert_eq!(reader.read(&mut buffer), Err(first.clone()));
    }
}

#[test]
fn a_file_is_written_in_pieces_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repo");
    // Encrypted too: the bytes pass through the same writer.
    let repo = init(&path, Some("correct horse"));
    let data = noise(3 << 20);
    let writer = Writer::open(
        &repo,
        &serde_json::json!({"ref": "inbox", "folder": "in/box", "name": "a.bin", "message": "Stream"}),
    )
    .unwrap();
    // A write larger than the queue's pieces, then small ones.
    writer.write(&data[..(2 << 20) + 5]).unwrap();
    for part in data[(2 << 20) + 5..].chunks(4099) {
        writer.write(part).unwrap();
    }
    writer.write(&[]).unwrap();
    let report = writer.finish().unwrap();
    assert_eq!(report["path"], "in/box/a.bin");
    assert_eq!(report["size"], data.len());
    let schema: Value = serde_json::from_str(&fs::read_to_string(schema_file()).unwrap()).unwrap();
    check_against(
        &schema,
        &schema["streams"]["writer"]["result"],
        &report,
        "writer result",
    );
    // Finished is finished.
    assert_eq!(writer.write(b"x"), Err("invalid_input".to_owned()));
    assert_eq!(writer.finish(), Err("invalid_input".to_owned()));
    drop(writer);

    let reader = Reader::open(
        &repo,
        &serde_json::json!({"snapshot": "inbox", "path": "in/box/a.bin"}),
    )
    .unwrap();
    assert!(reader.all() == data, "the bytes read back differ");
    repo.start("verify", &serde_json::json!({})).unwrap();

    // Taken names are not replaced.
    let writer = Writer::open(
        &repo,
        &serde_json::json!({"ref": "inbox", "folder": "in/box", "name": "a.bin"}),
    )
    .unwrap();
    assert_eq!(writer.finish().unwrap()["path"], "in/box/a (2).bin");
}

/// Stopping before the end stores nothing and leaves nothing behind, and the
/// repository goes on working.
#[test]
fn a_writer_not_finished_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repo");
    let repo = init(&path, None);
    backup_of(dir.path(), &repo, &noise(1 << 20));
    let log = |name: &str| repo.call("log", &serde_json::json!({"start": name}));
    let before = log("main").unwrap();
    let files = files_under(&path);

    for target in ["main", "new-ref"] {
        let writer =
            Writer::open(&repo, &serde_json::json!({"ref": target, "name": "a.bin"})).unwrap();
        writer.write(&noise(3 << 20)).unwrap();
        drop(writer);
    }
    // Also with nothing written at all.
    drop(Writer::open(&repo, &serde_json::json!({"ref": "main", "name": "b"})).unwrap());

    assert_eq!(log("main").unwrap(), before);
    assert_eq!(log("new-ref"), Err("not_found".to_owned()));
    assert_eq!(files_under(&path), files, "files were left behind");
    let report = repo.start("verify", &serde_json::json!({})).unwrap();
    assert!(report["objects_checked"].as_u64().unwrap() > 0, "{report}");
    assert_eq!(report["damaged_files"], serde_json::json!([]), "{report}");
    assert_eq!(report["damaged_objects"], serde_json::json!([]), "{report}");

    let writer = Writer::open(&repo, &serde_json::json!({"ref": "main", "name": "a.bin"})).unwrap();
    writer.write(b"whole").unwrap();
    assert_eq!(writer.finish().unwrap()["path"], "a.bin");
}

/// A bad request fails when opening; a failure found only at the end fails
/// `finish`, and every call after it, so nothing looks stored.
#[test]
fn a_writer_reports_what_went_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let repo = init(&dir.path().join("repo"), None);
    backup_of(dir.path(), &repo, b"data");
    let open = |request: Value| Writer::open(&repo, &request).map(|_| ());
    for name in ["", "..", "a/b"] {
        assert_eq!(
            open(serde_json::json!({"ref": "main", "name": name})),
            Err("invalid_input".to_owned()),
            "{name}"
        );
    }
    assert_eq!(
        open(serde_json::json!({"ref": "bad ref", "name": "a"})),
        Err("invalid_input".to_owned())
    );

    let before = repo
        .call("log", &serde_json::json!({"start": "main"}))
        .unwrap();
    let writer = Writer::open(
        &repo,
        &serde_json::json!({"ref": "main", "folder": "big.bin", "name": "a"}),
    )
    .unwrap();
    writer.write(b"bytes").unwrap();
    assert_eq!(writer.finish(), Err("invalid_input".to_owned()));
    assert_eq!(writer.finish(), Err("invalid_input".to_owned()));
    assert_eq!(writer.write(b"more"), Err("invalid_input".to_owned()));
    drop(writer);
    assert_eq!(
        repo.call("log", &serde_json::json!({"start": "main"}))
            .unwrap(),
        before
    );

    // NULL handles are errors; freeing NULL is allowed.
    let mut out = ptr::null_mut();
    assert_eq!(
        unsafe { apeiron_writer_finish(ptr::null_mut(), &mut out) },
        4
    );
    assert_eq!(
        unsafe { apeiron_writer_write(ptr::null_mut(), ptr::null(), 0) },
        4
    );
    let mut read = 0;
    assert_eq!(
        unsafe { apeiron_reader_read(ptr::null_mut(), [0_u8; 1].as_mut_ptr(), 1, &mut read) },
        4
    );
    unsafe {
        apeiron_reader_free(ptr::null_mut());
        apeiron_writer_free(ptr::null_mut());
    }
}
