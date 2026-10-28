//! Storing a folder as a snapshot.

use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use apeiron_core::{EntryKind, ObjectId, ObjectWriter, Repository, Snapshot, Tree, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::control::{Control, Phase, Warning, WarningKind};
use crate::error::{Error, IoContext, Result};
use crate::name::Name;
use crate::os;

/// What to back up and how to describe the snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupOptions {
    pub source: PathBuf,
    #[serde(rename = "ref")]
    pub ref_name: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    /// Free key-value pairs; sorted and checked when the snapshot is stored.
    #[serde(default)]
    pub meta: Vec<(String, String)>,
}

impl BackupOptions {
    pub fn new(source: impl Into<PathBuf>, ref_name: impl Into<String>) -> Self {
        BackupOptions {
            source: source.into(),
            ref_name: ref_name.into(),
            message: None,
            author: None,
            host: None,
            meta: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupOutcome {
    pub snapshot: ObjectId,
    /// The snapshot the ref pointed to before, now the first parent.
    pub parent: Option<ObjectId>,
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    /// Size of all files read.
    pub bytes: u64,
    /// Bytes that were not already in the repository.
    pub new_bytes: u64,
    pub warnings: Vec<Warning>,
}

/// Stores `options.source` as a new snapshot and moves the ref to it.
///
/// Unreadable entries become warnings. If the operation fails or is
/// cancelled, the ref is not moved.
pub fn backup(
    repo: &mut Repository,
    options: &BackupOptions,
    ctl: &Control,
) -> Result<BackupOutcome> {
    apeiron_core::validate_ref_name(&options.ref_name)?;
    let source = &options.source;
    if !fs::metadata(source).at(source)?.is_dir() {
        return Err(Error::InvalidInput(format!(
            "{} is not a folder",
            source.display()
        )));
    }
    // Fail early when the root itself cannot be listed.
    fs::read_dir(source).at(source)?;
    let parent = repo.read_ref(&options.ref_name)?;

    ctl.set_phase(Phase::Scanning);
    let mut totals = (0, 0);
    scan(source, ctl, &mut totals)?;
    ctl.update(|p| {
        p.phase = Phase::Storing;
        p.items_total = Some(totals.0);
        p.bytes_total = Some(totals.1);
    });

    let mut walk = Walk {
        ctl,
        warnings: Vec::new(),
        files: 0,
        dirs: 0,
        symlinks: 0,
        bytes: 0,
    };
    let mut writer = repo.writer();
    let tree = walk
        .store_dir(&mut writer, source, &Name::default())?
        .ok_or_else(|| Error::InvalidInput(format!("{} cannot be read", source.display())))?;
    let mut meta = options.meta.clone();
    meta.sort();
    let snapshot = Snapshot {
        tree,
        parents: parent.into_iter().collect(),
        time: os::to_time(SystemTime::now()),
        author: options.author.clone(),
        host: options.host.clone(),
        message: options.message.clone(),
        meta,
    };
    let id = writer.put_snapshot(&snapshot)?;
    let new_bytes = writer.new_bytes();

    ctl.check()?;
    ctl.set_phase(Phase::Finishing);
    writer.finish()?;
    repo.update_ref(&options.ref_name, parent, Some(id))?;
    ctl.set_phase(Phase::Done);
    Ok(BackupOutcome {
        snapshot: id,
        parent,
        files: walk.files,
        dirs: walk.dirs,
        symlinks: walk.symlinks,
        bytes: walk.bytes,
        new_bytes,
        warnings: walk.warnings,
    })
}

/// Counts regular files and their bytes below `dir`; errors are ignored
/// here and reported when the entry is stored.
fn scan(dir: &Path, ctl: &Control, totals: &mut (u64, u64)) -> Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        ctl.check()?;
        let Ok(md) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if md.is_file() {
            totals.0 += 1;
            totals.1 += md.len();
        } else if md.is_dir() {
            scan(&entry.path(), ctl, totals)?;
        }
    }
    Ok(())
}

struct Walk<'c> {
    ctl: &'c Control,
    warnings: Vec<Warning>,
    files: u64,
    dirs: u64,
    symlinks: u64,
    bytes: u64,
}

