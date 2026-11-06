//! The repository handle (API.md §8.2, §8.3).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use apeiron_core as core;
use apeiron_services as services;

use crate::MergeReport;
use crate::error::{Error, ErrorKind, Result};
use crate::job::Job;
use crate::report::{ComplianceReport, ReportRequest};
use crate::spec;
use crate::types::{
    AddedKey, CloneReport, CloneRequest, ClonedRef, DeleteRefRequest, DiffRequest, ExportRequest,
    GitRequest, ListRequest, LogPage, LogRequest, RefInfo, RepoInfo, RestoreRequest,
    RestoreTestRequest, SnapshotInfo, SyncRequest, TransferRequest,
};
use crate::types::{
    CreateRefRequest, DuplicatesRequest, MergeRequest, MirrorReport, MirroredRef, RemoteGcRequest,
    RemoteRequest, SearchRequest, SwitchRequest, TextDiff,
};
use crate::{
    AddFileReport, AddFileRequest, ArchiveFormat, BackupReport, BackupRequest, Change, EntryInfo,
    ExportReport, GcReport, GcRequest, GitExportReport, GitImportReport, GitPushResult,
    GitPushedRef, GitServedRef, ImportReport, ImportRequest, KeyInfo, LegalHold, ObjectId, Phase,
    PluginAdded, PluginInfo, PruneReport, PruneRequest, ReflogEntry, Remote, RepairReport,
    RestoreReport, RestoreTestReport, RetentionRule, SearchReport, SwitchReport, SyncReport,
    TransferReport, VerifyReport,
};

/// The ref the plugins a repository carries are kept in (PLUGIN.md §5).
const PLUGINS_REF: &str = "apeiron/plugins";

/// An open repository. Cloning is cheap; clones share the same state and can
/// be used from several threads at once.
#[derive(Clone, Debug)]
pub struct Repository {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    /// The view readers use. A backup writes through its own instance and
    /// replaces this one when it is done.
    core: RwLock<core::Repository>,
    /// Lets one writer at a time into the repository within this process.
    writer: Mutex<()>,
    /// The type drivers merges use (PLUGIN.md §4).
    plugins: Mutex<PluginSlot>,
}

#[derive(Debug)]
enum PluginSlot {
    /// Read from [`apeiron_plugin::default_dir`] when first needed.
    Default,
    /// Read from this folder when first needed.
    Dir(PathBuf),
    Loaded(Option<Arc<apeiron_plugin::Plugins>>),
}

impl Repository {
    /// Creates an empty repository in a new or empty folder and opens it.
    pub fn init(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Ok(Repository::wrap(path, core::Repository::init(path)?))
    }

