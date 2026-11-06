//! Bringing a tar, zip or 7z archive in as a snapshot (CLI.md §4.2).
//!
//! An archive may come from anyone. Nothing here writes to a folder, and a
//! name that climbs out of the archive is left out with a warning, so a tree
//! never holds one. Content is streamed, never held in memory.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use apeiron_core::{
    EntryKind, ObjectId, ObjectWriter, Repository, Snapshot, Time, Tree, TreeEntry,
};
use serde::{Deserialize, Serialize};

use crate::backup::{CancelMarker, is_cancel};
use crate::control::{Control, Phase, Warning, WarningKind};
use crate::error::{Error, IoContext, Result};
use crate::name::Name;
use crate::os;

/// Paths deeper than this are left out, so that a hostile archive cannot
/// exhaust the stack.
const MAX_DEPTH: usize = 256;
/// The most bytes read as the target of a link in a zip file.
const MAX_LINK: u64 = 64 * 1024;
const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;

/// Archive formats `import` can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportFormat {
    #[serde(rename = "tar")]
    Tar,
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "zip")]
    Zip,
    #[serde(rename = "7z")]
    SevenZ,
}

impl ImportFormat {
    /// The format named by a file name (`.tar`, `.tar.gz`, `.tgz`, `.zip`,
    /// `.7z`).
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(ImportFormat::TarGz)
        } else if name.ends_with(".tar") {
            Some(ImportFormat::Tar)
        } else if name.ends_with(".zip") {
            Some(ImportFormat::Zip)
        } else if name.ends_with(".7z") {
            Some(ImportFormat::SevenZ)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportOptions {
    pub archive: PathBuf,
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// From the file name when absent.
    #[serde(default)]
    pub format: Option<ImportFormat>,
    /// `import <file name>` when absent.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
}

impl ImportOptions {
    pub fn new(archive: impl Into<PathBuf>, ref_name: impl Into<String>) -> Self {
        ImportOptions {
            archive: archive.into(),
            ref_name: ref_name.into(),
            format: None,
            message: None,
            author: None,
            host: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub snapshot: ObjectId,
    pub parent: Option<ObjectId>,
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    pub bytes: u64,
    /// File content bytes that were not already in the repository.
    pub new_bytes: u64,
    /// Entries left out, with the reason.
    pub warnings: Vec<Warning>,
}

/// What one archive entry is, once its content is stored.
#[derive(Clone)]
struct Item {
    kind: EntryKind,
    mode: Option<u16>,
    mtime: Option<Time>,
    uid: Option<u32>,
    gid: Option<u32>,
}

enum Node {
    Item(Item),
    Dir(Dir),
}

/// A folder being put together; names sort as a tree needs them.
#[derive(Default)]
struct Dir {
    entries: BTreeMap<Vec<u8>, Node>,
    mode: Option<u16>,
    mtime: Option<Time>,
    uid: Option<u32>,
    gid: Option<u32>,
}

struct Import<'a, 'w> {
    writer: &'a mut ObjectWriter<'w>,
    ctl: &'a Control,
    root: Dir,
    entries: u64,
    bytes: u64,
    warnings: Vec<Warning>,
}

/// The parts of a path in an archive; `None` if it climbs out of the archive,
/// is too deep or holds a name no tree can hold.
fn split_path(path: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut parts = Vec::new();
    for part in path.split(|&b| b == b'/' || b == b'\\') {
        match part {
            b"" | b"." => {}
            b".." => return None,
            part if part.contains(&0) => return None,
            part => parts.push(part.to_vec()),
        }
    }
    (parts.len() <= MAX_DEPTH).then_some(parts)
}

impl Import<'_, '_> {
    fn warn(&mut self, path: &[u8], kind: WarningKind, message: impl ToString) {
        self.warnings.push(Warning {
            path: Name::default().join(path),
            kind,
            message: message.to_string(),
        });
    }

    fn check_path(&mut self, path: &[u8]) -> Option<Vec<Vec<u8>>> {
        let parts = split_path(path);
        if parts.is_none() {
            self.warn(
                path,
                WarningKind::Unsupported,
                "the path leaves the archive, is too deep or cannot be a name",
            );
        }
        parts
    }

    /// The folder at `parts`, made where missing. A file in the way gives
    /// way, as a later entry of an archive overrides an earlier one.
    fn dir_at(&mut self, parts: &[Vec<u8>]) -> &mut Dir {
        let mut dir = &mut self.root;
        for part in parts {
            let node = dir
                .entries
                .entry(part.clone())
                .and_modify(|n| {
                    if matches!(n, Node::Item(_)) {
                        *n = Node::Dir(Dir::default());
                    }
                })
                .or_insert_with(|| Node::Dir(Dir::default()));
            let Node::Dir(next) = node else {
                unreachable!("a file was just replaced by a folder");
            };
            dir = next;
        }
        dir
    }

    fn add_dir(&mut self, parts: &[Vec<u8>], attrs: Attrs) {
        self.entries += 1;
        let dir = self.dir_at(parts);
        dir.mode = attrs.mode;
        dir.mtime = attrs.mtime;
        dir.uid = attrs.uid;
        dir.gid = attrs.gid;
    }

    fn add_item(&mut self, parts: &[Vec<u8>], item: Item) {
        self.entries += 1;
        if let Some((name, folder)) = parts.split_last() {
            self.dir_at(folder)
                .entries
                .insert(name.clone(), Node::Item(item));
        }
    }

    /// Stores a file's content as it is read.
    fn store(&mut self, reader: impl Read) -> Result<EntryKind> {
        let mut reader = Progress {
            inner: reader,
            ctl: self.ctl,
        };
        let (size, blob) = match self.writer.put_file(&mut reader) {
            Ok(stored) => stored,
            Err(apeiron_core::Error::Input(e)) if is_cancel(&e) => return Err(Error::Cancelled),
            Err(apeiron_core::Error::Input(e)) => {
                return Err(Error::InvalidInput(format!("the archive is damaged: {e}")));
            }
            Err(e) => return Err(e.into()),
        };
        self.bytes += size;
        self.ctl.update(|p| p.items_done += 1);
        Ok(EntryKind::File { size, blob })
    }

    fn lookup(&self, parts: &[Vec<u8>]) -> Option<&Node> {
        let (name, folder) = parts.split_last()?;
        let mut dir = &self.root;
        for part in folder {
            match dir.entries.get(part)? {
                Node::Dir(next) => dir = next,
                Node::Item(_) => return None,
            }
        }
        dir.entries.get(name)
    }

    /// Writes the folders, the deepest first; returns the counts.
    fn write_dir(
        writer: &mut ObjectWriter<'_>,
        dir: Dir,
        counts: &mut (u64, u64, u64),
    ) -> Result<ObjectId> {
        let mut entries = Vec::with_capacity(dir.entries.len());
        for (name, node) in dir.entries {
            let entry = match node {
                Node::Item(item) => {
                    match item.kind {
                        EntryKind::Symlink { .. } => counts.2 += 1,
                        _ => counts.0 += 1,
                    }
                    TreeEntry {
                        name,
                        kind: item.kind,
                        mode: item.mode,
                        mtime: item.mtime,
                        uid: item.uid,
                        gid: item.gid,
                        xattrs: Vec::new(),
                    }
                }
                Node::Dir(sub) => {
                    counts.1 += 1;
                    let (mode, mtime, uid, gid) = (sub.mode, sub.mtime, sub.uid, sub.gid);
                    TreeEntry {
                        name,
                        kind: EntryKind::Directory {
                            tree: Self::write_dir(writer, sub, counts)?,
                        },
                        mode,
                        mtime,
                        uid,
                        gid,
                        xattrs: Vec::new(),
                    }
                }
            };
            entries.push(entry);
        }
        Ok(writer.put_tree(&Tree { entries })?)
    }
}

#[derive(Clone, Copy, Default)]
struct Attrs {
    mode: Option<u16>,
    mtime: Option<Time>,
    uid: Option<u32>,
    gid: Option<u32>,
}

/// Reads while reporting progress and checking for cancellation.
struct Progress<'c, R> {
    inner: R,
    ctl: &'c Control,
}

impl<R: Read> Read for Progress<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.ctl.is_cancelled() {
            return Err(io::Error::other(CancelMarker));
        }
        let n = self.inner.read(buf)?;
        self.ctl.update(|p| p.bytes_done += n as u64);
        Ok(n)
    }
}