impl Walk<'_> {
    fn warn(&mut self, path: &Name, kind: WarningKind, message: impl ToString) {
        self.warnings.push(Warning {
            path: path.clone(),
            kind,
            message: message.to_string(),
        });
    }

    /// Stores a folder; `None` if it cannot be listed.
    fn store_dir(
        &mut self,
        writer: &mut ObjectWriter<'_>,
        dir: &Path,
        rel: &Name,
    ) -> Result<Option<ObjectId>> {
        let listing = match fs::read_dir(dir) {
            Ok(listing) => listing,
            Err(e) => {
                self.warn(rel, WarningKind::Unreadable, e);
                return Ok(None);
            }
        };
        let mut children = Vec::new();
        for entry in listing {
            match entry {
                Ok(entry) => match os::name_bytes(&entry.file_name()) {
                    Some(name) => children.push((name, entry.path())),
                    None => self.warn(
                        &rel.join(entry.file_name().to_string_lossy().as_bytes()),
                        WarningKind::Unsupported,
                        "the name cannot be stored",
                    ),
                },
                Err(e) => self.warn(rel, WarningKind::Unreadable, e),
            }
        }
        children.sort();

        let mut entries = Vec::with_capacity(children.len());
        for (name, path) in children {
            self.ctl.check()?;
            let child = rel.join(&name);
            let md = match fs::symlink_metadata(&path) {
                Ok(md) => md,
                Err(e) => {
                    self.warn(&child, WarningKind::Unreadable, e);
                    continue;
                }
            };
            let kind = if md.is_file() {
                self.store_file(writer, &path, &child, &md)?
            } else if md.is_dir() {
                let tree = self.store_dir(writer, &path, &child)?;
                tree.map(|tree| EntryKind::Directory { tree })
            } else if md.file_type().is_symlink() {
                self.store_symlink(&path, &child)
            } else {
                self.warn(
                    &child,
                    WarningKind::Unsupported,
                    "not a file, folder or symbolic link",
                );
                None
            };
            let Some(kind) = kind else {
                continue;
            };
            let attrs = os::attrs(&md);
            entries.push(TreeEntry {
                name,
                kind,
                mode: attrs.mode,
                mtime: attrs.mtime,
                uid: attrs.uid,
                gid: attrs.gid,
                xattrs: Vec::new(),
            });
        }
        self.dirs += 1;
        Ok(Some(writer.put_tree(&Tree { entries })?))
    }

    fn store_file(
        &mut self,
        writer: &mut ObjectWriter<'_>,
        path: &Path,
        rel: &Name,
        before: &Metadata,
    ) -> Result<Option<EntryKind>> {
        self.ctl.update(|p| p.current = Some(rel.clone()));
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) => {
                self.warn(rel, WarningKind::Unreadable, e);
                return Ok(None);
            }
        };
        let mut reader = Reader {
            file,
            ctl: self.ctl,
        };
        let (size, blob) = match writer.put_file(&mut reader) {
            Ok(stored) => stored,
            Err(apeiron_core::Error::Input(e)) if is_cancel(&e) => return Err(Error::Cancelled),
            Err(apeiron_core::Error::Input(e)) => {
                self.warn(rel, WarningKind::Unreadable, e);
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        let after = reader.file.metadata().ok();
        let changed = after.is_none_or(|after| {
            after.len() != before.len()
                || after.modified().ok() != before.modified().ok()
                || after.len() != size
        });
        if changed {
            self.warn(
                rel,
                WarningKind::ChangedWhileReading,
                "the file changed while it was read; it was stored as read",
            );
        }
        self.files += 1;
        self.bytes += size;
        self.ctl.update(|p| p.items_done += 1);
        Ok(Some(EntryKind::File { size, blob }))
    }

    fn store_symlink(&mut self, path: &Path, rel: &Name) -> Option<EntryKind> {
        match os::link_target(path) {
            Ok(Some(target)) => {
                self.symlinks += 1;
                Some(EntryKind::Symlink { target })
            }
            Ok(None) => {
                self.warn(
                    rel,
                    WarningKind::Unsupported,
                    "the link target cannot be stored",
                );
                None
            }
            Err(e) => {
                self.warn(rel, WarningKind::Unreadable, e);
                None
            }
        }
    }
}

/// Marks the I/O error a [`Reader`] returns when the operation is cancelled.
#[derive(Debug)]
struct CancelMarker;

impl std::fmt::Display for CancelMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for CancelMarker {}

fn is_cancel(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<CancelMarker>())
}

/// Reads a file while reporting progress and checking for cancellation.
struct Reader<'c> {
    file: File,
    ctl: &'c Control,
}

impl Read for Reader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.ctl.is_cancelled() {
            return Err(io::Error::other(CancelMarker));
        }
        let n = self.file.read(buf)?;
        self.ctl.update(|p| p.bytes_done += n as u64);
        Ok(n)
    }
}
