//! The repository handle (API.md §8.2, §8.3).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use apeiron_core as core;
use apeiron_services as services;

use crate::error::{Error, ErrorKind, Result};
use crate::job::Job;
use crate::spec;
use crate::types::{
    DeleteRefRequest, DiffRequest, ExportRequest, ListRequest, LogPage, LogRequest, RefInfo,
    RepoInfo, RestoreRequest, SnapshotInfo,
};
use crate::{
    ArchiveFormat, BackupReport, BackupRequest, Change, EntryInfo, ExportReport, GcReport,
    GcRequest, ObjectId, ReflogEntry, RepairReport, RestoreReport, VerifyReport,
};

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
}

impl Repository {
    /// Creates an empty repository in a new or empty folder and opens it.
    pub fn init(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Ok(Repository::wrap(path, core::Repository::init(path)?))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Ok(Repository::wrap(path, core::Repository::open(path)?))
    }

    fn wrap(path: &Path, core: core::Repository) -> Self {
        Repository {
            inner: Arc::new(Inner {
                path: path.to_path_buf(),
                core: RwLock::new(core),
                writer: Mutex::new(()),
            }),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, core::Repository> {
        self.inner
            .core
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the repository lock shared (FORMAT.md §12) and returns the
    /// current view, reloaded if another program changed the index files.
    fn view(&self) -> Result<View<'_>> {
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
            let mut core = core::Repository::open(repo.path())?;
            let report = services::backup(&mut core, &request, ctl)?;
            *repo.write() = core;
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
            let mut core = core::Repository::open(repo.path())?;
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
        let mut snapshots = Vec::new();
        let mut next = Some(start);
        while let Some(id) = next {
            if snapshots.len() == request.limit {
                break;
            }
            let snapshot = core.read_snapshot(&id)?;
            next = snapshot.parents.first().copied();
            snapshots.push(SnapshotInfo::new(id, snapshot));
        }
        Ok(LogPage { snapshots, next })
    }

    /// Lists the folder at a path, or the single entry if the path is a file.
    pub fn list(&self, request: &ListRequest) -> Result<Vec<EntryInfo>> {
        let core = self.view()?;
        let snapshot = spec::resolve(&core, &request.snapshot)?;
        Ok(services::list(&core, &snapshot, request.path.as_bytes())?)
    }

    /// Paths added, removed and modified between two snapshots.
    pub fn diff(&self, request: &DiffRequest) -> Result<Vec<Change>> {
        let core = self.view()?;
        let from = spec::resolve(&core, &request.from)?;
        let to = spec::resolve(&core, &request.to)?;
        Ok(services::diff(&core, &from, &to)?)
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

    /// Removes stale ref locks, adds reflog lines lost in a crash and
    /// replaces damaged index files.
    pub fn repair(&self) -> Result<RepairReport> {
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.write();
        core.refresh_index()?;
        Ok(core.repair()?)
    }
}

/// A view of the repository while the repository lock is held shared.
struct View<'a> {
    core: RwLockReadGuard<'a, core::Repository>,
    _lock: core::RepoLock,
}

impl std::ops::Deref for View<'_> {
    type Target = core::Repository;

    fn deref(&self) -> &core::Repository {
        &self.core
    }
}
