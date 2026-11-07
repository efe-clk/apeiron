//! The C ABI of Apeiron (SDK.md §2): a small fixed set of functions;
//! operations are named by strings and requests, results, progress and
//! errors pass as UTF-8 JSON (ADR-015).
//!
//! Every function returns `0` on success, [`APEIRON_PENDING`] when a job is
//! not finished, or the code of the error (API.md §8.6), whose details
//! [`apeiron_last_error`] returns. A panic never crosses the boundary.

#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![allow(non_camel_case_types)]

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::time::Duration;

use api::json as ops;
use api::{Error, ErrorKind, Repository};

mod stream;

/// The job is not finished yet.
pub const APEIRON_PENDING: i32 = -1;
/// Version of this ABI: `major << 16 | minor`. An added function or field
/// raises the minor; any other change raises the major.
pub const APEIRON_ABI_VERSION: u32 = 2;

/// An open repository. It can be used from several threads at once.
pub struct apeiron_repo(Repository);

/// A running long operation. Use it from one thread at a time.
pub struct apeiron_job(Box<dyn ops::JsonJob>);

/// A file of a snapshot being read. Use it from one thread at a time.
pub struct apeiron_reader(api::FileReader);

/// A file being added to a backup. Use it from one thread at a time.
pub struct apeiron_writer(stream::FileWriter);

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

type Result<T> = std::result::Result<T, Error>;

/// Runs `f`, turning an error or a panic into its code and the last error.
fn guard(f: impl FnOnce() -> Result<i32>) -> i32 {
    let error = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(code)) => return code,
        Ok(Err(e)) => e,
        Err(_) => Error::new(
            ErrorKind::Internal,
            "Apeiron panicked; please report this bug",
        ),
    };
    let code = i32::from(error.code);
    let json = serde_json::to_string(&error).unwrap_or_default();
    LAST_ERROR.with(|last| *last.borrow_mut() = CString::new(json).ok());
    code
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

/// Borrows a NUL-terminated UTF-8 string.
///
/// # Safety
/// `s` is null or points to a NUL-terminated string valid for `'a`.
unsafe fn text<'a>(s: *const c_char, what: &str) -> Result<&'a str> {
    if s.is_null() {
        return Err(invalid(format!("{what} is NULL")));
    }
    // SAFETY: the caller passes a NUL-terminated string that outlives the call.
    unsafe { CStr::from_ptr(s) }
        .to_str()
        .map_err(|_| invalid(format!("{what} is not UTF-8")))
}

/// Clears an out-parameter so it is NULL whenever a call fails.
///
/// # Safety
/// `out` is null or valid for writes.
unsafe fn clear<T>(out: *mut *mut T, what: &str) -> Result<()> {
    if out.is_null() {
        return Err(invalid(format!("{what} is NULL")));
    }
    // SAFETY: checked for null; the caller passes a writable pointer.
    unsafe { *out = ptr::null_mut() };
    Ok(())
}

/// Hands a string to the caller, who frees it with [`apeiron_string_free`].
///
/// # Safety
/// `out` was checked by [`clear`].
unsafe fn give(out: *mut *mut c_char, s: String) -> Result<()> {
    let s = CString::new(s).map_err(|_| invalid("the result holds a NUL byte"))?;
    // SAFETY: see the function's contract.
    unsafe { *out = s.into_raw() };
    Ok(())
}

/// # Safety
/// `out` was checked by [`clear`].
unsafe fn give_repo(out: *mut *mut apeiron_repo, repo: Repository) {
    // SAFETY: see the function's contract.
    unsafe { *out = Box::into_raw(Box::new(apeiron_repo(repo))) };
}

/// # Safety
/// `repo` is null or a live handle.
unsafe fn repo_ref<'a>(repo: *const apeiron_repo) -> Result<&'a Repository> {
    // SAFETY: the caller passes a live handle or null.
    unsafe { repo.as_ref() }
        .map(|r| &r.0)
        .ok_or_else(|| invalid("repo is NULL"))
}

/// # Safety
/// `job` is null or a live handle used by no other thread.
unsafe fn job_mut<'a>(job: *mut apeiron_job) -> Result<&'a mut apeiron_job> {
    // SAFETY: the caller passes a live handle or null.
    unsafe { job.as_mut() }.ok_or_else(|| invalid("job is NULL"))
}