    /// Creates an encrypted repository in a new or empty folder and opens
    /// it. Returns the recovery key, which is shown to the user once: with
    /// every password lost, it is the only way back to the data.
    pub fn init_encrypted(path: impl AsRef<Path>, password: &str) -> Result<(Self, String)> {
        if password.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the password must not be empty",
            ));
        }
        let path = path.as_ref();
        let (core, recovery) =
            core::Repository::init_encrypted(path, core::ChunkerParams::default(), password)?;
        Ok((Repository::wrap(path, core), recovery))
    }

    /// Opens a repository. An encrypted one fails with [`ErrorKind::Key`];
    /// see [`Repository::unlock`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Ok(Repository::wrap(path, core::Repository::open(path)?))
    }

    /// Opens an encrypted repository with a password or its recovery key.
    /// A repository that is not encrypted opens without using `secret`.
    pub fn unlock(path: impl AsRef<Path>, secret: &str) -> Result<Self> {
        let path = path.as_ref();
        Ok(Repository::wrap(
            path,
            core::Repository::unlock(path, secret)?,
        ))
    }

    /// Makes a new repository in a new or empty folder from a remote copy
    /// and pulls every ref of it; the remote is then known as `origin`
    /// ([FORMAT.md](FORMAT.md) §15.6). Open the result with `unlock`.
    pub fn clone_from(request: CloneRequest) -> Result<Job<CloneReport>> {
        let url = absolute_url(&request.url)?;
        let remote = Remote {
            name: ORIGIN.to_owned(),
            url,
        };
        Job::spawn("clone", move |ctl| {
            ctl.update(|p| p.phase = Phase::Copying);
            // The copy is already where the user stored it; whether it is
            // encrypted is known only after reading it.
            let store = open_store(&remote, true, request.rate)?;
            let (core, reports) = core::Repository::clone_from(
                &*store,
                &request.path,
                request.secret.as_deref(),
                &mut |r| copied(ctl, r),
            )?;
            core.add_remote(&remote.name, &remote.url)?;
            ctl.update(|p| p.phase = Phase::Done);
            Ok(CloneReport {
                refs: reports
                    .into_iter()
                    .map(|(name, report)| ClonedRef { name, report })
                    .collect(),
            })
        })
    }

    fn wrap(path: &Path, core: core::Repository) -> Self {
        Repository {
            inner: Arc::new(Inner {
                path: path.to_path_buf(),
                core: RwLock::new(core),
                writer: Mutex::new(()),
                plugins: Mutex::new(PluginSlot::Default),
            }),
        }
    }

    /// Uses the plugins installed in `dir` instead of the user's plugin
    /// folder (PLUGIN.md §1).
    pub fn use_plugin_dir(&self, dir: impl Into<PathBuf>) {
        *self.plugin_slot() = PluginSlot::Dir(dir.into());
    }

    fn plugin_slot(&self) -> MutexGuard<'_, PluginSlot> {
        self.inner
            .plugins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The installed plugins, read once; `None` without a plugin folder.
    fn plugins(&self) -> Result<Option<Arc<apeiron_plugin::Plugins>>> {
        let mut slot = self.plugin_slot();
        let dir = match &*slot {
            PluginSlot::Loaded(p) => return Ok(p.clone()),
            PluginSlot::Dir(dir) => Some(dir.clone()),
            PluginSlot::Default => apeiron_plugin::default_dir().ok(),
        };
        let plugins = match dir {
            Some(dir) => Some(Arc::new(apeiron_plugin::Plugins::open(dir)?)),
            None => None,
        };
        *slot = PluginSlot::Loaded(plugins.clone());
        Ok(plugins)
    }

    fn read(&self) -> RwLockReadGuard<'_, core::Repository> {
        self.inner
            .core
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the repository lock shared (FORMAT.md §12) and returns the
    /// current view, reloaded if another program changed the index files.
    pub(crate) fn view(&self) -> Result<View<'_>> {
        let lock = core::RepoLock::shared(self.path())?;
        self.write().refresh_index()?;
        Ok(View {
            core: self.read(),
            _lock: lock,
        })
    }

    fn write(&self) -> RwLockWriteGuard<'_, core::Repository> {
        self.inner
            .core
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_writer(&self) -> MutexGuard<'_, ()> {
        self.inner
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    pub fn info(&self) -> RepoInfo {
        let core = self.read();
        let chunker = core.chunker();
        RepoInfo {
            damaged_files: core.damaged_files().to_vec(),
            path: self.inner.path.clone(),
            format_version: core::FORMAT_VERSION,
            api_version: crate::API_VERSION.to_owned(),
            chunk_min: chunker.min_size(),
            chunk_avg: chunker.avg_size(),
            chunk_max: chunker.max_size(),
            encrypted: core.is_encrypted(),
        }
    }

    /// Resolves a snapshot spec (API.md §8.4) to a snapshot id.
    pub fn resolve(&self, spec: &str) -> Result<ObjectId> {
        let view = self.view()?;
        spec::resolve(&view, spec)
    }

    /// Stores a folder as a new snapshot and moves the ref to it.
    pub fn backup(&self, request: BackupRequest) -> Result<Job<BackupReport>> {
        let repo = self.clone();
        Job::spawn("backup", move |ctl| {
            let _writer = repo.lock_writer();
            // The index is loaded after the lock is taken (FORMAT.md §12).
            let _lock = core::RepoLock::shared(repo.path())?;
            let mut core = repo.read().reopen()?;
            let report = services::backup(&mut core, &request, ctl)?;
            *repo.write() = core;
            Ok(report)
        })
    }

    /// Stores a tar or zip archive as a new snapshot of a ref (CLI.md §4.2).
    /// If it is damaged, the ref does not move.
    pub fn import_archive(&self, request: ImportRequest) -> Result<Job<ImportReport>> {
        let repo = self.clone();
        Job::spawn("import", move |ctl| {
            let _writer = repo.lock_writer();
            let _lock = core::RepoLock::shared(repo.path())?;
            let mut core = repo.read().reopen()?;
            let report = services::import(&mut core, &request, ctl)?;
            *repo.write() = core;
            Ok(report)
        })
    }

    /// Runs one sync round of a folder with a remote (SYNC.md §3): backs up
    /// the folder, pulls, merges if both sides changed, changes the folder
    /// and pushes. Fails with [`ErrorKind::Conflict`] in `send` mode when
    /// the remote has snapshots this copy does not have.
    pub fn sync(&self, request: SyncRequest) -> Result<Job<SyncReport>> {
        let remote = self.read().remote(&request.remote)?;
        let ref_name = match request.ref_name {
            Some(name) => name,
            None => format!("sync/{}", folder_name(&request.folder)?),
        };
        let options = services::SyncOptions {
            folder: request.folder,
            ref_name,
            mode: request.mode,
            dry_run: request.dry_run,
            host: gethostname::gethostname().into_string().ok(),
            exclude: request.exclude,
        };
        let repo = self.clone();
        Job::spawn("sync", move |ctl| {
            let store = open_store(&remote, repo.info().encrypted, request.rate)?;
            let _writer = repo.lock_writer();
            let _lock = core::RepoLock::shared(repo.path())?;
            let plugins = repo.plugins()?;
            let mut core = repo.read().reopen()?;
            let merger = plugins
                .as_deref()
                .map(|p| p as &dyn services::ContentMerger);
            let report = services::sync(&mut core, &*store, &options, merger, ctl);
            // Refs may have moved even when a later step failed.
            *repo.write() = core;
            Ok(report?)
        })
    }

    /// Makes a working folder hold a snapshot, usually a branch, recording
    /// what it held first under `folder/switch/<folder name>` (BRANCH.md
    /// §6).
    pub fn switch(&self, request: SwitchRequest) -> Result<Job<SwitchReport>> {
        let options = services::SwitchOptions {
            target: self.resolve(&request.snapshot)?,
            target_name: request.snapshot,
            name: format!("switch/{}", folder_name(&request.folder)?),
            folder: request.folder,
            dry_run: request.dry_run,
            host: gethostname::gethostname().into_string().ok(),
        };
        let repo = self.clone();
        Job::spawn("switch", move |ctl| {
            let _writer = repo.lock_writer();
            let _lock = core::RepoLock::shared(repo.path())?;
            let mut core = repo.read().reopen()?;
            let report = services::switch(&mut core, &options, ctl);
            *repo.write() = core;
            Ok(report?)
        })
    }

    /// Stores every branch, tag and remote-tracking branch of a Git
    /// repository, with its history, under `<prefix>/` (GIT.md §4).
    pub fn git_import(&self, request: GitRequest) -> Result<Job<GitImportReport>> {
        let repo = self.clone();
        Job::spawn("git-import", move |ctl| {
            ctl.update(|p| p.phase = Phase::Storing);
            let _writer = repo.lock_writer();
            let _lock = core::RepoLock::shared(repo.path())?;
            let mut core = repo.read().reopen()?;
            let report = apeiron_git::import(&mut core, &request.path, &request.prefix, &mut |n| {
                git_progress(ctl, n)
            });
            *repo.write() = core;
            ctl.update(|p| p.phase = Phase::Done);
            Ok(report?)
        })
    }

    /// Writes the snapshots of the refs under `<prefix>/` into a Git
    /// repository, created bare if it does not exist (GIT.md §5).
    pub fn git_export(&self, request: GitRequest) -> Result<Job<GitExportReport>> {
        let repo = self.clone();
        Job::spawn("git-export", move |ctl| {
            ctl.update(|p| p.phase = Phase::Exporting);
            let core = repo.view()?;
            let report = apeiron_git::export(&core, &request.path, &request.prefix, &mut |n| {
                git_progress(ctl, n)
            })?;
            ctl.update(|p| p.phase = Phase::Done);
            Ok(report)
        })
    }

    /// Brings the Git copy at `request.path` up to date with the branches
    /// and tags under `request.prefix` and lists them, for serving them to
    /// Git clients (GIT.md §7).
    pub fn git_serve_refs(&self, request: &GitRequest) -> Result<Vec<GitServedRef>> {
        let core = self.view()?;
        Ok(apeiron_git::serve_refs(
            &core,
            &request.path,
            &request.prefix,
        )?)
    }

    /// Stores what `reader` gives as a new file in a folder of the snapshot
    /// a ref points to, as a new snapshot on that ref; nothing is replaced
    /// (SHARE.md §7).
    pub fn add_file(
        &self,
        request: &AddFileRequest,
        reader: &mut dyn std::io::Read,
    ) -> Result<AddFileReport> {
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.read().reopen()?;
        let mut request = request.clone();
        if request.host.is_none() {
            request.host = gethostname::gethostname().into_string().ok();
        }
        let report = services::add_file(&mut core, &request, reader);
        *self.write() = core;
        Ok(report?)
    }

    /// Puts the installed plugin `name` from the plugin folder `dir` (the
    /// user's by default) into the repository's ref `apeiron/plugins`, so
    /// copies of the repository carry it (PLUGIN.md §5).
    pub fn share_plugin(&self, name: &str, dir: Option<&Path>) -> Result<PluginInfo> {
        let info = crate::plugins(dir)?
            .into_iter()
            .find(|i| i.name == name)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::NotFound,
                    format!("no plugin {name} is installed"),
                )
            })?;
        let dir = match dir {
            Some(d) => d.to_path_buf(),
            None => crate::plugin_dir()?,
        };
        let mut put = Vec::new();
        for ext in ["wasm", "json"] {
            let path = dir.join(format!("{name}.{ext}"));
            let data = std::fs::read(&path).map_err(|e| Error::io(path.clone(), &e))?;
            put.push((format!("{name}.{ext}").into_bytes(), data));
        }
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.read().reopen()?;
        let message = format!("share the plugin {name} {}", info.version);
        let stored = services::put_files(&mut core, PLUGINS_REF, &put, &[], &message);
        *self.write() = core;
        stored?;
        Ok(info)
    }

    /// Takes the plugin `name` out of `apeiron/plugins`; installed copies
    /// stay.
    pub fn unshare_plugin(&self, name: &str) -> Result<()> {
        if !self.shared_plugins()?.iter().any(|i| i.name == name) {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("the repository holds no plugin {name}"),
            ));
        }
        let remove = [
            format!("{name}.wasm").into_bytes(),
            format!("{name}.json").into_bytes(),
        ];
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.read().reopen()?;
        let stored = services::put_files(
            &mut core,
            PLUGINS_REF,
            &[],
            &remove,
            &format!("unshare the plugin {name}"),
        );
        *self.write() = core;
        stored?;
        Ok(())
    }

    /// The plugins the repository carries in `apeiron/plugins`.
    pub fn shared_plugins(&self) -> Result<Vec<PluginInfo>> {
        let entries = match self.list(&ListRequest {
            snapshot: PLUGINS_REF.to_owned(),
            path: crate::Name::default(),
        }) {
            Ok(entries) => entries,
            Err(e) if e.kind == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for e in entries {
            if !e.name.to_string().ends_with(".json") {
                continue;
            }
            let mut data = Vec::new();
            self.read_file(
                &ListRequest {
                    snapshot: PLUGINS_REF.to_owned(),
                    path: e.name.clone(),
                },
                &mut data,
            )?;
            if let Ok(info) = serde_json::from_slice::<PluginInfo>(&data) {
                out.push(info);
            }
        }
        Ok(out)
    }

    /// Installs the plugin `name` the repository carries, checked as
    /// [`crate::add_plugin`] checks a file, into `dir` (the user's plugin
    /// folder by default).
    pub fn install_shared_plugin(&self, name: &str, dir: Option<&Path>) -> Result<PluginAdded> {
        if !self.shared_plugins()?.iter().any(|i| i.name == name) {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("the repository holds no plugin {name}"),
            ));
        }
        let mut wasm = Vec::new();
        self.read_file(
            &ListRequest {
                snapshot: PLUGINS_REF.to_owned(),
                path: crate::Name::from(format!("{name}.wasm").as_str()),
            },
            &mut wasm,
        )?;
        let file =
            std::env::temp_dir().join(format!("apeiron-plugin-{}-{name}.wasm", std::process::id()));
        std::fs::write(&file, &wasm).map_err(|e| Error::io(file.clone(), &e))?;
        let added = crate::add_plugin(&file, dir);
        let _ = std::fs::remove_file(&file);
        added
    }

    /// Takes `git push` into the Git copy at `request.path`: stores the
    /// objects of `pack` and the commits the refs reach under
    /// `request.prefix`, and moves each ref that `may` allows, that moves
    /// forward and that is still where the client saw it (GIT.md §7).
    pub fn git_receive(
        &self,
        request: &GitRequest,
        updates: &[GitPushedRef],
        pack: &[u8],
        may: &dyn Fn(&str) -> bool,
    ) -> Result<Vec<GitPushResult>> {
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.read().reopen()?;
        let results = apeiron_git::receive(
            &mut core,
            &request.path,
            &request.prefix,
            updates,
            pack,
            may,
        );
        // Refs may have moved even when a later step failed.
        *self.write() = core;
        Ok(results?)
    }

    /// The Git ids of `haves` that `wants` reach in the Git copy at `path`:
    /// what a client already has (GIT.md §7).
    pub fn git_common(
        &self,
        path: &Path,
        wants: &[String],
        haves: &[String],
    ) -> Result<Vec<String>> {
        Ok(apeiron_git::common_haves(path, wants, haves)?)
    }

    /// Writes a Git pack of everything the Git ids `wants` reach in the Git
    /// copy at `path`, made by [`Repository::git_serve_refs`], and `common`
    /// from [`Repository::git_common`] does not; returns the number of
    /// objects (GIT.md §7).
    pub fn git_pack(
        &self,
        path: &Path,
        wants: &[String],
        common: &[String],
        out: &mut dyn Write,
    ) -> Result<u64> {
        Ok(apeiron_git::write_pack(path, wants, common, out)?)
    }

    /// Removes from a remote the objects its refs cannot reach and rewrites
    /// its partly dead packs, from here (FORMAT.md §15.8). Only for a remote
    /// this repository alone writes to, such as a replica. Fails with
    /// [`ErrorKind::Locked`] while any other operation uses this repository
    /// or another cleanup runs on the remote.
    pub fn gc_remote(&self, request: RemoteGcRequest) -> Result<Job<GcReport>> {
        let remote = self.read().remote(&request.remote)?;
        let repo = self.clone();
        Job::spawn("gc-remote", move |ctl| {
            let _writer = repo.lock_writer();
            ctl.update(|p| p.phase = crate::Phase::Finishing);
            let store = open_store(&remote, repo.info().encrypted, None)?;
            let host = gethostname::gethostname().into_string().ok();
            let report = repo
                .read()
                .gc_remote(&*store, request.dry_run, host.as_deref())?;
            ctl.update(|p| p.phase = crate::Phase::Done);
            Ok(report)
        })
    }

    /// Removes objects no ref and no recent reflog line can reach. Fails with
    /// [`ErrorKind::Locked`] while any other operation uses the repository.
    pub fn gc(&self, request: GcRequest) -> Result<Job<GcReport>> {
        let repo = self.clone();
        Job::spawn("gc", move |ctl| {
            let _writer = repo.lock_writer();
            ctl.update(|p| p.phase = crate::Phase::Finishing);
            let mut core = repo.read().reopen()?;
            let report = core.gc(&request)?;
            *repo.write() = core;
            ctl.update(|p| p.phase = crate::Phase::Done);
            Ok(report)
        })
    }

    /// Writes a snapshot, or one path in it, into a new or empty folder.
    pub fn restore(&self, request: RestoreRequest) -> Result<Job<RestoreReport>> {
        let snapshot = self.resolve(&request.snapshot)?;
        let repo = self.clone();
        Job::spawn("restore", move |ctl| {
            let core = repo.view()?;
            Ok(services::restore(
                &core,
                &snapshot,
                request.path.as_bytes(),
                &request.target,
                ctl,
            )?)
        })
    }

    /// Restores a random sample of a snapshot into a temporary folder,
    /// checks every file against the snapshot and removes the folder. Damage
    /// is reported in the result, not as an error.
    pub fn restore_test(&self, request: RestoreTestRequest) -> Result<Job<RestoreTestReport>> {
        let snapshot = self.resolve(&request.snapshot)?;
        let repo = self.clone();
        Job::spawn("restore-test", move |ctl| {
            let core = repo.view()?;
            Ok(services::restore_test(
                &core,
                &snapshot,
                request.sample_bytes,
                &std::env::temp_dir(),
                ctl,
            )?)
        })
    }

    /// Writes a snapshot, or one path in it, into a new tar, zip or 7z file.
    pub fn export(&self, request: ExportRequest) -> Result<Job<ExportReport>> {
        let snapshot = self.resolve(&request.snapshot)?;
        let format = match request.format {
            Some(format) => format,
            None => ArchiveFormat::from_path(&request.output).ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "cannot tell the archive format of {}; use .tar, .zip or .7z",
                        request.output.display()
                    ),
                )
            })?,
        };
        let repo = self.clone();
        Job::spawn("export", move |ctl| {
            let core = repo.view()?;
            Ok(services::export(
                &core,
                &snapshot,
                request.path.as_bytes(),
                &request.output,
                format,
                ctl,
            )?)
        })
    }

    /// A compliance report: what the repository keeps, how fresh and whole
    /// it is, how it is protected and whether its remotes are current
    /// ([REPORTS.md](REPORTS.md)).
    pub fn report(&self, request: ReportRequest) -> Result<Job<ComplianceReport>> {
        let repo = self.clone();
        Job::spawn("report", move |ctl| {
            crate::report::make(&repo, &request, ctl)
        })
    }

    /// Reads and checks every pack, index and object.
    pub fn verify(&self) -> Result<Job<VerifyReport>> {
        let repo = self.clone();
        Job::spawn("verify", move |ctl| {
            ctl.update(|p| p.phase = crate::Phase::Verifying);
            let report = repo.view()?.verify()?;
            ctl.update(|p| p.phase = crate::Phase::Done);
            Ok(report)
        })
    }

    /// One page of history, newest first.
    pub fn log(&self, request: &LogRequest) -> Result<LogPage> {
        let core = self.view()?;
        let start = spec::resolve(&core, &request.start)?;
        let needle = request.grep.as_deref().map(str::to_lowercase);
        let mut snapshots = Vec::new();
        let mut next = Some(start);
        while let Some(id) = next {
            if snapshots.len() == request.limit {
                break;
            }
            let snapshot = core.read_snapshot(&id)?;
            next = snapshot.parents.first().copied();
            let info = SnapshotInfo::new(id, snapshot);
            if needle.as_deref().is_none_or(|n| info.mentions(n)) {
                snapshots.push(info);
            }
        }
        Ok(LogPage { snapshots, next })
    }

    /// Lists the folder at a path, or the single entry if the path is a file.
    pub fn list(&self, request: &ListRequest) -> Result<Vec<EntryInfo>> {
        let core = self.view()?;
        let snapshot = spec::resolve(&core, &request.snapshot)?;
        Ok(services::list(&core, &snapshot, request.path.as_bytes())?)
    }

    /// Writes the content of the file at a path to `out`, as a stream, and
    /// returns its size; a folder or a link there is `invalid_input`.
    pub fn read_file(&self, request: &ListRequest, out: impl std::io::Write) -> Result<u64> {
        let core = self.view()?;
        let snapshot = spec::resolve(&core, &request.snapshot)?;
        Ok(services::read_to(
            &core,
            &snapshot,
            request.path.as_bytes(),
            out,
        )?)
    }

    /// Writes the folder or file at a path as a zip archive into `out`, as
    /// a stream, for a download; nothing is written to disk.
    pub fn export_zip_to(
        &self,
        request: &ListRequest,
        out: impl std::io::Write,
    ) -> Result<ExportReport> {
        let core = self.view()?;
        let snapshot = spec::resolve(&core, &request.snapshot)?;
        Ok(services::export_zip_to(
            &core,
            &snapshot,
            request.path.as_bytes(),
            out,
            &services::Control::new(),
        )?)
    }

    /// Paths added, removed and modified between two snapshots.
    pub fn diff(&self, request: &DiffRequest) -> Result<Vec<Change>> {
        let core = self.view()?;
        let from = spec::resolve(&core, &request.from)?;
        let to = spec::resolve(&core, &request.to)?;
        Ok(services::diff(&core, &from, &to)?)
    }

    /// The files of a snapshot whose content is at more than one path, the
    /// groups taking the most room by their extra copies first (SEARCH.md §5).
    pub fn duplicates(&self, request: &DuplicatesRequest) -> Result<Vec<crate::Duplicate>> {
        let core = self.view()?;
        let snapshot = spec::resolve(&core, &request.snapshot)?;
        Ok(services::duplicates(
            &core,
            &snapshot,
            request.smallest,
            &services::Control::new(),
        )?)
    }

    /// The changed paths between two snapshots with the difference the
    /// plugin handling each file shows (TEXT.md §3).
    pub fn diff_text(&self, request: &DiffRequest) -> Result<Vec<TextDiff>> {
        let core = self.view()?;
        let from = spec::resolve(&core, &request.from)?;
        let to = spec::resolve(&core, &request.to)?;
        let plugins = self.plugins()?;
        let limit = services::PLUGIN_SIZE_LIMIT;
        let mut out = Vec::new();
        for change in services::diff(&core, &from, &to)? {
            let path = change.path.as_bytes();
            let name = path.rsplit(|&b| b == b'/').next().unwrap_or(path);
            let mut diff = TextDiff {
                path: change.path.clone(),
                kind: change.kind,
                text: None,
                error: None,
            };
            if let Some(plugins) = plugins.as_deref().filter(|p| {
                use services::ContentMerger;
                p.handles(name)
            }) {
                let old = services::read_path(&core, &from, path, limit)?;
                let new = services::read_path(&core, &to, path, limit)?;
                // An added or removed file is compared with an empty one.
                if old.is_some() || new.is_some() {
                    let (old, new) = (old.unwrap_or_default(), new.unwrap_or_default());
                    match plugins.diff_file(name, &old, &new) {
                        Ok(text) => diff.text = text,
                        Err(e) => diff.error = Some(e.to_string()),
                    }
                }
            }
            out.push(diff);
        }
        Ok(out)
    }

    /// Finds files by name, metadata and text in a snapshot or in its
    /// history (SEARCH.md).
    pub fn search(&self, request: SearchRequest) -> Result<Job<SearchReport>> {
        let starts = match &request.snapshot {
            Some(spec) => vec![self.resolve(spec)?],
            None if request.history => self
                .view()?
                .list_refs()?
                .into_iter()
                .map(|(_, id)| id)
                .collect(),
            None => vec![self.resolve("main")?],
        };
        let repo = self.clone();
        Job::spawn("search", move |ctl| {
            let core = repo.view()?;
            let options = services::SearchOptions {
                starts,
                history: request.history,
                query: request.query,
                limit: request.limit,
            };
            Ok(services::search(&core, &options, ctl)?)
        })
    }

    pub fn refs(&self) -> Result<Vec<RefInfo>> {
        Ok(self
            .view()?
            .list_refs()?
            .into_iter()
            .map(|(name, snapshot)| RefInfo { name, snapshot })
            .collect())
    }

    pub fn reflog(&self, name: &str) -> Result<Vec<ReflogEntry>> {
        Ok(self.view()?.reflog(name)?)
    }

    /// Deletes a ref if it still points to the expected snapshot. The
    /// snapshots stay until garbage collection removes them.
    pub fn delete_ref(&self, request: &DeleteRefRequest) -> Result<()> {
        let _writer = self.lock_writer();
        Ok(self
            .view()?
            .update_ref(&request.name, Some(request.expected), None)?)
    }

    /// Creates the ref `name` pointing to a snapshot; fails with
    /// [`ErrorKind::AlreadyExists`] if the ref exists (BRANCH.md §1).
    pub fn create_ref(&self, request: &CreateRefRequest) -> Result<ObjectId> {
        let _writer = self.lock_writer();
        let view = self.view()?;
        let snapshot = spec::resolve(&view, &request.snapshot)?;
        if view.read_ref(&request.name)?.is_some() {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!("ref {} already exists", request.name),
            ));
        }
        view.update_ref(&request.name, None, Some(snapshot))?;
        Ok(snapshot)
    }

    /// Merges a snapshot into a ref (BRANCH.md §2): nothing to do, a
    /// fast-forward, or a new snapshot with both as parents.
    pub fn merge(&self, request: &MergeRequest) -> Result<MergeReport> {
        self.merge_with(request, services::merge)
    }

    /// Applies the changes the snapshot `request.from` made to its parent
    /// onto the ref `request.into`, as one new snapshot there
    /// ([BRANCH.md](BRANCH.md) §4).
    pub fn cherry_pick(&self, request: &MergeRequest) -> Result<MergeReport> {
        self.merge_with(request, services::cherry_pick)
    }

    /// Copies the snapshots of the ref `request.into` that `request.from`
    /// lacks on top of it, one by one, and moves the ref to the last copy
    /// ([BRANCH.md](BRANCH.md) §5); `request.message` is not used.
    pub fn rebase(&self, request: &MergeRequest) -> Result<MergeReport> {
        self.merge_with(request, services::rebase)
    }

    fn merge_with(
        &self,
        request: &MergeRequest,
        run: fn(
            &mut core::Repository,
            &services::MergeOptions,
            Option<&dyn services::ContentMerger>,
        ) -> services::Result<MergeReport>,
    ) -> Result<MergeReport> {
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.read().reopen()?;
        let options = services::MergeOptions {
            from: spec::resolve(&core, &request.from)?,
            from_name: request.from.clone(),
            into: request.into.clone(),
            message: request.message.clone(),
            author: request.author.clone(),
            host: gethostname::gethostname().into_string().ok(),
            dry_run: request.dry_run,
        };
        let plugins = self.plugins()?;
        let merger = plugins
            .as_deref()
            .map(|p| p as &dyn services::ContentMerger);
        let report = run(&mut core, &options, merger);
        *self.write() = core;
        Ok(report?)
    }

    /// The passwords and recovery keys of an encrypted repository, oldest
    /// first.
    pub fn keys(&self) -> Result<Vec<KeyInfo>> {
        Ok(self.read().list_keys()?)
    }

    /// Adds a password to an encrypted repository.
    pub fn add_password(&self, password: &str) -> Result<AddedKey> {
        if password.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the password must not be empty",
            ));
        }
        Ok(AddedKey {
            id: self.read().add_password(password)?,
            recovery_key: None,
        })
    }

    /// Adds a new recovery key to an encrypted repository. The key is in the
    /// result only; it is shown to the user once.
    pub fn add_recovery_key(&self) -> Result<AddedKey> {
        let (id, recovery_key) = self.read().add_recovery_key()?;
        Ok(AddedKey {
            id,
            recovery_key: Some(recovery_key),
        })
    }

    /// Removes a password or recovery key by its id or the start of it and
    /// returns the whole id. The last key cannot be removed. Fails with
    /// [`ErrorKind::Locked`] while any other operation uses the repository.
    pub fn remove_key(&self, id: &str) -> Result<String> {
        let core = self.read();
        let matches: Vec<KeyInfo> = core
            .list_keys()?
            .into_iter()
            .filter(|k| !id.is_empty() && k.id.starts_with(id))
            .collect();
        match matches.as_slice() {
            [key] => {
                core.remove_key(&key.id)?;
                Ok(key.id.clone())
            }
            [] => Err(Error::new(
                ErrorKind::NotFound,
                format!("no key with id {id:?}"),
            )),
            _ => Err(Error::new(
                ErrorKind::Ambiguous,
                format!("{id:?} is the start of more than one key id"),
            )),
        }
    }

    /// The retention rules, oldest first ([ARCHIVE.md](ARCHIVE.md) §2).
    pub fn retention(&self) -> Result<Vec<RetentionRule>> {
        Ok(self.read().retention_rules()?)
    }

    /// Adds a retention rule: the refs `refs` matches (a name, or a prefix
    /// followed by `*`) then only move forward and are not deleted until
    /// `days` after their last change. A rule can never be removed or
    /// shortened ([FORMAT.md](FORMAT.md) §16).
    pub fn add_retention(&self, refs: &str, days: u32) -> Result<RetentionRule> {
        let _writer = self.lock_writer();
        Ok(self.read().add_retention_rule(refs, days)?)
    }

    /// Every legal hold, lifted ones too, oldest first ([ARCHIVE.md](ARCHIVE.md)
    /// §2.1).
    pub fn legal_holds(&self) -> Result<Vec<LegalHold>> {
        Ok(self.read().legal_holds()?)
    }

    /// Places a legal hold on the refs `refs` matches (a name, or a prefix
    /// followed by `*`): until it is lifted they only move forward, are not
    /// deleted, and keep every version their reflog names
    /// ([FORMAT.md](FORMAT.md) §17).
    pub fn add_legal_hold(&self, refs: &str, reason: &str) -> Result<LegalHold> {
        let _writer = self.lock_writer();
        Ok(self.read().add_legal_hold(refs, reason)?)
    }

    /// Lifts the legal hold `id`, for `reason`; this cannot be undone.
    pub fn lift_legal_hold(&self, id: &str, reason: &str) -> Result<LegalHold> {
        let _writer = self.lock_writer();
        Ok(self.read().lift_legal_hold(id, reason)?)
    }

    /// The remotes of this copy, sorted by name.
    pub fn remotes(&self) -> Result<Vec<Remote>> {
        Ok(self.read().remotes()?)
    }

    /// Adds a remote. A folder may be given relative to the current folder;
    /// it is stored as an absolute path.
    pub fn add_remote(&self, name: &str, url: &str) -> Result<Remote> {
        let url = absolute_url(url)?;
        self.read().add_remote(name, &url)?;
        Ok(Remote {
            name: name.to_owned(),
            url,
        })
    }

    /// Forgets a remote; the copy it names is not touched.
    pub fn remove_remote(&self, name: &str) -> Result<()> {
        Ok(self.read().remove_remote(name)?)
    }

    /// Copies the snapshots of a ref to a remote and moves the ref there.
    /// Fails with [`ErrorKind::Conflict`] if the remote ref has snapshots
    /// this copy does not have.
    pub fn push(&self, request: TransferRequest) -> Result<Job<TransferReport>> {
        let remote = self.read().remote(&request.remote)?;
        let repo = self.clone();
        Job::spawn("push", move |ctl| {
            ctl.update(|p| p.phase = Phase::Copying);
            let store = open_store(&remote, repo.info().encrypted, request.rate)?;
            let view = repo.view()?;
            let report = if request.replace {
                view.push_replacing(&*store, &request.ref_name, &mut |r| copied(ctl, r))?
            } else {
                view.push(&*store, &request.ref_name, &mut |r| copied(ctl, r))?
            };
            ctl.update(|p| p.phase = Phase::Done);
            Ok(report)
        })
    }

    /// Copies the snapshots of a ref from a remote and moves the local ref
    /// forward. Fails with [`ErrorKind::Conflict`] if the local ref has
    /// snapshots the remote does not have.
    pub fn pull(&self, request: TransferRequest) -> Result<Job<TransferReport>> {
        let remote = self.read().remote(&request.remote)?;
        let repo = self.clone();
        Job::spawn("pull", move |ctl| {
            ctl.update(|p| p.phase = Phase::Copying);
            let store = open_store(&remote, repo.info().encrypted, request.rate)?;
            let _writer = repo.lock_writer();
            let _lock = core::RepoLock::shared(repo.path())?;
            let mut core = repo.read().reopen()?;
            let report = core.pull(&*store, &request.ref_name, &mut |r| copied(ctl, r))?;
            *repo.write() = core;
            ctl.update(|p| p.phase = Phase::Done);
            Ok(report)
        })
    }

    /// Pushes every ref to a remote, reading its index once
    /// ([REPLICATION.md](REPLICATION.md) §2). A ref the remote has moved
    /// past is reported in [`MirrorReport::conflicts`] and the others are
    /// still pushed.
    pub fn push_all(&self, request: RemoteRequest) -> Result<Job<MirrorReport>> {
        let remote = self.read().remote(&request.remote)?;
        let repo = self.clone();
        Job::spawn("push", move |ctl| {
            ctl.update(|p| p.phase = Phase::Copying);
            let store = open_store(&remote, repo.info().encrypted, request.rate)?;
            let outcomes = repo.view()?.push_all(&*store, &mut |r| copied(ctl, r))?;
            ctl.update(|p| p.phase = Phase::Done);
            Ok(mirrored(outcomes))
        })
    }

    /// Pulls every ref of a remote, reading its index once
    /// ([REPLICATION.md](REPLICATION.md) §2). A ref this copy has moved past
    /// is reported in [`MirrorReport::conflicts`] and the others are still
    /// pulled.
    pub fn pull_all(&self, request: RemoteRequest) -> Result<Job<MirrorReport>> {
        let remote = self.read().remote(&request.remote)?;
        let repo = self.clone();
        Job::spawn("pull", move |ctl| {
            ctl.update(|p| p.phase = Phase::Copying);
            let store = open_store(&remote, repo.info().encrypted, request.rate)?;
            let _writer = repo.lock_writer();
            let _lock = core::RepoLock::shared(repo.path())?;
            let mut core = repo.read().reopen()?;
            let outcomes = core.pull_all(&*store, &mut |r| copied(ctl, r))?;
            *repo.write() = core;
            ctl.update(|p| p.phase = Phase::Done);
            Ok(mirrored(outcomes))
        })
    }

    /// Removes stale ref locks, adds reflog lines lost in a crash and
    /// replaces damaged index files.
    pub fn repair(&self) -> Result<RepairReport> {
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.write();
        core.refresh_index()?;
        Ok(core.repair()?)
    }

    /// Keeps the snapshots of a ref the rules name and moves the ref to the
    /// copy of the newest (FORMAT.md §13.4). Freeing their room is left to
    /// [`Repository::gc`], and following on a remote to `push` with
    /// `replace`.
    pub fn prune(&self, request: &PruneRequest) -> Result<PruneReport> {
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.write();
        core.refresh_index()?;
        Ok(services::prune(&mut core, request)?)
    }

    /// Replaces damaged or missing objects from a whole copy on a remote and
    /// deletes the packs no longer needed (FORMAT.md §8.3). Fails with
    /// [`ErrorKind::Locked`] while any other operation uses the repository.
    pub fn repair_from(&self, request: RemoteRequest) -> Result<Job<core::CopyRepairReport>> {
        let remote = self.read().remote(&request.remote)?;
        let repo = self.clone();
        Job::spawn("repair", move |ctl| {
            let _writer = repo.lock_writer();
            ctl.update(|p| p.phase = Phase::Copying);
            let store = open_store(&remote, repo.info().encrypted, request.rate)?;
            let mut core = repo.read().reopen()?;
            let report = core.repair_from(&*store, &mut |objects| {
                ctl.update(|p| p.items_done = objects);
                if ctl.is_cancelled() {
                    return Err(core::Error::Cancelled);
                }
                Ok(())
            })?;
            *repo.write() = core;
            ctl.update(|p| p.phase = Phase::Done);
            Ok(report)
        })
    }
}

