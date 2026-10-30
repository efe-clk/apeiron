//! Operations by name: JSON requests in, JSON results out (SDK.md §2.1).
//! The C ABI and the local server (DESKTOP.md §3) are built on these.

use std::path::PathBuf;
use std::time::Duration;

use crate::error::{Error, ErrorKind, Result};
use crate::{Job, Progress, Repository};
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

/// A job whose result is turned into JSON.
pub trait JsonJob: Send {
    fn progress(&self) -> Progress;
    fn cancel(&self);
    /// `None` while the job runs; `timeout` `None` waits without limit.
    fn wait(&mut self, timeout: Option<Duration>) -> Option<Result<Value>>;
}

impl<T: Serialize + Send + 'static> JsonJob for Job<T> {
    fn progress(&self) -> Progress {
        Job::progress(self)
    }

    fn cancel(&self) {
        Job::cancel(self);
    }

    fn wait(&mut self, timeout: Option<Duration>) -> Option<Result<Value>> {
        let result = match timeout {
            Some(t) => self.wait_timeout(t)?,
            None => loop {
                if let Some(r) = self.wait_timeout(Duration::from_secs(3600)) {
                    break r;
                }
            },
        };
        Some(result.and_then(|v| json(&v)))
    }
}

fn json(value: &impl Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))
}

fn parse<T: DeserializeOwned>(request: Value) -> Result<T> {
    serde_json::from_value(request).map_err(|e| invalid(format!("request: {e}")))
}

fn boxed<T: Serialize + Send + 'static>(job: Job<T>) -> Box<dyn JsonJob> {
    Box::new(job)
}

#[derive(Deserialize)]
struct Spec {
    spec: String,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct Password {
    password: String,
}

#[derive(Deserialize)]
struct KeyId {
    id: String,
}

#[derive(Deserialize)]
struct PluginDir {
    #[serde(default)]
    dir: Option<PathBuf>,
}

#[derive(Deserialize)]
struct PluginFile {
    file: PathBuf,
    #[serde(default)]
    dir: Option<PathBuf>,
}

#[derive(Deserialize)]
struct PluginName {
    name: String,
    #[serde(default)]
    dir: Option<PathBuf>,
}

#[derive(Deserialize)]
struct NewRemote {
    name: String,
    url: String,
}

#[derive(Deserialize)]
struct NewRetention {
    refs: String,
    days: u32,
}

fn unknown(operation: &str) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        format!("unknown operation {operation:?}"),
    )
}

/// Runs a short operation.
pub fn call(repo: &Repository, operation: &str, request: Value) -> Result<Value> {
    match operation {
        "info" => json(&repo.info()),
        "resolve" => json(&repo.resolve(&parse::<Spec>(request)?.spec)?),
        "log" => json(&repo.log(&parse(request)?)?),
        "list" => json(&repo.list(&parse(request)?)?),
        "diff" => json(&repo.diff(&parse(request)?)?),
        "diff_text" => json(&repo.diff_text(&parse(request)?)?),
        "refs" => json(&repo.refs()?),
        "reflog" => json(&repo.reflog(&parse::<Named>(request)?.name)?),
        "create_ref" => json(&repo.create_ref(&parse(request)?)?),
        "delete_ref" => json(&repo.delete_ref(&parse(request)?)?),
        "merge" => json(&repo.merge(&parse(request)?)?),
        "repair" => json(&repo.repair()?),
        "keys" => json(&repo.keys()?),
        "add_password" => json(&repo.add_password(&parse::<Password>(request)?.password)?),
        "add_recovery_key" => json(&repo.add_recovery_key()?),
        "remove_key" => json(&repo.remove_key(&parse::<KeyId>(request)?.id)?),
        "remotes" => json(&repo.remotes()?),
        "add_remote" => {
            let r: NewRemote = parse(request)?;
            json(&repo.add_remote(&r.name, &r.url)?)
        }
        "remove_remote" => json(&repo.remove_remote(&parse::<Named>(request)?.name)?),
        "retention" => json(&repo.retention()?),
        "add_retention" => {
            let r: NewRetention = parse(request)?;
            json(&repo.add_retention(&r.refs, r.days)?)
        }
        _ => Err(unknown(operation)),
    }
}

/// Runs an operation that needs no repository: the plugins of the user
/// (PLUGIN.md §1).
pub fn call_global(operation: &str, request: Value) -> Result<Value> {
    match operation {
        "plugins" => json(&crate::plugins(
            parse::<PluginDir>(request)?.dir.as_deref(),
        )?),
        "plugin_add" => {
            let r: PluginFile = parse(request)?;
            json(&crate::add_plugin(&r.file, r.dir.as_deref())?)
        }
        "plugin_remove" => {
            let r: PluginName = parse(request)?;
            json(&crate::remove_plugin(&r.name, r.dir.as_deref())?)
        }
        _ => Err(invalid(format!(
            "operation {operation:?} needs a repository; repo is NULL"
        ))),
    }
}

/// Starts a long operation on a repository.
pub fn start(repo: &Repository, operation: &str, request: Value) -> Result<Box<dyn JsonJob>> {
    Ok(match operation {
        "backup" => boxed(repo.backup(parse(request)?)?),
        "restore" => boxed(repo.restore(parse(request)?)?),
        "export" => boxed(repo.export(parse(request)?)?),
        "verify" => boxed(repo.verify()?),
        "gc" => boxed(repo.gc(parse(request)?)?),
        "push" => boxed(repo.push(parse(request)?)?),
        "pull" => boxed(repo.pull(parse(request)?)?),
        "push_all" => boxed(repo.push_all(parse(request)?)?),
        "pull_all" => boxed(repo.pull_all(parse(request)?)?),
        "report" => boxed(repo.report(parse(request)?)?),
        "restore_test" => boxed(repo.restore_test(parse(request)?)?),
        "search" => boxed(repo.search(parse(request)?)?),
        "sync" => boxed(repo.sync(parse(request)?)?),
        "git_import" => boxed(repo.git_import(parse(request)?)?),
        "git_export" => boxed(repo.git_export(parse(request)?)?),
        _ => return Err(unknown(operation)),
    })
}

/// Starts `clone`, which makes a new repository from a remote.
pub fn clone(request: Value) -> Result<Box<dyn JsonJob>> {
    Ok(boxed(Repository::clone_from(parse(request)?)?))
}
