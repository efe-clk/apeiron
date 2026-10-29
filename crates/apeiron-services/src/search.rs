//! Finding files in snapshots and their history (SEARCH.md).

use std::collections::{HashMap, HashSet};

use apeiron_core::{EntryKind, ObjectId, Repository, Time, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::control::{Control, Phase};
use crate::error::{Error, Result};
use crate::list::EntryInfo;
use crate::name::Name;

/// Files larger than this are not read for text (SEARCH.md §3).
const TEXT_LIMIT: u64 = 16 << 20;
/// Longest line shown for a text match, in characters.
const LINE_CHARS: usize = 200;

/// What an entry must match; every criterion given must hold (SEARCH.md §1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    /// A name, or with `/` a whole path; `*` and `?` as wildcards, ignoring case.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Text a text file must hold, ignoring case.
    #[serde(default)]
    pub text: Option<String>,
    /// Smallest size of a file, in bytes.
    #[serde(default)]
    pub larger: Option<u64>,
    /// A file must be smaller than this, in bytes.
    #[serde(default)]
    pub smaller: Option<u64>,
    /// Modified at or after this time.
    #[serde(default)]
    pub after: Option<Time>,
    /// Modified before this time.
    #[serde(default)]
    pub before: Option<Time>,
}

impl Query {
    fn is_empty(&self) -> bool {
        self.pattern.is_none()
            && self.text.is_none()
            && self.larger.is_none()
            && self.smaller.is_none()
            && self.after.is_none()
            && self.before.is_none()
    }

    /// Only files can match text and size criteria.
    fn files_only(&self) -> bool {
        self.text.is_some() || self.larger.is_some() || self.smaller.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchOptions {
    /// The snapshots searched.
    pub starts: Vec<ObjectId>,
    /// Also search every ancestor of `starts`.
    pub history: bool,
    pub query: Query,
    /// Most results returned.
    pub limit: usize,
}

/// The first line of a file holding the text searched for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextLine {
    /// Counted from 1.
    pub number: u64,
    pub text: String,
}

/// One version of a path that matched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: Name,
    pub entry: EntryInfo,
    /// The newest snapshot searched that holds this version.
    pub snapshot: ObjectId,
    pub time: Time,
    pub line: Option<TextLine>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOutcome {
    /// Newest snapshot first, then by path.
    pub hits: Vec<SearchHit>,
    /// More results exist than the limit allowed.
    pub truncated: bool,
    /// Number of snapshots searched.
    pub snapshots: u64,
    /// Files not read for text because they are too large.
    pub skipped: u64,
}

/// Searches snapshots newest first, walking each folder version only once
/// per path and reporting each version of a path once (SEARCH.md §2).
pub fn search(repo: &Repository, options: &SearchOptions, ctl: &Control) -> Result<SearchOutcome> {
    if options.query.is_empty() {
        return Err(Error::InvalidInput(
            "a search needs at least one criterion".to_owned(),
        ));
    }
    ctl.set_phase(Phase::Scanning);
    let snapshots = snapshots(repo, options, ctl)?;
    ctl.update(|p| p.items_total = Some(snapshots.len() as u64));

    let pattern = options.query.pattern.as_deref().map(|p| {
        let lower = p.to_lowercase();
        (lower.contains('/'), lower.chars().collect::<Vec<_>>())
    });
    let mut s = Searcher {
        repo,
        query: &options.query,
        pattern,
        text: options.query.text.as_deref().map(str::to_lowercase),
        limit: options.limit,
        walked: HashSet::new(),
        reported: HashSet::new(),
        texts: HashMap::new(),
        out: SearchOutcome {
            hits: Vec::new(),
            truncated: false,
            snapshots: 0,
            skipped: 0,
        },
        ctl,
    };
    for (id, tree, time) in snapshots {
        s.walk(&tree, &Name::default(), id, time)?;
        s.out.snapshots += 1;
        ctl.update(|p| p.items_done += 1);
        if s.out.truncated {
            break;
        }
    }
    ctl.set_phase(Phase::Done);
    Ok(s.out)
}

/// The snapshots to search with their trees and times, newest first.
fn snapshots(
    repo: &Repository,
    options: &SearchOptions,
    ctl: &Control,
) -> Result<Vec<(ObjectId, ObjectId, Time)>> {
    let mut seen = HashSet::new();
    let mut queue: Vec<ObjectId> = options.starts.clone();
    let mut out = Vec::new();
    while let Some(id) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        ctl.check()?;
        let snapshot = repo.read_snapshot(&id)?;
        if options.history {
            queue.extend(snapshot.parents.iter().copied());
        }
        out.push((id, snapshot.tree, snapshot.time));
    }
    out.sort_by_key(|a| std::cmp::Reverse((a.2.seconds, a.2.nanos, a.0)));
    Ok(out)
}

/// What makes two entries at one path the same version.
#[derive(PartialEq, Eq, Hash)]
enum Version {
    File(Option<ObjectId>),
    /// A folder is reported once per path, whatever it holds.
    Dir,
    Link(Vec<u8>),
}

/// What reading a file for text found.
#[derive(Clone)]
enum Text {
    Found(Option<TextLine>),
    TooLarge,
}

struct Searcher<'a> {
    repo: &'a Repository,
    query: &'a Query,
    /// Whether the pattern matches whole paths, and its lowercase characters.
    pattern: Option<(bool, Vec<char>)>,
    /// Lowercase text searched for.
    text: Option<String>,
    limit: usize,
    /// Folders already walked, by path and tree.
    walked: HashSet<(Name, ObjectId)>,
    /// Versions already reported, by path.
    reported: HashSet<(Name, Version)>,
    /// What each file content holds, by blob.
    texts: HashMap<ObjectId, Text>,
    out: SearchOutcome,
    ctl: &'a Control,
}