/// The ABI version, `major << 16 | minor`.
#[unsafe(no_mangle)]
pub extern "C" fn apeiron_abi_version() -> u32 {
    APEIRON_ABI_VERSION
}

/// The error of the last failed call on this thread as JSON (code, kind,
/// message, path), or NULL. It stays valid until the next failed call on
/// this thread and must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn apeiron_last_error() -> *const c_char {
    LAST_ERROR.with(|last| last.borrow().as_ref().map_or(ptr::null(), |s| s.as_ptr()))
}

/// Frees a string returned by Apeiron; NULL is ignored.
///
/// # Safety
/// `s` is null or a string returned by Apeiron and not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_string_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: the string was made by `CString::into_raw` in `give`.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Creates a repository in a new or empty folder and opens it.
///
/// # Safety
/// `path` is a NUL-terminated string; `out_repo` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_repo_init(
    path: *const c_char,
    out_repo: *mut *mut apeiron_repo,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_repo, "out_repo")?;
            let repo = Repository::init(text(path, "path")?)?;
            give_repo(out_repo, repo);
        }
        Ok(0)
    })
}

/// Creates an encrypted repository, opens it and returns its recovery key,
/// to be shown to the user once.
///
/// # Safety
/// `path` and `password` are NUL-terminated strings; `out_repo` and
/// `out_recovery_key` are valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_repo_init_encrypted(
    path: *const c_char,
    password: *const c_char,
    out_repo: *mut *mut apeiron_repo,
    out_recovery_key: *mut *mut c_char,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_repo, "out_repo")?;
            clear(out_recovery_key, "out_recovery_key")?;
            let (repo, key) =
                Repository::init_encrypted(text(path, "path")?, text(password, "password")?)?;
            give(out_recovery_key, key)?;
            give_repo(out_repo, repo);
        }
        Ok(0)
    })
}

/// Opens a repository that is not encrypted.
///
/// # Safety
/// `path` is a NUL-terminated string; `out_repo` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_repo_open(
    path: *const c_char,
    out_repo: *mut *mut apeiron_repo,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_repo, "out_repo")?;
            let repo = Repository::open(text(path, "path")?)?;
            give_repo(out_repo, repo);
        }
        Ok(0)
    })
}

/// Opens a repository with a password or the recovery key.
///
/// # Safety
/// `path` and `secret` are NUL-terminated strings; `out_repo` is valid for
/// writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_repo_unlock(
    path: *const c_char,
    secret: *const c_char,
    out_repo: *mut *mut apeiron_repo,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_repo, "out_repo")?;
            let repo = Repository::unlock(text(path, "path")?, text(secret, "secret")?)?;
            give_repo(out_repo, repo);
        }
        Ok(0)
    })
}

/// Closes a repository handle; NULL is ignored.
///
/// # Safety
/// `repo` is null or a live handle no other thread is using.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_repo_free(repo: *mut apeiron_repo) {
    if !repo.is_null() {
        // SAFETY: the handle was made by `Box::into_raw` in `give_repo`.
        drop(unsafe { Box::from_raw(repo) });
    }
}

/// Runs a short operation (SDK.md §2.1). `repo` may be NULL for
/// `plugins`, `plugin_add`, `plugin_remove`, `conflicts`,
/// `resolve_conflict`, `saved_searches`, `save_search` and `forget_search`,
/// which need no repository. A NULL request is `{}`.
///
/// # Safety
/// `repo` is null or a live handle; `operation` and `request_json` (or
/// NULL) are NUL-terminated strings; `out_result_json` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_call(
    repo: *const apeiron_repo,
    operation: *const c_char,
    request_json: *const c_char,
    out_result_json: *mut *mut c_char,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_result_json, "out_result_json")?;
            let request = request(request_json)?;
            let operation = text(operation, "operation")?;
            let result = if repo.is_null() {
                ops::call_global(operation, request)?
            } else {
                ops::call(repo_ref(repo)?, operation, request)?
            };
            give(out_result_json, result.to_string())?;
        }
        Ok(0)
    })
}

