//! Writing a snapshot into a tar, zip or 7z archive.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use apeiron_core::{EntryKind, ObjectId, Repository, Time, Tree, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::control::{Control, Phase, Warning, WarningKind};
use crate::error::{Error, IoContext, Result};
use crate::list::{Node, lookup};
use crate::name::Name;
use crate::os;
use crate::restore::{count_entry, count_tree};

/// Archive formats `export` can write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveFormat {
    #[serde(rename = "tar")]
    Tar,
    #[serde(rename = "zip")]
    Zip,
    #[serde(rename = "7z")]
    SevenZ,
}

impl ArchiveFormat {
    /// The format named by a file extension (`.tar`, `.zip`, `.7z`).
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "tar" => Some(ArchiveFormat::Tar),
            "zip" => Some(ArchiveFormat::Zip),
            "7z" => Some(ArchiveFormat::SevenZ),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportOutcome {
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    pub bytes: u64,
    pub warnings: Vec<Warning>,
}

/// Writes `path` of `snapshot` (everything for an empty path) into a new
/// archive file `output`. An existing file is never overwritten. If the
/// operation fails or is cancelled, the partial archive is removed.
pub fn export(
    repo: &Repository,
    snapshot: &ObjectId,
    path: &[u8],
    output: &Path,
    format: ArchiveFormat,
    ctl: &Control,
) -> Result<ExportOutcome> {
    let root = repo.read_snapshot(snapshot)?.tree;
    let node = lookup(repo, &root, path)?;

    ctl.set_phase(Phase::Scanning);
    let mut totals = (0, 0);
    match &node {
        Node::Tree(tree) => count_tree(repo, tree, ctl, &mut totals)?,
        Node::Entry(entry) => count_entry(repo, entry, ctl, &mut totals)?,
    }
    ctl.update(|p| {
        p.phase = Phase::Exporting;
        p.items_total = Some(totals.0);
        p.bytes_total = Some(totals.1);
    });

    let file = match File::create_new(output) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(Error::AlreadyExists(output.to_path_buf()));
        }
        Err(e) => return Err(e).at(output),
    };
    let result = write_archive(repo, &node, file, output, format, ctl);
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    let out = result?;
    ctl.set_phase(Phase::Done);
    Ok(out)
}

fn write_archive(
    repo: &Repository,
    node: &Node,
    file: File,
    output: &Path,
    format: ArchiveFormat,
    ctl: &Control,
) -> Result<ExportOutcome> {
    let sink: Box<dyn Sink> = match format {
        ArchiveFormat::Tar => Box::new(TarSink(tar::Builder::new(file))),
        ArchiveFormat::Zip => Box::new(ZipSink(zip::ZipWriter::new(file))),
        ArchiveFormat::SevenZ => Box::new(SevenZSink(
            sevenz_rust2::ArchiveWriter::new(file).map_err(|e| archive_error(output, e))?,
        )),
    };
    let mut run = Run {
        repo,
        ctl,
        output: output.to_path_buf(),
        sink,
        out: ExportOutcome::default(),
    };
    match node {
        Node::Tree(tree) => run.tree(tree, &Name::default())?,
        Node::Entry(entry) => run.entry(entry, &Name::default())?,
    }
    let Run { sink, out, .. } = run;
    sink.finish().at(output)?;
    Ok(out)
}

fn archive_error(output: &Path, e: impl std::fmt::Display) -> Error {
    Error::Io {
        path: output.to_path_buf(),
        source: io::Error::other(e.to_string()),
    }
}

/// Facts about one archive member.
struct Member<'a> {
    /// `/`-separated path inside the archive.
    name: &'a Name,
    mode: Option<u16>,
    mtime: Option<Time>,
    uid: Option<u32>,
    gid: Option<u32>,
}

