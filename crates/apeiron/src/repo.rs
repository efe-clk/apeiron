//! The repository handle (API.md §8.2, §8.3).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use apeiron_core as core;
use apeiron_services as services;

use crate::error::{Error, ErrorKind, Result};
use crate::job::Job;
use crate::spec;
use crate::types::{
    AddedKey, DeleteRefRequest, DiffRequest, ExportRequest, ListRequest, LogPage, LogRequest,
    RefInfo, RepoInfo, RestoreRequest, SnapshotInfo, TransferRequest,
};
use crate::{
    ArchiveFormat, BackupReport, BackupRequest, Change, EntryInfo, ExportReport, GcReport,
    GcRequest, KeyInfo, ObjectId, Phase, ReflogEntry, Remote, RepairReport, RestoreReport,
    TransferReport, VerifyReport,
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

    /// The remotes of this copy, sorted by name.
    pub fn remotes(&self) -> Result<Vec<Remote>> {
        Ok(self.read().remotes()?)
    }

    /// Adds a remote. A folder may be given relative to the current folder;
    /// it is stored as an absolute path.
    pub fn add_remote(&self, name: &str, url: &str) -> Result<Remote> {
        let url = if url.contains("://") {
            url.to_owned()
        } else {
            let path = std::path::absolute(url)
                .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("{url}: {e}")))?;
            path.to_str()
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} is not valid UTF-8", path.display()),
                    )
                })?
                .to_owned()
        };
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
            let store = open_store(&remote, repo.info().encrypted)?;
            let report = repo
                .view()?
                .push(&*store, &request.ref_name, &mut |r| copied(ctl, r))?;
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
            let store = open_store(&remote, repo.info().encrypted)?;
            let _writer = repo.lock_writer();
            let _lock = core::RepoLock::shared(repo.path())?;
            let mut core = repo.read().reopen()?;
            let report = core.pull(&*store, &request.ref_name, &mut |r| copied(ctl, r))?;
            *repo.write() = core;
            ctl.update(|p| p.phase = Phase::Done);
            Ok(report)
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
}

/// Connects to a remote. Only an encrypted repository is sent over the
/// network (FORMAT.md §15.1).
fn open_store(remote: &Remote, encrypted: bool) -> Result<Box<dyn core::Store>> {
    match core::RemoteUrl::parse(&remote.url)? {
        core::RemoteUrl::Folder(path) => Ok(Box::new(core::FolderStore::open(&path)?)),
        core::RemoteUrl::Sftp(url) => {
            if !encrypted {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "only an encrypted repository can use an SFTP remote; \
                     create one with `init --encrypt`",
                ));
            }
            let options = apeiron_remote::SshOptions::from_home()?;
            Ok(Box::new(apeiron_remote::SftpStore::connect(
                &url, &options,
            )?))
        }
    }
}

/// Reports the progress of a push or pull and stops it once cancelled.
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
