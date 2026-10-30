//! The audit log of the web interface (AUDIT.md): JSON lines, each holding
//! the hash of the line before it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use apeiron::{Error, ErrorKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Longest string kept in `detail` (AUDIT.md §2).
const MAX_TEXT: usize = 256;
/// Size past which the log goes on in a new file (AUDIT.md §3).
const ROTATE_AT: u64 = 16 * 1024 * 1024;
/// `prev` of the first line.
const START: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One line of the log (AUDIT.md §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub event: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
    pub outcome: String,
    pub prev: String,
}

/// What `verify` found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Verified {
    /// Lines checked.
    pub events: u64,
    /// The `seq` of the first line checked: 1, or more when older parts
    /// were removed.
    pub first: u64,
    /// SHA-256 of the last line, hex; worth keeping elsewhere (AUDIT.md §3).
    pub last: String,
}

/// Who did something and from where.
#[derive(Clone, Debug, Default)]
pub struct Actor {
    pub user: Option<String>,
    pub from: Option<String>,
}

fn hash(line: &[u8]) -> String {
    Sha256::digest(line)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn io_error(path: &Path, e: io::Error) -> Error {
    Error::new(ErrorKind::Io, format!("{}: {e}", path.display()))
}

/// Cuts long strings, so one request cannot fill the log.
fn shorten(value: Value) -> Value {
    match value {
        Value::String(s) if s.chars().count() > MAX_TEXT => {
            Value::String(s.chars().take(MAX_TEXT).chain(['…']).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(shorten).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, shorten(v))).collect())
        }
        other => other,
    }
}

/// The audit log: `audit.log` and the parts it went on from.
#[derive(Clone, Debug)]
pub struct Audit {
    file: PathBuf,
    rotate_at: u64,
}