fn damaged(e: impl std::fmt::Display) -> Error {
    Error::InvalidInput(format!("the archive is damaged: {e}"))
}

fn read_tar(import: &mut Import<'_, '_>, reader: impl Read) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(damaged)? {
        import.ctl.check()?;
        let mut entry = entry.map_err(damaged)?;
        let path = entry.path_bytes().into_owned();
        let Some(parts) = import.check_path(&path) else {
            continue;
        };
        let header = entry.header();
        let attrs = Attrs {
            mode: header.mode().ok().map(|m| (m & 0o7777) as u16),
            mtime: header
                .mtime()
                .ok()
                .and_then(|s| i64::try_from(s).ok())
                .map(|seconds| Time { seconds, nanos: 0 }),
            uid: header.uid().ok().and_then(|u| u32::try_from(u).ok()),
            gid: header.gid().ok().and_then(|g| u32::try_from(g).ok()),
        };
        let kind = header.entry_type();
        let link = entry.link_name_bytes().map(|l| l.into_owned());
        if kind.is_dir() {
            import.add_dir(&parts, attrs);
        } else if kind.is_file() || kind.is_gnu_sparse() || kind.is_contiguous() {
            if parts.is_empty() {
                continue;
            }
            let stored = import.store(&mut entry)?;
            import.add_item(&parts, item(stored, attrs));
        } else if kind.is_symlink() {
            match link {
                Some(target) if !parts.is_empty() && !target.is_empty() => {
                    let stored = EntryKind::Symlink {
                        target: target.to_vec(),
                    };
                    import.add_item(&parts, item(stored, attrs));
                }
                _ => import.warn(&path, WarningKind::Unsupported, "a link without a target"),
            }
        } else if kind.is_hard_link() {
            // The link's file is stored once; both paths share its content.
            let source = link.as_deref().and_then(split_path);
            let found = source.as_deref().and_then(|s| import.lookup(s));
            match found {
                Some(Node::Item(file)) if !parts.is_empty() => {
                    let copy = file.clone();
                    if let EntryKind::File { size, .. } = copy.kind {
                        import.bytes += size;
                    }
                    import.add_item(&parts, copy);
                }
                _ => import.warn(
                    &path,
                    WarningKind::Unsupported,
                    "a hard link to a file that comes later or not at all",
                ),
            }
        } else {
            import.warn(
                &path,
                WarningKind::Unsupported,
                "not a file, folder or link",
            );
        }
    }
    Ok(())
}

