//! Legal holds: refs kept, with all they ever pointed to, until the hold is
//! lifted (FORMAT.md §17).

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::refs::validate_ref_name;

/// The longest reason, in characters.
const MAX_REASON: usize = 200;
const LIFTED: &str = ".lifted.toml";

/// One hold file (FORMAT.md §17.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hold {
    /// A ref name, or a prefix followed by `*`.
    pub refs: String,
    pub reason: String,
    /// Unix seconds when the hold was placed.
    pub added: u64,
}

/// The file that lifts a hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lift {
    pub reason: String,
    /// Unix seconds when the hold was lifted.
    pub lifted: u64,
}

/// A hold with its id and, once lifted, its lift.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalHold {
    pub id: String,
    #[serde(flatten)]
    pub hold: Hold,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lift: Option<Lift>,
}

impl LegalHold {
    /// Whether the hold keeps the ref `name` now.
    pub fn holds(&self, name: &str) -> bool {
        self.lift.is_none()
            && match self.hold.refs.strip_suffix('*') {
                Some(prefix) => name.starts_with(prefix),
                None => self.hold.refs == name,
            }
    }
}

fn check_reason(reason: &str) -> Result<()> {
    let n = reason.chars().count();
    if n == 0 || n > MAX_REASON || reason.contains(['\n', '\r']) {
        return Err(Error::InvalidHold(format!(
            "a reason is one line of 1 to {MAX_REASON} characters"
        )));
    }
    Ok(())
}

impl Hold {
    /// A new hold, checked.
    pub fn new(refs: &str, reason: &str, added: u64) -> Result<Hold> {
        let hold = Hold {
            refs: refs.to_owned(),
            reason: reason.trim().to_owned(),
            added,
        };
        hold.check()?;
        Ok(hold)
    }

    fn check(&self) -> Result<()> {
        validate_ref_name(&self.refs.replacen('*', "x", 1)).map_err(|_| {
            Error::InvalidHold(format!(
                "{:?} is not a ref name or a prefix followed by *",
                self.refs
            ))
        })?;
        if self
            .refs
            .find('*')
            .is_some_and(|i| i + 1 != self.refs.len())
        {
            return Err(Error::InvalidHold(format!(
                "{:?}: * may come only at the end",
                self.refs
            )));
        }
        check_reason(&self.reason)
    }

    /// The file's name, `<id>.toml`, and its bytes.
    pub(crate) fn encode(&self) -> Result<(String, Vec<u8>)> {
        let text = toml::to_string(self).map_err(|e| Error::Encode(e.to_string()))?;
        Ok((
            format!("{}.toml", id_of(text.as_bytes())),
            text.into_bytes(),
        ))
    }
}

impl Lift {
    /// The lift of the hold `id`: its file's name and bytes.
    pub(crate) fn encode(&self, id: &str) -> Result<(String, Vec<u8>)> {
        check_reason(&self.reason)?;
        let text = toml::to_string(self).map_err(|e| Error::Encode(e.to_string()))?;
        Ok((format!("{id}{LIFTED}"), text.into_bytes()))
    }
}

fn id_of(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex()[..32].to_owned()
}

/// Whether `name` is the name of a lift file; then the hold's id.
pub(crate) fn lifts(name: &str) -> Option<&str> {
    name.strip_suffix(LIFTED)
}

