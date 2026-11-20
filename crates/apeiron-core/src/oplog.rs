//! The operation log (FORMAT.md §9.5): one line for every operation that
//! changed the repository, with who ran it and how it ended.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{Error, IoContext, Result};
use crate::fsutil;
use crate::id::to_hex;

const FILE: &str = "oplog";

/// How an operation ended.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Failed,
    Cancelled,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Failed => "failed",
            Outcome::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(Outcome::Ok),
            "failed" => Some(Outcome::Failed),
            "cancelled" => Some(Outcome::Cancelled),
            _ => None,
        }
    }
}

/// One line of the operation log.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Operation {
    /// Unix time in seconds when the operation ended.
    pub time: u64,
    /// The name of the operation, such as `backup` or `gc`.
    pub op: String,
    pub outcome: Outcome,
    /// The user of the operating system that ran it, when known.
    pub user: Option<String>,
    /// What it worked on: ref, remote, key or hold names, separated by
    /// spaces.
    pub detail: Option<String>,
}

impl Operation {
    fn encode(&self) -> String {
        let body = format!(
            "{} {} {} {} {}",
            self.time,
            field(Some(&self.op)),
            self.outcome.as_str(),
            field(self.user.as_deref()),
            field(self.detail.as_deref())
        );
        let check = short_check(&body);
        format!("{body} {check}\n")
    }

    pub(crate) fn decode(line: &str) -> Option<Self> {
        let (body, check) = line.rsplit_once(' ')?;
        if short_check(body) != check {
            return None;
        }
        let mut fields = body.split(' ');
        let time = fields.next()?.parse().ok()?;
        let op = unfield(fields.next()?)??;
        let outcome = Outcome::parse(fields.next()?)?;
        let user = unfield(fields.next()?)?;
        let detail = unfield(fields.next()?)?;
        if fields.next().is_some() {
            return None;
        }
        Some(Operation {
            time,
            op,
            outcome,
            user,
            detail,
        })
    }
}

pub(crate) fn short_check(body: &str) -> String {
    to_hex(&blake3::hash(body.as_bytes()).as_bytes()[..8])
}

/// A text field: `-` when absent, otherwise the text with `%`, `-` at the
/// start, and bytes outside printable ASCII or a space written as `%XX`.
fn field(text: Option<&str>) -> String {
    let Some(text) = text else {
        return "-".to_owned();
    };
    if text.is_empty() {
        return "-".to_owned();
    }
    let mut out = String::new();
    for (i, b) in text.bytes().enumerate() {
        if b == b'%' || (i == 0 && b == b'-') || !(0x21..0x7f).contains(&b) {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(char::from(b));
        }
    }
    out
}

/// The text of a field; `None` when the field is invalid.
fn unfield(field: &str) -> Option<Option<String>> {
    if field == "-" {
        return Some(None);
    }
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(Some)
}

/// Appends `op` to the operation log of the repository at `repo`. Writers
/// take a lock on the file, so lines of programs running at once do not mix.
pub fn append(repo: &Path, op: &Operation) -> Result<()> {
    let path = repo.join(FILE);
    let existed = path.exists();
    let mut file = fsutil::checkpoint()
        .and_then(|()| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
        })
        .at(&path)?;
    fs4::FileExt::lock(&file).at(&path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).at(&path)?;
    let (_, valid_len) = parse(&path, &bytes)?;
    // Drop a torn last line left by a crash before appending.
    fsutil::checkpoint()
        .and_then(|()| file.set_len(valid_len))
        .and_then(|()| file.seek(SeekFrom::Start(valid_len)).map(|_| ()))
        .and_then(|()| fsutil::checkpoint())
        .and_then(|()| file.write_all(op.encode().as_bytes()))
        .and_then(|()| fsutil::checkpoint())
        .and_then(|()| file.sync_all())
        .at(&path)?;
    if !existed {
        fsutil::sync_dir(repo)?;
    }
    Ok(())
}

/// Every operation of the log, oldest first.
pub fn read(repo: &Path) -> Result<Vec<Operation>> {
    let path = repo.join(FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).at(&path),
    };
    Ok(parse(&path, &bytes)?.0)
}

/// The valid lines and the byte length they occupy.
fn parse(path: &Path, bytes: &[u8]) -> Result<(Vec<Operation>, u64)> {
    let mut ops = Vec::new();
    let mut valid_len = 0usize;
    let lines: Vec<&[u8]> = bytes.split_inclusive(|&b| b == b'\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let op = line
            .strip_suffix(b"\n")
            .and_then(|l| std::str::from_utf8(l).ok())
            .and_then(Operation::decode);
        match op {
            Some(op) => {
                ops.push(op);
                valid_len += line.len();
            }
            // A crash can only damage the last line.
            None if i + 1 == lines.len() => break,
            None => {
                return Err(Error::corrupt(
                    path,
                    format!("invalid operation log line {}", i + 1),
                ));
            }
        }
    }
    Ok((ops, valid_len as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(name: &str, detail: Option<&str>) -> Operation {
        Operation {
            time: 1_700_000_000,
            op: name.to_owned(),
            outcome: Outcome::Ok,
            user: Some("efe".to_owned()),
            detail: detail.map(str::to_owned),
        }
    }

    #[test]
    fn line_format_matches_spec() {
        let line = op("backup", Some("backup/home")).encode();
        let body = "1700000000 backup ok efe backup/home";
        assert_eq!(line, format!("{body} {}\n", short_check(body)));
    }

    #[test]
    fn odd_text_survives() {
        let dir = tempfile::tempdir().unwrap();
        let odd = op("hold-add", Some("- 100% a b\nç"));
        append(dir.path(), &odd).unwrap();
        append(dir.path(), &op("gc", None)).unwrap();
        assert_eq!(read(dir.path()).unwrap(), [odd, op("gc", None)]);
    }

    #[test]
    fn torn_last_line_is_ignored_and_truncated() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &op("gc", None)).unwrap();
        let path = dir.path().join(FILE);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"17 half a li").unwrap();
        drop(file);
        assert_eq!(read(dir.path()).unwrap().len(), 1);
        append(dir.path(), &op("prune", Some("main"))).unwrap();
        assert_eq!(read(dir.path()).unwrap().len(), 2);
        assert!(!fs::read_to_string(&path).unwrap().contains("half"));
    }

    #[test]
    fn a_changed_middle_line_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &op("gc", None)).unwrap();
        append(dir.path(), &op("prune", Some("main"))).unwrap();
        let path = dir.path().join(FILE);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, text.replacen(" gc ", " gd ", 1)).unwrap();
        assert!(matches!(read(dir.path()), Err(Error::Corrupt { .. })));
    }
}