fn item(kind: EntryKind, attrs: Attrs) -> Item {
    Item {
        kind,
        mode: attrs.mode,
        mtime: attrs.mtime,
        uid: attrs.uid,
        gid: attrs.gid,
    }
}

/// Seconds since 1970 of a zip time, taken as UTC (Howard Hinnant's
/// days-from-civil algorithm).
fn zip_time(t: zip::DateTime) -> Time {
    let (y, m, d) = (
        i64::from(t.year()),
        i64::from(t.month()),
        i64::from(t.day()),
    );
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Time {
        seconds: days * 86_400
            + i64::from(t.hour()) * 3600
            + i64::from(t.minute()) * 60
            + i64::from(t.second()),
        nanos: 0,
    }
}

fn read_zip(import: &mut Import<'_, '_>, file: File) -> Result<()> {
    let mut archive = zip::ZipArchive::new(file).map_err(damaged)?;
    for i in 0..archive.len() {
        import.ctl.check()?;
        let mut entry = match archive.by_index(i) {
            Ok(entry) => entry,
            // A password-protected entry can be left out; damage cannot.
            Err(e @ zip::result::ZipError::UnsupportedArchive(_)) => {
                let name = format!("entry {i}");
                import.warn(name.as_bytes(), WarningKind::Unreadable, e);
                continue;
            }
            Err(e) => return Err(damaged(e)),
        };
        let path = entry.name_raw().to_vec();
        let Some(parts) = import.check_path(&path) else {
            continue;
        };
        let unix = entry.unix_mode();
        let attrs = Attrs {
            mode: unix.map(|m| (m & 0o7777) as u16),
            mtime: entry.last_modified().map(zip_time),
            uid: None,
            gid: None,
        };
        if entry.is_dir() {
            import.add_dir(&parts, attrs);
        } else if parts.is_empty() {
            continue;
        } else if unix.is_some_and(|m| m & S_IFMT == S_IFLNK) {
            let mut target = Vec::new();
            (&mut entry)
                .take(MAX_LINK)
                .read_to_end(&mut target)
                .map_err(damaged)?;
            if target.is_empty() {
                import.warn(&path, WarningKind::Unsupported, "a link without a target");
                continue;
            }
            import.add_item(&parts, item(EntryKind::Symlink { target }, attrs));
        } else {
            let stored = import.store(&mut entry)?;
            import.add_item(&parts, item(stored, attrs));
        }
    }
    Ok(())
}

