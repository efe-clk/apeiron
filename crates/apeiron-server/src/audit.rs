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

/// The audit log in one file.
#[derive(Clone, Debug)]
pub struct Audit {
    file: PathBuf,
}

impl Audit {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Audit { file: file.into() }
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
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)
            .map_err(|e| io_error(path, e))?;
        // The server and `apeiron web user` may write at the same time.
        file.lock().map_err(|e| io_error(path, e))?;
        let (seq, prev) = match last_line(&mut file).map_err(|e| io_error(path, e))? {
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

    /// The lines of the log, read once no event is being written: the
    /// writer's lock would make a read fail on Windows and could show half a
    /// line elsewhere.
    fn lines(&self) -> Result<Vec<Vec<u8>>, Error> {
        let mut file = match fs::File::open(&self.file) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(&self.file, e)),
            Ok(file) => file,
        };
        let mut data = Vec::new();
        file.lock_shared()
            .and_then(|()| file.read_to_end(&mut data))
            .map_err(|e| io_error(&self.file, e))?;
        Ok(data
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(<[u8]>::to_vec)
            .collect())
    }

    /// The newest `limit` events, of `user` only if given, oldest first.
    pub fn read(&self, limit: usize, user: Option<&str>) -> Result<Vec<Event>, Error> {
        let mut events: Vec<Event> = self
            .lines()?
            .iter()
            .filter_map(|l| serde_json::from_slice::<Event>(l).ok())
            .filter(|e| user.is_none_or(|u| e.user.as_deref() == Some(u)))
            .collect();
        let skip = events.len().saturating_sub(limit);
        events.drain(..skip);
        Ok(events)
    }

    /// Checks the chain of the whole log (AUDIT.md §3).
    pub fn verify(&self) -> Result<Verified, Error> {
        let mut prev = START.to_owned();
        let mut count = 0;
        for (i, line) in self.lines()?.iter().enumerate() {
            let n = i + 1;
            let broken = |why: &str| {
                Error::new(
                    ErrorKind::Corrupt,
                    format!("{}: line {n} {why}", self.file.display()),
                )
            };
            let event: Event =
                serde_json::from_slice(line).map_err(|_| broken("is not an event"))?;
            if event.seq != n as u64 {
                return Err(broken("is out of order or a line before it is missing"));
            }
            if event.prev != prev {
                return Err(broken("does not follow the line before it"));
            }
            prev = hash(line);
            count = n as u64;
        }
        Ok(Verified {
            events: count,
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