/// Starts a long operation (SDK.md §2.1). `repo` is NULL for `clone`, which
/// makes a new repository. A NULL request is `{}`.
///
/// # Safety
/// `repo` is null or a live handle; `operation` and `request_json` (or
/// NULL) are NUL-terminated strings; `out_job` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_start(
    repo: *const apeiron_repo,
    operation: *const c_char,
    request_json: *const c_char,
    out_job: *mut *mut apeiron_job,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_job, "out_job")?;
            let request = request(request_json)?;
            let operation = text(operation, "operation")?;
            let job = if operation == "clone" {
                ops::clone(request)?
            } else {
                ops::start(repo_ref(repo)?, operation, request)?
            };
            *out_job = Box::into_raw(Box::new(apeiron_job(job)));
        }
        Ok(0)
    })
}

/// Reads the progress of a job as JSON.
///
/// # Safety
/// `job` is a live handle; `out_progress_json` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_job_progress(
    job: *mut apeiron_job,
    out_progress_json: *mut *mut c_char,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_progress_json, "out_progress_json")?;
            let progress = job_mut(job)?.0.progress();
            let json = serde_json::to_string(&progress)
                .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
            give(out_progress_json, json)?;
        }
        Ok(0)
    })
}

/// Asks a job to stop; it then finishes with the error `cancelled`.
///
/// # Safety
/// `job` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_job_cancel(job: *mut apeiron_job) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe { job_mut(job)? }.0.cancel();
        Ok(0)
    })
}

/// Waits at most `timeout_ms` milliseconds (negative: without limit) for the
/// result of a job. Returns [`APEIRON_PENDING`] if the time passed first.
/// The result, or the error of the job, is given only once.
///
/// # Safety
/// `job` is a live handle; `out_result_json` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_job_wait(
    job: *mut apeiron_job,
    timeout_ms: i64,
    out_result_json: *mut *mut c_char,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_result_json, "out_result_json")?;
            let timeout = u64::try_from(timeout_ms).ok().map(Duration::from_millis);
            match job_mut(job)?.0.wait(timeout) {
                None => Ok(APEIRON_PENDING),
                Some(result) => {
                    give(out_result_json, result?.to_string())?;
                    Ok(0)
                }
            }
        }
    })
}

/// Releases a job; a running job is cancelled and waited for first. NULL is
/// ignored.
///
/// # Safety
/// `job` is null or a live handle no other thread is using.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_job_free(job: *mut apeiron_job) {
    if !job.is_null() {
        // SAFETY: the handle was made by `Box::into_raw` in `apeiron_start`.
        drop(unsafe { Box::from_raw(job) });
    }
}

/// Parses a request; NULL is `{}`.
///
/// # Safety
/// `json` is null or a NUL-terminated string.
unsafe fn request(json: *const c_char) -> Result<serde_json::Value> {
    if json.is_null() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    // SAFETY: the function's contract.
    let json = unsafe { text(json, "request_json")? };
    serde_json::from_str(json).map_err(|e| invalid(format!("request_json: {e}")))
}

/// A request given as JSON to one of the stream functions.
///
/// # Safety
/// `request_json` is null or a NUL-terminated string.
unsafe fn stream_request<T: serde::de::DeserializeOwned>(request_json: *const c_char) -> Result<T> {
    // SAFETY: the function's contract.
    let request = unsafe { request(request_json) }?;
    serde_json::from_value(request).map_err(|e| invalid(format!("request: {e}")))
}

/// Opens a file of a snapshot for reading (SDK.md §2.3). The request is
/// `{"snapshot", "path"}`. Fails here if the file is not there.
///
/// # Safety
/// `repo` is null or a live handle; `request_json` is a NUL-terminated
/// string; `out_reader` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_reader_open(
    repo: *const apeiron_repo,
    request_json: *const c_char,
    out_reader: *mut *mut apeiron_reader,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_reader, "out_reader")?;
            let request: api::ListRequest = stream_request(request_json)?;
            let reader = repo_ref(repo)?.open_file(&request)?;
            *out_reader = Box::into_raw(Box::new(apeiron_reader(reader)));
        }
        Ok(0)
    })
}