fn read_7z(import: &mut Import<'_, '_>, file: File) -> Result<()> {
    let mut archive =
        sevenz_rust2::ArchiveReader::new(file, sevenz_rust2::Password::empty()).map_err(damaged)?;
    // The reader's own errors cannot carry ours, so an error stops it here.
    let mut failure = None;
    let read = archive.for_each_entries(|entry, content| {
        let done = read_7z_entry(import, entry, content);
        let more = done.is_ok();
        failure = done.err();
        Ok(more)
    });
    match failure {
        Some(e) => Err(e),
        None => read.map_err(damaged),
    }
}

fn read_7z_entry(
    import: &mut Import<'_, '_>,
    entry: &sevenz_rust2::ArchiveEntry,
    content: &mut dyn Read,
) -> Result<()> {
    const UNIX_EXTENSION: u32 = 0x8000;
    import.ctl.check()?;
    let path = entry.name.as_bytes();
    let Some(parts) = import.check_path(path) else {
        return Ok(());
    };
    let unix = (entry.has_windows_attributes && entry.windows_attributes & UNIX_EXTENSION != 0)
        .then_some(entry.windows_attributes >> 16);
    let attrs = Attrs {
        mode: unix.map(|m| (m & 0o7777) as u16),
        mtime: entry
            .has_last_modified_date
            .then(|| os::to_time(SystemTime::from(entry.last_modified_date))),
        uid: None,
        gid: None,
    };
    // The reader marks every folder as an entry that removes one, so folders
    // come first; at worst a real one leaves an empty folder.
    if entry.is_directory {
        import.add_dir(&parts, attrs);
    } else if entry.is_anti_item {
        import.warn(
            path,
            WarningKind::Unsupported,
            "an entry that removes a file",
        );
    } else if parts.is_empty() {
        // Nothing to name it by.
    } else if unix.is_some_and(|m| m & S_IFMT == S_IFLNK) {
        let mut target = Vec::new();
        content
            .take(MAX_LINK)
            .read_to_end(&mut target)
            .map_err(damaged)?;
        if target.is_empty() {
            import.warn(path, WarningKind::Unsupported, "a link without a target");
        } else {
            import.add_item(&parts, item(EntryKind::Symlink { target }, attrs));
        }
    } else {
        let stored = import.store(content)?;
        import.add_item(&parts, item(stored, attrs));
    }
    Ok(())
}