impl Searcher<'_> {
    fn walk(
        &mut self,
        tree: &ObjectId,
        prefix: &Name,
        snapshot: ObjectId,
        time: Time,
    ) -> Result<()> {
        if !self.walked.insert((prefix.clone(), *tree)) {
            return Ok(());
        }
        self.ctl.check()?;
        for entry in self.repo.read_tree(tree)?.entries {
            let path = prefix.join(&entry.name);
            if let Some(line) = self.matches(&entry, &path)? {
                let version = match &entry.kind {
                    EntryKind::File { blob, .. } => Version::File(*blob),
                    EntryKind::Directory { .. } => Version::Dir,
                    EntryKind::Symlink { target } => Version::Link(target.clone()),
                };
                if self.reported.insert((path.clone(), version)) {
                    if self.out.hits.len() == self.limit {
                        self.out.truncated = true;
                        return Ok(());
                    }
                    self.out.hits.push(SearchHit {
                        path: path.clone(),
                        entry: EntryInfo::from(&entry),
                        snapshot,
                        time,
                        line,
                    });
                }
            }
            if let EntryKind::Directory { tree } = &entry.kind {
                self.walk(tree, &path, snapshot, time)?;
                if self.out.truncated {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// `Some` when the entry matches, with the line holding the text if text
    /// was searched for.
    fn matches(&mut self, entry: &TreeEntry, path: &Name) -> Result<Option<Option<TextLine>>> {
        let q = self.query;
        if let Some((whole, pattern)) = &self.pattern {
            let subject = if *whole { path.as_bytes() } else { &entry.name };
            let subject: Vec<char> = String::from_utf8_lossy(subject)
                .to_lowercase()
                .chars()
                .collect();
            if !wildcard(pattern, &subject) {
                return Ok(None);
            }
        }
        if q.after.is_some() || q.before.is_some() {
            let Some(mtime) = entry.mtime else {
                return Ok(None);
            };
            let t = (mtime.seconds, mtime.nanos);
            if q.after.is_some_and(|a| t < (a.seconds, a.nanos))
                || q.before.is_some_and(|b| t >= (b.seconds, b.nanos))
            {
                return Ok(None);
            }
        }
        let (size, blob) = match &entry.kind {
            EntryKind::File { size, blob } => (*size, *blob),
            _ if q.files_only() => return Ok(None),
            _ => return Ok(Some(None)),
        };
        if q.larger.is_some_and(|l| size < l) || q.smaller.is_some_and(|s| size >= s) {
            return Ok(None);
        }
        let Some(needle) = &self.text else {
            return Ok(Some(None));
        };
        // An empty file holds no text.
        let Some(blob) = blob else {
            return Ok(None);
        };
        if !self.texts.contains_key(&blob) {
            let text = if size > TEXT_LIMIT {
                self.out.skipped += 1;
                Text::TooLarge
            } else {
                let mut data = Vec::new();
                self.repo.read_file(&blob, &mut data)?;
                Text::Found(find_line(&data, needle))
            };
            self.texts.insert(blob, text);
        }
        Ok(match self.texts.get(&blob) {
            Some(Text::Found(Some(line))) => Some(Some(line.clone())),
            _ => None,
        })
    }
}

/// The first line of a text file holding `needle` (lowercase); `None` for a
/// file that is not text (SEARCH.md §3).
fn find_line(data: &[u8], needle: &str) -> Option<TextLine> {
    if data.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(data).ok()?;
    text.lines().enumerate().find_map(|(i, line)| {
        line.to_lowercase().contains(needle).then(|| TextLine {
            number: i as u64 + 1,
            text: line.trim_end().chars().take(LINE_CHARS).collect(),
        })
    })
}

/// Whether `subject` matches `pattern`, where `*` stands for any characters
/// and `?` for one.
fn wildcard(pattern: &[char], subject: &[char]) -> bool {
    let (mut p, mut s) = (0, 0);
    // Where the last `*` was, and the subject position it was tried at.
    let mut star: Option<(usize, usize)> = None;
    while s < subject.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, s));
                p += 1;
            }
            Some(&c) if c == '?' || c == subject[s] => {
                p += 1;
                s += 1;
            }
            _ => match star {
                Some((sp, ss)) => {
                    p = sp + 1;
                    s = ss + 1;
                    star = Some((sp, ss + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pattern: &str, subject: &str) -> bool {
        let p: Vec<char> = pattern.chars().collect();
        let s: Vec<char> = subject.chars().collect();
        wildcard(&p, &s)
    }

    #[test]
    fn wildcards() {
        assert!(m("*.txt", "a.txt"));
        assert!(m("*.txt", ".txt"));
        assert!(!m("*.txt", "a.txt.bak"));
        assert!(m("a?c", "abc"));
        assert!(!m("a?c", "ac"));
        assert!(m("*a*b*", "xxaxxbxx"));
        assert!(!m("*a*b", "xxbxxa"));
        assert!(m("docs/*.md", "docs/sub/x.md"));
        assert!(m("*", ""));
        assert!(!m("a", ""));
    }

    #[test]
    fn text_lines() {
        let line = find_line(b"first\nSecond Line\r\nsecond again\n", "second");
        assert_eq!(
            line,
            Some(TextLine {
                number: 2,
                text: "Second Line".to_owned()
            })
        );
        assert_eq!(find_line(b"bin\0ary second", "second"), None);
        assert_eq!(find_line(&[0xff, b's'], "s"), None);
    }
}
