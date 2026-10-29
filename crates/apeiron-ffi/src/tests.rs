//! The C ABI called the way a C program calls it, the shared conformance
//! scenarios (SDK.md §6) and the check that `apeiron.h` is up to date.

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
            (Some(kind), Err(got)) => assert_eq!(got, kind, "{at}"),
            (Some(kind), Ok(v)) => panic!("{at}: expected error {kind}, got {v}"),
            (None, Err(got)) => panic!("{at}: failed with {got}: {}", last_error()),
            (None, Ok(v)) => {
                if let Some(expected) = step.get("expect") {
                    assert!(holds(&v, expected), "{at}\ngot {v}");
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
    for file in files {
        run_scenario(&file);
    }
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
        "include/apeiron.h is out of date; run APEIRON_BLESS=1 cargo test -p apeiron-ffi"
    );
}
