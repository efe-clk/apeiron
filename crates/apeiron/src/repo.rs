//! The repository handle (API.md §8.2, §8.3).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use apeiron_core as core;
use apeiron_services as services;

use crate::error::Result;
use crate::job::Job;
use crate::spec;
use crate::types::{
    DeleteRefRequest, DiffRequest, ListRequest, LogPage, LogRequest, RefInfo, RepoInfo,
    RestoreRequest, SnapshotInfo,
};
use crate::{
    BackupReport, BackupRequest, Change, EntryInfo, ObjectId, ReflogEntry, RepairReport,
    RestoreReport, VerifyReport,
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
        let chunker = self.read().chunker();
        RepoInfo {
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
        spec::resolve(&self.read(), spec)
    }

    /// Stores a folder as a new snapshot and moves the ref to it.
    pub fn backup(&self, request: BackupRequest) -> Result<Job<BackupReport>> {
        let repo = self.clone();
        Job::spawn("backup", move |ctl| {
            let _writer = repo.lock_writer();
            let mut core = core::Repository::open(repo.path())?;
            let report = services::backup(&mut core, &request, ctl)?;
            *repo.write() = core;
            Ok(report)
        })
    }

    /// Writes a snapshot, or one path in it, into a new or empty folder.
    pub fn restore(&self, request: RestoreRequest) -> Result<Job<RestoreReport>> {
        let snapshot = self.resolve(&request.snapshot)?;
        let repo = self.clone();
        Job::spawn("restore", move |ctl| {
            let core = repo.read();
            Ok(services::restore(
                &core,
                &snapshot,
                request.path.as_bytes(),
                &request.target,
                ctl,
            )?)
        })
    }

    /// Reads and checks every pack, index and object.
    pub fn verify(&self) -> Result<Job<VerifyReport>> {
        let repo = self.clone();
        Job::spawn("verify", move |ctl| {
            ctl.update(|p| p.phase = crate::Phase::Verifying);
            let report = repo.read().verify()?;
            ctl.update(|p| p.phase = crate::Phase::Done);
            Ok(report)
        })
    }

    /// One page of history, newest first.
    pub fn log(&self, request: &LogRequest) -> Result<LogPage> {
        let core = self.read();
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
        let core = self.read();
        let snapshot = spec::resolve(&core, &request.snapshot)?;
        Ok(services::list(&core, &snapshot, request.path.as_bytes())?)
    }

    /// Paths added, removed and modified between two snapshots.
    pub fn diff(&self, request: &DiffRequest) -> Result<Vec<Change>> {
        let core = self.read();
        let from = spec::resolve(&core, &request.from)?;
        let to = spec::resolve(&core, &request.to)?;
        Ok(services::diff(&core, &from, &to)?)
    }

    pub fn refs(&self) -> Result<Vec<RefInfo>> {
        Ok(self
            .read()
            .list_refs()?
            .into_iter()
            .map(|(name, snapshot)| RefInfo { name, snapshot })
            .collect())
    }

    pub fn reflog(&self, name: &str) -> Result<Vec<ReflogEntry>> {
        Ok(self.read().reflog(name)?)
    }

    /// Deletes a ref if it still points to the expected snapshot. The
    /// snapshots stay until garbage collection removes them.
    pub fn delete_ref(&self, request: &DeleteRefRequest) -> Result<()> {
        let _writer = self.lock_writer();
        Ok(self
            .read()
            .update_ref(&request.name, Some(request.expected), None)?)
    }

    /// Removes stale ref locks and adds reflog lines lost in a crash.
    pub fn repair(&self) -> Result<RepairReport> {
        let _writer = self.lock_writer();
        Ok(self.read().repair()?)
    }
}
