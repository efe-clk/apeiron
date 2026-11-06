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
    /// Whether the job has ended; its result stays to be collected.
    fn is_finished(&mut self) -> bool;
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

    fn is_finished(&mut self) -> bool {
        Job::is_finished(self)
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

#[derive(Deserialize, schemars::JsonSchema)]
struct Spec {
    spec: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Named {
    name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Password {
    password: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct KeyId {
    id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Folder {
    folder: PathBuf,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Resolve {
    folder: PathBuf,
    copy: String,
    keep: crate::Keep,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SearchesFile {
    #[serde(default)]
    file: Option<PathBuf>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SaveSearch {
    #[serde(flatten)]
    saved: crate::SavedSearch,
    #[serde(default)]
    file: Option<PathBuf>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SearchName {
    name: String,
    #[serde(default)]
    file: Option<PathBuf>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PluginDir {
    #[serde(default)]
    dir: Option<PathBuf>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PluginFile {
    file: PathBuf,
    #[serde(default)]
    dir: Option<PathBuf>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PluginName {
    name: String,
    #[serde(default)]
    dir: Option<PathBuf>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct NewRemote {
    name: String,
    url: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct NewRetention {
    refs: String,
    days: u32,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct NewHold {
    refs: String,
    reason: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct LiftHold {
    id: String,
    reason: String,
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
        "duplicates" => json(&repo.duplicates(&parse(request)?)?),
        "refs" => json(&repo.refs()?),
        "reflog" => json(&repo.reflog(&parse::<Named>(request)?.name)?),
        "create_ref" => json(&repo.create_ref(&parse(request)?)?),
        "delete_ref" => json(&repo.delete_ref(&parse(request)?)?),
        "merge" => json(&repo.merge(&parse(request)?)?),
        "cherry_pick" => json(&repo.cherry_pick(&parse(request)?)?),
        "rebase" => json(&repo.rebase(&parse(request)?)?),
        "prune" => json(&repo.prune(&parse(request)?)?),
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
        "holds" => json(&repo.legal_holds()?),
        "add_hold" => {
            let r: NewHold = parse(request)?;
            json(&repo.add_legal_hold(&r.refs, &r.reason)?)
        }
        "lift_hold" => {
            let r: LiftHold = parse(request)?;
            json(&repo.lift_legal_hold(&r.id, &r.reason)?)
        }
        "retention" => json(&repo.retention()?),
        "add_retention" => {
            let r: NewRetention = parse(request)?;
            json(&repo.add_retention(&r.refs, r.days)?)
        }
        "conflicts" | "resolve_conflict" | "plugins" | "plugin_add" | "plugin_remove"
        | "saved_searches" | "save_search" | "forget_search" => call_global(operation, request),
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
        "saved_searches" => json(&crate::saved_searches(
            parse::<SearchesFile>(request)?.file.as_deref(),
        )?),
        "save_search" => {
            let r: SaveSearch = parse(request)?;
            json(&crate::save_search(&r.saved, r.file.as_deref())?)
        }
        "forget_search" => {
            let r: SearchName = parse(request)?;
            json(&crate::forget_search(&r.name, r.file.as_deref())?)
        }
        "conflicts" => json(&crate::conflicts(&parse::<Folder>(request)?.folder)?),
        "resolve_conflict" => {
            let r: Resolve = parse(request)?;
            json(&crate::resolve_conflict(&r.folder, &r.copy, r.keep)?)
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
        "import_archive" => boxed(repo.import_archive(parse(request)?)?),
        "verify" => boxed(repo.verify()?),
        "gc" => boxed(repo.gc(parse(request)?)?),
        "gc_remote" => boxed(repo.gc_remote(parse(request)?)?),
        "push" => boxed(repo.push(parse(request)?)?),
        "pull" => boxed(repo.pull(parse(request)?)?),
        "push_all" => boxed(repo.push_all(parse(request)?)?),
        "pull_all" => boxed(repo.pull_all(parse(request)?)?),
        "report" => boxed(repo.report(parse(request)?)?),
        "repair_from" => boxed(repo.repair_from(parse(request)?)?),
        "restore_test" => boxed(repo.restore_test(parse(request)?)?),
        "search" => boxed(repo.search(parse(request)?)?),
        "sync" => boxed(repo.sync(parse(request)?)?),
        "switch" => boxed(repo.switch(parse(request)?)?),
        "git_import" => boxed(repo.git_import(parse(request)?)?),
        "git_export" => boxed(repo.git_export(parse(request)?)?),
        _ => return Err(unknown(operation)),
    })
}

/// Starts `clone`, which makes a new repository from a remote.
pub fn clone(request: Value) -> Result<Box<dyn JsonJob>> {
    Ok(boxed(Repository::clone_from(parse(request)?)?))
}

/// The request of an operation that takes none.
#[derive(schemars::JsonSchema)]
struct NoRequest {}

/// Every operation with its request and result, and the values shared by all
/// (SDK.md §3): `sdk/schema/apeiron.schema.json` is this.
pub fn schema() -> Value {
    use crate::*;
    let mut generator = schemars::SchemaGenerator::default();
    let mut operations = serde_json::Map::new();
    macro_rules! ops {
        ($long:literal: $($name:literal $request:ty => $result:ty,)*) => {$(
            operations.insert($name.to_owned(), serde_json::json!({
                "long": $long,
                "request": generator.subschema_for::<$request>(),
                "result": generator.subschema_for::<$result>(),
            }));
        )*};
    }
    ops!(false:
        "info" NoRequest => RepoInfo,
        "resolve" Spec => ObjectId,
        "log" LogRequest => LogPage,
        "list" ListRequest => Vec<EntryInfo>,
        "diff" DiffRequest => Vec<Change>,
        "diff_text" DiffRequest => Vec<TextDiff>,
        "duplicates" DuplicatesRequest => Vec<Duplicate>,
        "refs" NoRequest => Vec<RefInfo>,
        "reflog" Named => Vec<ReflogEntry>,
        "create_ref" CreateRefRequest => ObjectId,
        "delete_ref" DeleteRefRequest => (),
        "merge" MergeRequest => MergeReport,
        "cherry_pick" MergeRequest => MergeReport,
        "rebase" MergeRequest => MergeReport,
        "prune" PruneRequest => PruneReport,
        "repair" NoRequest => RepairReport,
        "keys" NoRequest => Vec<KeyInfo>,
        "add_password" Password => AddedKey,
        "add_recovery_key" NoRequest => AddedKey,
        "remove_key" KeyId => String,
        "remotes" NoRequest => Vec<Remote>,
        "add_remote" NewRemote => Remote,
        "remove_remote" Named => (),
        "holds" NoRequest => Vec<LegalHold>,
        "add_hold" NewHold => LegalHold,
        "lift_hold" LiftHold => LegalHold,
        "retention" NoRequest => Vec<RetentionRule>,
        "add_retention" NewRetention => RetentionRule,
        "plugins" PluginDir => Vec<PluginInfo>,
        "plugin_add" PluginFile => PluginAdded,
        "plugin_remove" PluginName => PluginInfo,
        "saved_searches" SearchesFile => Vec<SavedSearch>,
        "save_search" SaveSearch => (),
        "forget_search" SearchName => SavedSearch,
        "conflicts" Folder => Vec<ConflictCopy>,
        "resolve_conflict" Resolve => ConflictCopy,
    );
    ops!(true:
        "backup" BackupRequest => BackupReport,
        "restore" RestoreRequest => RestoreReport,
        "export" ExportRequest => ExportReport,
        "import_archive" ImportRequest => ImportReport,
        "verify" NoRequest => VerifyReport,
        "gc" GcRequest => GcReport,
        "gc_remote" RemoteGcRequest => GcReport,
        "push" TransferRequest => TransferReport,
        "pull" TransferRequest => TransferReport,
        "push_all" RemoteRequest => MirrorReport,
        "pull_all" RemoteRequest => MirrorReport,
        "report" ReportRequest => ComplianceReport,
        "repair_from" RemoteRequest => CopyRepairReport,
        "restore_test" RestoreTestRequest => RestoreTestReport,
        "search" SearchRequest => SearchReport,
        "sync" SyncRequest => SyncReport,
        "switch" SwitchRequest => SwitchReport,
        "git_import" GitRequest => GitImportReport,
        "git_export" GitRequest => GitExportReport,
        "clone" CloneRequest => CloneReport,
    );
    let progress = generator.subschema_for::<Progress>();
    let error = generator.subschema_for::<Error>();
    let definitions = generator.take_definitions(true);
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Apeiron operations",
        "version": crate::API_VERSION,
        "progress": progress,
        "error": error,
        "operations": operations,
        "$defs": definitions,
    })
}