/// One archive format.
trait Sink {
    fn dir(&mut self, m: &Member<'_>) -> io::Result<()>;
    fn file(&mut self, m: &Member<'_>, size: u64, content: &mut dyn Read) -> io::Result<()>;
    fn symlink(&mut self, m: &Member<'_>, target: &[u8]) -> io::Result<()>;
    /// Completes the archive and makes it durable.
    fn finish(self: Box<Self>) -> io::Result<()>;
}

struct Run<'a> {
    repo: &'a Repository,
    ctl: &'a Control,
    output: PathBuf,
    sink: Box<dyn Sink>,
    out: ExportOutcome,
}

impl Run<'_> {
    fn tree(&mut self, tree: &Tree, rel: &Name) -> Result<()> {
        for entry in &tree.entries {
            self.entry(entry, rel)?;
        }
        Ok(())
    }

    fn entry(&mut self, entry: &TreeEntry, rel: &Name) -> Result<()> {
        self.ctl.check()?;
        let name = rel.join(&entry.name);
        let member = Member {
            name: &name,
            mode: entry.mode,
            mtime: entry.mtime,
            uid: entry.uid,
            gid: entry.gid,
        };
        match &entry.kind {
            EntryKind::Directory { tree } => {
                let written = self.sink.dir(&member);
                if self.written(&name, written)? {
                    self.out.dirs += 1;
                }
                self.tree(&self.repo.read_tree(tree)?, &name)?;
            }
            EntryKind::File { size, blob } => {
                self.ctl.update(|p| p.current = Some(name.clone()));
                let chunks = match blob {
                    Some(blob) => self.repo.read_blob(blob)?.chunks,
                    None => Vec::new(),
                };
                let mut reader = BlobReader::new(self.repo, chunks, self.ctl);
                let written = self.sink.file(&member, *size, &mut reader);
                if let Some(e) = reader.error.take() {
                    return Err(e);
                }
                if self.written(&name, written)? {
                    self.out.files += 1;
                    self.out.bytes += size;
                    self.ctl.update(|p| p.items_done += 1);
                }
            }
            EntryKind::Symlink { target } => {
                let written = self.sink.symlink(&member, target);
                if self.written(&name, written)? {
                    self.out.symlinks += 1;
                }
            }
        }
        Ok(())
    }

    /// Turns an unrepresentable name into a warning; other errors stop the export.
    fn written(&mut self, name: &Name, result: io::Result<()>) -> Result<bool> {
        match result {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                self.out.warnings.push(Warning {
                    path: name.clone(),
                    kind: WarningKind::Unsupported,
                    message: e.to_string(),
                });
                Ok(false)
            }
            Err(e) => Err(e).at(&self.output),
        }
    }
}

/// Streams a blob chunk by chunk, verifying each one.
struct BlobReader<'a> {
    repo: &'a Repository,
    chunks: std::vec::IntoIter<(ObjectId, u32)>,
    current: Vec<u8>,
    pos: usize,
    ctl: &'a Control,
    /// The real error behind an I/O error this reader returned.
    error: Option<Error>,
}

impl<'a> BlobReader<'a> {
    fn new(repo: &'a Repository, chunks: Vec<(ObjectId, u32)>, ctl: &'a Control) -> Self {
        BlobReader {
            repo,
            chunks: chunks.into_iter(),
            current: Vec::new(),
            pos: 0,
            ctl,
            error: None,
        }
    }

    fn fail(&mut self, e: Error) -> io::Error {
        let io = io::Error::other(e.to_string());
        self.error = Some(e);
        io
    }
}

impl Read for BlobReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.current.len() {
            let Some((id, len)) = self.chunks.next() else {
                return Ok(0);
            };
            if self.ctl.is_cancelled() {
                return Err(self.fail(Error::Cancelled));
            }
            match self.repo.read_chunk(&id, len) {
                Ok(chunk) => {
                    self.current = chunk;
                    self.pos = 0;
                }
                Err(e) => return Err(self.fail(e.into())),
            }
        }
        let n = buf.len().min(self.current.len() - self.pos);
        buf[..n].copy_from_slice(&self.current[self.pos..self.pos + n]);
        self.pos += n;
        self.ctl.update(|p| p.bytes_done += n as u64);
        Ok(n)
    }
}

fn not_representable(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{what} cannot be stored in this archive format"),
    )
}

/// A name as UTF-8, required by zip and 7z.
fn utf8_name(name: &Name) -> io::Result<&str> {
    std::str::from_utf8(name.as_bytes())
        .map_err(|_| not_representable("a name that is not valid UTF-8"))
}

fn system_time(t: Option<Time>) -> Option<std::time::SystemTime> {
    t.and_then(os::from_time)
}

const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;

struct TarSink(tar::Builder<File>);

impl TarSink {
    fn header(m: &Member<'_>, kind: tar::EntryType, default_mode: u16) -> tar::Header {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(kind);
        h.set_mode(u32::from(m.mode.unwrap_or(default_mode)));
        h.set_mtime(m.mtime.map_or(0, |t| u64::try_from(t.seconds).unwrap_or(0)));
        if let Some(uid) = m.uid {
            h.set_uid(u64::from(uid));
        }
        if let Some(gid) = m.gid {
            h.set_gid(u64::from(gid));
        }
        h.set_size(0);
        h
    }
}

#[cfg(unix)]
fn os_path(name: &Name) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(name.as_bytes())))
}

#[cfg(not(unix))]
fn os_path(name: &Name) -> io::Result<PathBuf> {
    utf8_name(name).map(PathBuf::from)
}

impl Sink for TarSink {
    fn dir(&mut self, m: &Member<'_>) -> io::Result<()> {
        let mut h = TarSink::header(m, tar::EntryType::Directory, 0o755);
        self.0.append_data(&mut h, os_path(m.name)?, io::empty())
    }

    fn file(&mut self, m: &Member<'_>, size: u64, content: &mut dyn Read) -> io::Result<()> {
        let mut h = TarSink::header(m, tar::EntryType::Regular, 0o644);
        h.set_size(size);
        self.0.append_data(&mut h, os_path(m.name)?, content)
    }

    fn symlink(&mut self, m: &Member<'_>, target: &[u8]) -> io::Result<()> {
        let mut h = TarSink::header(m, tar::EntryType::Symlink, 0o777);
        let target = os_path(&Name(target.to_vec()))?;
        self.0.append_link(&mut h, os_path(m.name)?, target)
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        self.0.into_inner()?.sync_all()
    }
}