/// Reads up to `capacity` bytes into `buffer` and writes how many to
/// `out_read`; 0 means the end of the file. After a failure, every later
/// read fails too: a file with a hole in it never looks whole.
///
/// # Safety
/// `reader` is null or a live handle used by no other thread; `buffer` is
/// valid for writes of `capacity` bytes; `out_read` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_reader_read(
    reader: *mut apeiron_reader,
    buffer: *mut u8,
    capacity: usize,
    out_read: *mut usize,
) -> i32 {
    guard(|| {
        if out_read.is_null() {
            return Err(invalid("out_read is NULL"));
        }
        // SAFETY: checked for null; the caller passes a writable pointer.
        unsafe { *out_read = 0 };
        if buffer.is_null() || capacity == 0 {
            return Err(invalid("the buffer is NULL or empty"));
        }
        // SAFETY: the caller passes a live handle or null.
        let reader = unsafe { reader.as_mut() }.ok_or_else(|| invalid("reader is NULL"))?;
        // SAFETY: the caller passes `capacity` writable bytes at `buffer`.
        // They are cleared first, so no uninitialized byte is ever read.
        let buffer = unsafe {
            ptr::write_bytes(buffer, 0, capacity);
            std::slice::from_raw_parts_mut(buffer, capacity)
        };
        let read = reader.0.read_some(buffer)?;
        // SAFETY: checked for null above.
        unsafe { *out_read = read };
        Ok(0)
    })
}

/// Releases a reader. NULL is allowed.
///
/// # Safety
/// `reader` is null or a live handle, not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_reader_free(reader: *mut apeiron_reader) {
    if !reader.is_null() {
        // SAFETY: the handle was made by `Box::into_raw` in `apeiron_reader_open`.
        drop(unsafe { Box::from_raw(reader) });
    }
}

/// Starts adding a file to a backup (SDK.md §2.3). The request is
/// `{"ref", "folder", "name", "message", "host"}`. Other writes to the
/// repository wait until the writer is finished or freed, so the same thread
/// must not start one in between.
///
/// # Safety
/// `repo` is null or a live handle; `request_json` is a NUL-terminated
/// string; `out_writer` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_writer_open(
    repo: *const apeiron_repo,
    request_json: *const c_char,
    out_writer: *mut *mut apeiron_writer,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_writer, "out_writer")?;
            let request: api::AddFileRequest = stream_request(request_json)?;
            let writer = stream::FileWriter::open(repo_ref(repo)?, request)?;
            *out_writer = Box::into_raw(Box::new(apeiron_writer(writer)));
        }
        Ok(0)
    })
}

/// Adds `length` bytes to the file.
///
/// # Safety
/// `writer` is null or a live handle used by no other thread; `data` is
/// valid for reads of `length` bytes, or `length` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_writer_write(
    writer: *mut apeiron_writer,
    data: *const u8,
    length: usize,
) -> i32 {
    guard(|| {
        // SAFETY: the caller passes a live handle or null.
        let writer = unsafe { writer.as_mut() }.ok_or_else(|| invalid("writer is NULL"))?;
        if length == 0 {
            writer.0.write(&[])?;
            return Ok(0);
        }
        if data.is_null() {
            return Err(invalid("data is NULL"));
        }
        // SAFETY: the caller passes `length` readable bytes at `data`.
        writer
            .0
            .write(unsafe { std::slice::from_raw_parts(data, length) })?;
        Ok(0)
    })
}

/// Ends the file and stores it as a new snapshot, giving the snapshot, the
/// path and the size as JSON. Only this call stores anything. The handle is
/// still freed afterwards.
///
/// # Safety
/// `writer` is null or a live handle used by no other thread;
/// `out_result_json` is valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_writer_finish(
    writer: *mut apeiron_writer,
    out_result_json: *mut *mut c_char,
) -> i32 {
    guard(|| {
        // SAFETY: the function's contract.
        unsafe {
            clear(out_result_json, "out_result_json")?;
            let writer = writer.as_mut().ok_or_else(|| invalid("writer is NULL"))?;
            let report = writer.0.finish()?;
            let json = serde_json::to_string(&report)
                .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
            give(out_result_json, json)?;
        }
        Ok(0)
    })
}

/// Releases a writer. If it was not finished, nothing is stored. NULL is
/// allowed.
///
/// # Safety
/// `writer` is null or a live handle, not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apeiron_writer_free(writer: *mut apeiron_writer) {
    if !writer.is_null() {
        // SAFETY: the handle was made by `Box::into_raw` in `apeiron_writer_open`.
        drop(unsafe { Box::from_raw(writer) });
    }
}

#[cfg(test)]
mod tests;