/// The last part of a folder's path, lowercased, with other characters
/// than `a-z 0-9 . _ -` replaced by `-`, to name a ref after it.
fn folder_name(folder: &Path) -> Result<String> {
    let absolute = std::path::absolute(folder).map_err(|e| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("{}: {e}", folder.display()),
        )
    })?;
    let name: String = absolute
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '_' | '-' => c,
            _ => '-',
        })
        .collect();
    let name = name.trim_start_matches('.').trim_end_matches(".lock");
    Ok(if name.is_empty() { "folder" } else { name }.to_owned())
}

/// Reports the commits and tags done and stops once cancelled.
fn git_progress(ctl: &services::Control, done: u64) -> apeiron_git::Result<()> {
    ctl.update(|p| p.items_done = done);
    if ctl.is_cancelled() {
        return Err(apeiron_git::Error::Cancelled);
    }
    Ok(())
}

/// The name `clone` gives the remote it copied.
const ORIGIN: &str = "origin";

/// A folder given relative to the current folder, as an absolute path;
/// other URLs as they are.
fn absolute_url(url: &str) -> Result<String> {
    if url.contains("://") {
        return Ok(url.to_owned());
    }
    let path = std::path::absolute(url)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("{url}: {e}")))?;
    Ok(path
        .to_str()
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("{} is not valid UTF-8", path.display()),
            )
        })?
        .to_owned())
}