struct ZipSink(zip::ZipWriter<File>);

impl ZipSink {
    fn options(m: &Member<'_>, default_mode: u16) -> zip::write::SimpleFileOptions {
        let mut options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(u32::from(m.mode.unwrap_or(default_mode)));
        if let Some(time) = m.mtime.and_then(zip_time) {
            options = options.last_modified_time(time);
        }
        options
    }
}

/// Zip stores local calendar times from 1980 to 2107; UTC is used here.
fn zip_time(t: Time) -> Option<zip::DateTime> {
    let (y, mo, d) = civil_from_days(t.seconds.div_euclid(86_400));
    let secs = t.seconds.rem_euclid(86_400);
    zip::DateTime::from_date_and_time(
        u16::try_from(y).ok()?,
        u8::try_from(mo).ok()?,
        u8::try_from(d).ok()?,
        u8::try_from(secs / 3600).ok()?,
        u8::try_from(secs % 3600 / 60).ok()?,
        u8::try_from(secs % 60).ok()?,
    )
    .ok()
}

/// Days since 1970-01-01 to a calendar date (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn zip_io(e: zip::result::ZipError) -> io::Error {
    match e {
        zip::result::ZipError::Io(e) => e,
        e => io::Error::other(e.to_string()),
    }
}

impl Sink for ZipSink {
    fn dir(&mut self, m: &Member<'_>) -> io::Result<()> {
        let options = ZipSink::options(m, 0o755);
        self.0
            .add_directory(utf8_name(m.name)?, options)
            .map_err(zip_io)
    }

    fn file(&mut self, m: &Member<'_>, size: u64, content: &mut dyn Read) -> io::Result<()> {
        let options = ZipSink::options(m, 0o644).large_file(size >= u64::from(u32::MAX));
        self.0
            .start_file(utf8_name(m.name)?, options)
            .map_err(zip_io)?;
        io::copy(content, &mut self.0)?;
        Ok(())
    }

    fn symlink(&mut self, m: &Member<'_>, target: &[u8]) -> io::Result<()> {
        let target = std::str::from_utf8(target)
            .map_err(|_| not_representable("a link target that is not valid UTF-8"))?;
        let options = ZipSink::options(m, 0o777);
        self.0
            .add_symlink(utf8_name(m.name)?, target, options)
            .map_err(zip_io)
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        self.0.finish().map_err(zip_io)?.sync_all()
    }
}

struct SevenZSink(sevenz_rust2::ArchiveWriter<File>);

/// Windows attributes with the Unix mode in the high 16 bits, as p7zip does.
fn sevenz_attributes(file_type: u32, mode: Option<u16>, default_mode: u16, windows: u32) -> u32 {
    const UNIX_EXTENSION: u32 = 0x8000;
    let mode = file_type | u32::from(mode.unwrap_or(default_mode));
    windows | UNIX_EXTENSION | (mode << 16)
}

impl SevenZSink {
    fn entry(m: &Member<'_>, entry: &mut sevenz_rust2::ArchiveEntry, attributes: u32) {
        if let Some(time) =
            system_time(m.mtime).and_then(|t| sevenz_rust2::NtTime::try_from(t).ok())
        {
            entry.has_last_modified_date = true;
            entry.last_modified_date = time;
        }
        entry.has_windows_attributes = true;
        entry.windows_attributes = attributes;
    }

    fn push(
        &mut self,
        entry: sevenz_rust2::ArchiveEntry,
        content: Option<&mut dyn Read>,
    ) -> io::Result<()> {
        self.0
            .push_archive_entry(entry, content)
            .map(drop)
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

impl Sink for SevenZSink {
    fn dir(&mut self, m: &Member<'_>) -> io::Result<()> {
        const DIRECTORY: u32 = 0x10;
        let mut entry = sevenz_rust2::ArchiveEntry::new_directory(utf8_name(m.name)?);
        SevenZSink::entry(
            m,
            &mut entry,
            sevenz_attributes(S_IFDIR, m.mode, 0o755, DIRECTORY),
        );
        self.push(entry, None)
    }

    fn file(&mut self, m: &Member<'_>, size: u64, content: &mut dyn Read) -> io::Result<()> {
        let mut entry = sevenz_rust2::ArchiveEntry::new_file(utf8_name(m.name)?);
        SevenZSink::entry(m, &mut entry, sevenz_attributes(S_IFREG, m.mode, 0o644, 0));
        self.push(entry, (size > 0).then_some(content))
    }

    fn symlink(&mut self, m: &Member<'_>, target: &[u8]) -> io::Result<()> {
        let mut entry = sevenz_rust2::ArchiveEntry::new_file(utf8_name(m.name)?);
        SevenZSink::entry(m, &mut entry, sevenz_attributes(S_IFLNK, m.mode, 0o777, 0));
        let mut target = target;
        self.push(entry, Some(&mut target))
    }

    fn finish(self: Box<Self>) -> io::Result<()> {
        self.0.finish()?.sync_all()
    }
}