/// Stores the archive as a new snapshot and moves the ref to it. If it is
/// damaged, or the operation fails or is cancelled, the ref is not moved.
pub fn import(
    repo: &mut Repository,
    options: &ImportOptions,
    ctl: &Control,
) -> Result<ImportOutcome> {
    apeiron_core::validate_ref_name(&options.ref_name)?;
    let format = options
        .format
        .or_else(|| ImportFormat::from_path(&options.archive))
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "cannot tell the format of {}; name it with --format",
                options.archive.display()
            ))
        })?;
    let file = File::open(&options.archive).at(&options.archive)?;
    let parent = repo.read_ref(&options.ref_name)?;

    ctl.set_phase(Phase::Storing);
    let mut writer = repo.writer();
    let mut state = Import {
        writer: &mut writer,
        ctl,
        root: Dir::default(),
        entries: 0,
        bytes: 0,
        warnings: Vec::new(),
    };
    match format {
        ImportFormat::Tar => read_tar(&mut state, file)?,
        ImportFormat::TarGz => read_tar(&mut state, flate2::read::GzDecoder::new(file))?,
        ImportFormat::Zip => read_zip(&mut state, file)?,
        ImportFormat::SevenZ => read_7z(&mut state, file)?,
    }
    if state.entries == 0 {
        return Err(Error::InvalidInput(format!(
            "{} holds nothing that can be stored",
            options.archive.display()
        )));
    }
    let (root, bytes, warnings) = (state.root, state.bytes, state.warnings);

    let mut counts = (0, 0, 0);
    let tree = Import::write_dir(&mut writer, root, &mut counts)?;
    let message = options.message.clone().or_else(|| {
        options
            .archive
            .file_name()
            .map(|n| format!("import {}", n.to_string_lossy()))
    });
    let snapshot = Snapshot {
        tree,
        parents: parent.into_iter().collect(),
        time: os::to_time(SystemTime::now()),
        author: options.author.clone(),
        host: options.host.clone(),
        message,
        meta: Vec::new(),
    };
    let id = writer.put_snapshot(&snapshot)?;
    let new_bytes = writer.new_bytes();
    ctl.check()?;
    ctl.set_phase(Phase::Finishing);
    writer.finish()?;
    repo.update_ref(&options.ref_name, parent, Some(id))?;
    ctl.set_phase(Phase::Done);
    Ok(ImportOutcome {
        snapshot: id,
        parent,
        files: counts.0,
        dirs: counts.1,
        symlinks: counts.2,
        bytes,
        new_bytes,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_cleaned_and_those_that_climb_out_refused() {
        let p = |s: &str| split_path(s.as_bytes());
        assert_eq!(
            p("a/./b//c/"),
            Some(vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()])
        );
        assert_eq!(
            p("/etc/passwd"),
            Some(vec![b"etc".to_vec(), b"passwd".to_vec()])
        );
        assert_eq!(p("a\\b"), Some(vec![b"a".to_vec(), b"b".to_vec()]));
        assert_eq!(p("../evil"), None);
        assert_eq!(p("a/../../evil"), None);
        assert_eq!(p("a\\..\\evil"), None);
        assert_eq!(split_path(b"a\0b"), None);
        assert_eq!(p(&"a/".repeat(MAX_DEPTH + 1)), None);
        assert_eq!(p("./"), Some(Vec::new()));
    }

    #[test]
    fn formats_come_from_the_name() {
        let f = |s: &str| ImportFormat::from_path(Path::new(s));
        assert_eq!(f("a.tar"), Some(ImportFormat::Tar));
        assert_eq!(f("a.TAR.GZ"), Some(ImportFormat::TarGz));
        assert_eq!(f("a.tgz"), Some(ImportFormat::TarGz));
        assert_eq!(f("a.zip"), Some(ImportFormat::Zip));
        assert_eq!(f("a.7Z"), Some(ImportFormat::SevenZ));
        assert_eq!(f("a.rar"), None);
    }

    #[test]
    fn zip_times_are_seconds_since_1970() {
        let t = zip::DateTime::from_date_and_time(2000, 3, 1, 12, 30, 2).unwrap();
        assert_eq!(zip_time(t).seconds, 951_913_802);
    }
}