/// Reads a hold file; one whose name is not its hash or that does not
/// parse is damaged.
fn decode_hold(path: &Path, name: &str, bytes: &[u8]) -> Result<Hold> {
    let damaged = |reason: String| Error::corrupt(path, reason);
    let id = name
        .strip_suffix(".toml")
        .ok_or_else(|| damaged("not a .toml file".to_owned()))?;
    if id_of(bytes) != id {
        return Err(damaged(
            "the name is not the hash of the content".to_owned(),
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| damaged("not UTF-8".to_owned()))?;
    let hold: Hold = toml::from_str(text).map_err(|e| damaged(e.to_string()))?;
    hold.check().map_err(|e| damaged(e.to_string()))?;
    Ok(hold)
}

fn decode_lift(path: &Path, bytes: &[u8], hold: &Hold) -> Result<Lift> {
    let damaged = |reason: String| Error::corrupt(path, reason);
    let text = std::str::from_utf8(bytes).map_err(|_| damaged("not UTF-8".to_owned()))?;
    let lift: Lift = toml::from_str(text).map_err(|e| damaged(e.to_string()))?;
    check_reason(&lift.reason).map_err(|e| damaged(e.to_string()))?;
    if lift.lifted < hold.added {
        return Err(damaged("lifted before it was placed".to_owned()));
    }
    Ok(lift)
}

/// The files of `dir` by name with their bytes, hold files before lift
/// files; none if the folder does not exist.
pub(crate) fn read_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).at(dir),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.at(dir)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".toml") {
            continue;
        }
        let path = entry.path();
        files.push((name, fs::read(&path).at(&path)?));
    }
    files.sort_by(|a, b| (lifts(&a.0).is_some(), &a.0).cmp(&(lifts(&b.0).is_some(), &b.0)));
    Ok(files)
}

/// Checks the files of `dir`, as [`read_files`] gives them, and returns
/// every hold, oldest first.
pub(crate) fn decode_all(dir: &Path, files: &[(String, Vec<u8>)]) -> Result<Vec<LegalHold>> {
    let mut holds: Vec<LegalHold> = Vec::new();
    for (name, bytes) in files {
        let path = dir.join(name);
        match lifts(name) {
            None => holds.push(LegalHold {
                id: name.trim_end_matches(".toml").to_owned(),
                hold: decode_hold(&path, name, bytes)?,
                lift: None,
            }),
            Some(id) => {
                let held = holds
                    .iter_mut()
                    .find(|h| h.id == id)
                    .ok_or_else(|| Error::corrupt(&path, "it lifts no hold".to_owned()))?;
                held.lift = Some(decode_lift(&path, bytes, &held.hold)?);
            }
        }
    }
    holds.sort_by(|a, b| (a.hold.added, &a.id).cmp(&(b.hold.added, &b.id)));
    Ok(holds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_and_lifts_are_checked() {
        assert!(Hold::new("backup/*", "Case 1", 10).is_ok());
        assert!(Hold::new("back*up", "Case 1", 10).is_err());
        assert!(Hold::new("backup/a", "", 10).is_err());
        assert!(Hold::new("backup/a", "two\nlines", 10).is_err());

        let dir = Path::new("holds");
        let hold = Hold::new("backup/*", "Case 1", 10).unwrap();
        let (name, bytes) = hold.encode().unwrap();
        let id = name.trim_end_matches(".toml").to_owned();
        let lift = Lift {
            reason: "Closed".to_owned(),
            lifted: 20,
        };
        let lifted = lift.encode(&id).unwrap();
        let all = decode_all(dir, &[(name.clone(), bytes.clone()), lifted.clone()]).unwrap();
        assert_eq!(all[0].lift.as_ref(), Some(&lift));
        assert!(!all[0].holds("backup/a"));
        let open = decode_all(dir, &[(name.clone(), bytes.clone())]).unwrap();
        assert!(open[0].holds("backup/a") && !open[0].holds("other"));

        // A changed hold, a lift of nothing and a lift before the hold are
        // damaged.
        let mut changed = bytes.clone();
        changed[0] ^= 1;
        assert!(decode_all(dir, &[(name.clone(), changed)]).is_err());
        assert!(decode_all(dir, &[lifted]).is_err());
        let early = Lift {
            reason: "x".to_owned(),
            lifted: 5,
        }
        .encode(&id)
        .unwrap();
        assert!(decode_all(dir, &[(name, bytes), early]).is_err());
    }
}
