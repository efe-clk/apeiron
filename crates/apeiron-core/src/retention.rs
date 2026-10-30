//! Retention rules: refs whose history can only grow and which cannot be
//! deleted before their retention has passed (FORMAT.md §16).

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::refs::validate_ref_name;

/// The longest retention, in days: about a hundred years.
pub const MAX_DAYS: u32 = 36_500;

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// One rule file (FORMAT.md §16.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// A ref name, or a prefix followed by `*`.
    pub refs: String,
    pub days: u32,
    /// Unix seconds when the rule was added.
    pub added: u64,
}

impl Rule {
    /// A new rule, checked.
    pub fn new(refs: &str, days: u32, added: u64) -> Result<Rule> {
        let rule = Rule {
            refs: refs.to_owned(),
            days,
            added,
        };
        rule.check()?;
        Ok(rule)
    }

    fn check(&self) -> Result<()> {
        validate_ref_name(&self.refs.replacen('*', "x", 1)).map_err(|_| {
            Error::InvalidRetention(format!(
                "{:?} is not a ref name or a prefix followed by *",
                self.refs
            ))
        })?;
        let star = self.refs.find('*');
        if star.is_some_and(|i| i + 1 != self.refs.len()) {
            return Err(Error::InvalidRetention(format!(
                "{:?}: * may come only at the end",
                self.refs
            )));
        }
        if !(1..=MAX_DAYS).contains(&self.days) {
            return Err(Error::InvalidRetention(format!(
                "a retention lasts 1 to {MAX_DAYS} days, not {}",
                self.days
            )));
        }
        Ok(())
    }

    /// Whether the rule protects the ref `name`.
    pub fn matches(&self, name: &str) -> bool {
        match self.refs.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => self.refs == name,
        }
    }

    /// The file's bytes and its name, `<id>.toml`.
    pub(crate) fn encode(&self) -> Result<(String, Vec<u8>)> {
        let text = toml::to_string(self).map_err(|e| Error::Encode(e.to_string()))?;
        Ok((file_name(text.as_bytes()), text.into_bytes()))
    }
}

/// The name a rule file with `bytes` has.
fn file_name(bytes: &[u8]) -> String {
    format!("{}.toml", &blake3::hash(bytes).to_hex()[..32])
}

/// Reads one rule file; a file whose name is not its hash or that does
/// not parse is damaged.
pub(crate) fn decode(path: &Path, name: &str, bytes: &[u8]) -> Result<Rule> {
    let damaged = |reason: String| Error::corrupt(path, reason);
    if file_name(bytes) != name {
        return Err(damaged(
            "the name is not the hash of the content".to_owned(),
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| damaged("not UTF-8".to_owned()))?;
    let rule: Rule = toml::from_str(text).map_err(|e| damaged(e.to_string()))?;
    rule.check().map_err(|e| damaged(e.to_string()))?;
    Ok(rule)
}

/// The rule files in `dir`, checked, by name with their bytes; none if the
/// folder does not exist.
pub(crate) fn read_files(dir: &Path) -> Result<Vec<(String, Vec<u8>, Rule)>> {
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
        let bytes = fs::read(&path).at(&path)?;
        let rule = decode(&path, &name, &bytes)?;
        files.push((name, bytes, rule));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// The rules in `dir`, sorted by when they were added.
pub(crate) fn read_rules(dir: &Path) -> Result<Vec<Rule>> {
    let mut rules: Vec<Rule> = read_files(dir)?.into_iter().map(|f| f.2).collect();
    rules.sort_by(|a, b| (a.added, &a.refs, a.days).cmp(&(b.added, &b.refs, b.days)));
    Ok(rules)
}

/// The retention in days that protects `name`: the longest of the rules
/// that match it, or `None`.
pub(crate) fn days_for(rules: &[Rule], name: &str) -> Option<u32> {
    rules
        .iter()
        .filter(|r| r.matches(name))
        .map(|r| r.days)
        .max()
}

/// When a ref last changed at `changed` and kept `days` may be deleted.
pub(crate) fn ends(changed: u64, days: u32) -> u64 {
    changed.saturating_add(u64::from(days) * SECONDS_PER_DAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_days_and_names_are_checked() {
        assert!(
            Rule::new("archive/*", 10, 0)
                .unwrap()
                .matches("archive/x/y")
        );
        assert!(!Rule::new("archive/*", 10, 0).unwrap().matches("backup/x"));
        assert!(Rule::new("*", 10, 0).unwrap().matches("main"));
        assert!(Rule::new("main", 10, 0).unwrap().matches("main"));
        assert!(!Rule::new("main", 10, 0).unwrap().matches("main2"));
        for (refs, days) in [
            ("a*b", 1),
            ("a/../b", 1),
            ("", 1),
            ("main", 0),
            ("main", MAX_DAYS + 1),
        ] {
            assert!(Rule::new(refs, days, 0).is_err(), "{refs} {days}");
        }

        let dir = tempfile::tempdir().unwrap();
        assert!(read_rules(&dir.path().join("none")).unwrap().is_empty());
        let rule = Rule::new("archive/*", 3650, 1_793_318_400).unwrap();
        let (name, bytes) = rule.encode().unwrap();
        fs::write(dir.path().join(&name), &bytes).unwrap();
        assert_eq!(read_rules(dir.path()).unwrap(), std::slice::from_ref(&rule));
        assert_eq!(
            days_for(std::slice::from_ref(&rule), "archive/a"),
            Some(3650)
        );
        assert_eq!(days_for(&[rule], "main"), None);

        // A changed file no longer has its name.
        let text = String::from_utf8(bytes).unwrap().replace("3650", "1");
        fs::write(dir.path().join(&name), text).unwrap();
        assert!(matches!(read_rules(dir.path()), Err(Error::Corrupt { .. })));
    }
}