/// Connects to a remote. Only an encrypted repository is sent over the
/// network (FORMAT.md §15.1).
pub(crate) fn open_store(
    remote: &Remote,
    encrypted: bool,
    rate: Option<u64>,
) -> Result<Box<dyn core::Store>> {
    let network_needs_encryption = || {
        if encrypted {
            return Ok(());
        }
        Err(Error::new(
            ErrorKind::InvalidInput,
            "only an encrypted repository can use an SFTP or S3 remote; \
             create one with `init --encrypt`",
        ))
    };
    let store: Box<dyn core::Store> = match core::RemoteUrl::parse(&remote.url)? {
        core::RemoteUrl::Folder(path) => Box::new(core::FolderStore::open(&path)?),
        core::RemoteUrl::Sftp(url) => {
            network_needs_encryption()?;
            let options = apeiron_remote::SshOptions::from_home()?;
            Box::new(apeiron_remote::SftpStore::connect(&url, &options)?)
        }
        core::RemoteUrl::S3(url) => {
            network_needs_encryption()?;
            Box::new(apeiron_remote::S3Store::connect(&url)?)
        }
    };
    Ok(match rate {
        Some(rate) => Box::new(core::Limited::new(store, rate)?),
        None => store,
    })
}

/// Reports the progress of a push or pull and stops it once cancelled.
fn mirrored(outcomes: Vec<core::RefOutcome>) -> MirrorReport {
    MirrorReport {
        refs: outcomes
            .into_iter()
            .map(|(name, result)| match result {
                Ok(report) => MirroredRef {
                    name,
                    report: Some(report),
                    conflict: None,
                },
                Err(e) => MirroredRef {
                    name,
                    report: None,
                    conflict: Some(e.into()),
                },
            })
            .collect(),
    }
}

fn copied(ctl: &services::Control, report: &TransferReport) -> core::Result<()> {
    ctl.update(|p| {
        p.items_done = report.objects;
        p.bytes_done = report.bytes;
    });
    if ctl.is_cancelled() {
        return Err(core::Error::Cancelled);
    }
    Ok(())
}

/// A view of the repository while the repository lock is held shared.
pub(crate) struct View<'a> {
    core: RwLockReadGuard<'a, core::Repository>,
    _lock: core::RepoLock,
}

impl std::ops::Deref for View<'_> {
    type Target = core::Repository;

    fn deref(&self) -> &core::Repository {
        &self.core
    }
}