impl Audit {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Audit {
            file: file.into(),
            rotate_at: ROTATE_AT,
        }
    }

    /// The same log, going on in a new file past `bytes` instead of 16 MiB.
    pub fn rotating_at(mut self, bytes: u64) -> Self {
        self.rotate_at = bytes;
        self
    }

    /// `audit.log.<suffix>` beside the log.
    fn beside(&self, suffix: &str) -> PathBuf {
        let mut name = self.file.file_name().unwrap_or_default().to_owned();
        name.push(".");
        name.push(suffix);
        self.file.with_file_name(name)
    }

    /// Takes the lock that orders writers and readers, exclusive to write.
    fn lock(&self, write: bool) -> Result<File, Error> {
        let path = self.beside("lock");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| io_error(&path, e))?;
        if write {
            file.lock()
        } else {
            file.lock_shared()
        }
        .map_err(|e| io_error(&path, e))?;
        Ok(file)
    }

    /// The parts the log went on from, oldest first (AUDIT.md §3).
    fn parts(&self) -> Result<Vec<PathBuf>, Error> {
        let dir = self.file.parent().unwrap_or(Path::new("."));
        let prefix = format!(
            "{}.",
            self.file.file_name().unwrap_or_default().to_string_lossy()
        );
        let entries = match fs::read_dir(dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(dir, e)),
            Ok(entries) => entries,
        };
        let mut parts = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| io_error(dir, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_part = name
                .strip_prefix(&prefix)
                .is_some_and(|n| n.len() == 12 && n.bytes().all(|b| b.is_ascii_digit()));
            if is_part {
                parts.push(entry.path());
            }
        }
        parts.sort();
        Ok(parts)
    }

    /// Renames a full `audit.log` to the part named by its last event.
    fn rotate(&self) -> Result<(), Error> {
        let path = &self.file;
        let size = match fs::metadata(path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(io_error(path, e)),
        };
        if size < self.rotate_at {
            return Ok(());
        }
        let mut file = File::open(path).map_err(|e| io_error(path, e))?;
        let last = last_line(&mut file).map_err(|e| io_error(path, e))?;
        drop(file);
        let Some(seq) = last
            .and_then(|l| serde_json::from_slice::<Event>(&l).ok())
            .map(|e| e.seq)
        else {
            // A file ending in a broken line keeps growing; `verify` shows it.
            return Ok(());
        };
        let part = self.beside(&format!("{seq:012}"));
        if part.exists() {
            return Ok(());
        }
        fs::rename(path, &part).map_err(|e| io_error(path, e))
    }

    /// `audit.log` in the state folder (AUDIT.md §2).
    pub fn default_file() -> Option<PathBuf> {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|d| d.join("apeiron").join("audit.log"))
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Appends an event and flushes it to disk.
    pub fn record(
        &self,
        actor: &Actor,
        event: &str,
        detail: Value,
        outcome: &str,
    ) -> Result<(), Error> {
        let path = &self.file;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
        }
        // The server and `apeiron web user` may write at the same time.
        let _lock = self.lock(true)?;
        self.rotate()?;
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)
            .map_err(|e| io_error(path, e))?;
        let mut last = last_line(&mut file).map_err(|e| io_error(path, e))?;
        // A new file goes on from the last part.
        if last.is_none()
            && let Some(part) = self.parts()?.pop()
        {
            let mut old = File::open(&part).map_err(|e| io_error(&part, e))?;
            last = last_line(&mut old).map_err(|e| io_error(&part, e))?;
        }
        let (seq, prev) = match last {
            None => (1, START.to_owned()),
            Some(line) => {
                let seq = serde_json::from_slice::<Event>(&line).map_or(0, |e| e.seq);
                (seq + 1, hash(&line))
            }
        };
        let event = Event {
            seq,
            time: jiff::Timestamp::now()
                .round(jiff::Unit::Second)
                .map_or_else(|_| jiff::Timestamp::now().to_string(), |t| t.to_string()),
            user: actor.user.clone(),
            from: actor.from.clone(),
            event: event.to_owned(),
            detail: shorten(detail),
            outcome: outcome.to_owned(),
            prev,
        };
        let mut line = serde_json::to_vec(&event)
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        line.push(b'\n');
        // A line cut short by a crash stays alone on its line; `verify`
        // reports it.
        if ends_open(&mut file).map_err(|e| io_error(path, e))? {
            line.insert(0, b'\n');
        }
        file.write_all(&line)
            .and_then(|()| file.sync_data())
            .map_err(|e| io_error(path, e))
    }

    /// The lines of every part and of `audit.log`, oldest first, each with
    /// its file and line number; read while no event is being written.
    fn lines(&self) -> Result<Vec<(PathBuf, usize, Vec<u8>)>, Error> {
        let _lock = match self.file.parent() {
            Some(dir) if !dir.as_os_str().is_empty() && !dir.exists() => return Ok(Vec::new()),
            _ => self.lock(false)?,
        };
        let mut files = self.parts()?;
        files.push(self.file.clone());
        let mut lines = Vec::new();
        for path in files {
            let data = match fs::read(&path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_error(&path, e)),
                Ok(data) => data,
            };
            for (i, line) in data
                .split(|b| *b == b'\n')
                .filter(|l| !l.is_empty())
                .enumerate()
            {
                lines.push((path.clone(), i + 1, line.to_vec()));
            }
        }
        Ok(lines)
    }

    /// The newest `limit` events, of `user` only if given, oldest first.
    pub fn read(&self, limit: usize, user: Option<&str>) -> Result<Vec<Event>, Error> {
        let mut events: Vec<Event> = self
            .lines()?
            .iter()
            .filter_map(|(_, _, l)| serde_json::from_slice::<Event>(l).ok())
            .filter(|e| user.is_none_or(|u| e.user.as_deref() == Some(u)))
            .collect();
        let skip = events.len().saturating_sub(limit);
        events.drain(..skip);
        Ok(events)
    }

    /// Checks the chain of the whole log, across its parts (AUDIT.md §3).
    pub fn verify(&self) -> Result<Verified, Error> {
        let mut prev = START.to_owned();
        let mut next = None;
        let mut first = 1;
        let mut count = 0;
        for (path, n, line) in &self.lines()? {
            let broken = |why: &str| {
                Error::new(
                    ErrorKind::Corrupt,
                    format!("{}: line {n} {why}", path.display()),
                )
            };
            let event: Event =
                serde_json::from_slice(line).map_err(|_| broken("is not an event"))?;
            match next {
                // The log starts at 1 unless older parts were removed; then
                // the chain is checked from the oldest line left.
                None if event.seq > 1 => {
                    first = event.seq;
                    prev.clone_from(&event.prev);
                }
                None => {}
                Some(seq) if event.seq != seq => {
                    return Err(broken("is out of order or a line before it is missing"));
                }
                Some(_) => {}
            }
            if event.prev != prev {
                return Err(broken("does not follow the line before it"));
            }
            prev = hash(line);
            next = Some(event.seq + 1);
            count += 1;
        }
        Ok(Verified {
            events: count,
            first,
            last: prev,
        })
    }
}

/// Whether `file` is not empty and does not end with a newline.
fn ends_open(file: &mut File) -> io::Result<bool> {
    let len = file.seek(SeekFrom::End(0))?;
    if len == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(len - 1))?;
    let mut last = [0];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

/// The last line of `file` without its newline, read from the end.
fn last_line(file: &mut File) -> io::Result<Option<Vec<u8>>> {
    let len = file.seek(SeekFrom::End(0))?;
    let mut tail = Vec::new();
    let mut start = len;
    // Read backwards in growing steps until a whole line is in `tail`.
    let mut step = 4096;
    loop {
        let from = start.saturating_sub(step);
        file.seek(SeekFrom::Start(from))?;
        let mut chunk = vec![0; (start - from) as usize];
        file.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&tail);
        tail = chunk;
        start = from;
        let body = tail.strip_suffix(b"\n").unwrap_or(&tail);
        if let Some(i) = body.iter().rposition(|b| *b == b'\n') {
            return Ok(Some(body[i + 1..].to_vec()));
        }
        if start == 0 {
            return Ok((!body.is_empty()).then(|| body.to_vec()));
        }
        step *= 2;
    }
}
