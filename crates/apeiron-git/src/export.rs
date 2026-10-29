//! From snapshots to a Git repository (GIT.md §5).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use apeiron_core::{EntryKind, ObjectId, Repository, Snapshot};
use gix::ObjectId as GitId;
use gix::objs::Write as _;
use gix::refs::transaction::PreviousValue;

use crate::raw::{self, Entry, GITLINK_MODE, LINK_MODE, TREE_MODE};
use crate::{
    Error, ExportReport, OnObject, RefUpdate, Result, Skipped, check_git, git_error, git_name, keys,
};

/// Writes the snapshots of the refs under `prefix` into the Git repository
/// at `path`, created bare if it does not exist (GIT.md §5).
pub fn export(
    repo: &Repository,
    path: &Path,
    prefix: &str,
    on_object: OnObject<'_>,
) -> Result<ExportReport> {
    apeiron_core::validate_ref_name(prefix)?;
    let mut report = ExportReport::default();
    let own = format!("{prefix}/");
    let mut refs = Vec::new();
    for (name, snapshot) in repo.list_refs()? {
        if !name.starts_with(&own) {
            continue;
        }
        match git_name(prefix, &name) {
            Some(git) => refs.push((name, git, snapshot)),
            None => report.skipped.push(Skipped {
                name,
                reason: "not a branch, tag or remote-tracking branch".to_owned(),
            }),
        }
    }

    let created = !path.exists();
    let git = if created {
        gix::init_bare(path).map_err(|e| git_error(path, e))?
    } else {
        gix::open(path).map_err(|e| git_error(path, e))?
    };
    check_git(path, &git)?;

    let mut run = Run {
        repo,
        git: &git,
        path,
        written: HashMap::new(),
        trees: HashMap::new(),
        blobs: HashMap::new(),
        report: &mut report,
        done: 0,
    };
    for (_, _, snapshot) in &refs {
        run.export_snapshot(*snapshot, on_object)?;
    }
    let written = run.written;
    for (name, git_ref, snapshot) in refs {
        let id = written[&snapshot];
        git.reference(git_ref.as_str(), id, PreviousValue::Any, "apeiron export")
            .map_err(|e| git_error(path, e))?;
        report.refs.push(RefUpdate {
            git: git_ref,
            ref_name: name,
            id: id.to_string(),
            snapshot,
            previous: None,
        });
    }
    if created {
        set_head(&git, path, &report)?;
    }
    Ok(report)
}

/// Points `HEAD` of a new repository to `main`, else `master`, else the
/// first branch.
fn set_head(git: &gix::Repository, path: &Path, report: &ExportReport) -> Result<()> {
    let branches: Vec<&str> = report
        .refs
        .iter()
        .map(|r| r.git.as_str())
        .filter(|g| g.starts_with("refs/heads/"))
        .collect();
    let head = ["refs/heads/main", "refs/heads/master"]
        .into_iter()
        .find(|b| branches.contains(b))
        .or_else(|| branches.first().copied());
    if let Some(head) = head {
        let file = git.git_dir().join("HEAD");
        fs::write(&file, format!("ref: {head}\n")).map_err(|e| git_error(path, e))?;
    }
    Ok(())
}

struct Run<'a> {
    repo: &'a Repository,
    git: &'a gix::Repository,
    path: &'a Path,
    /// Snapshot → the commit or tag written for it.
    written: HashMap<ObjectId, GitId>,
    /// Apeiron tree without submodules below it → Git tree.
    trees: HashMap<ObjectId, GitId>,
    blobs: HashMap<Option<ObjectId>, GitId>,
    report: &'a mut ExportReport,
    done: u64,
}

fn meta<'a>(snapshot: &'a Snapshot, key: &str) -> Option<&'a str> {
    snapshot
        .meta
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

impl Run<'_> {
    fn write(&self, kind: gix::object::Kind, bytes: &[u8]) -> Result<GitId> {
        self.git
            .write_buf(kind, bytes)
            .map_err(|e| git_error(self.path, e))
    }

    fn invalid(id: ObjectId, reason: &str) -> Error {
        Error::InvalidObject {
            id: id.to_hex(),
            reason: reason.to_owned(),
        }
    }

    /// Writes `start` and what it reaches, each after its parents.
    fn export_snapshot(&mut self, start: ObjectId, on_object: OnObject<'_>) -> Result<()> {
        let mut stack = vec![(start, false)];
        let mut loaded: HashMap<ObjectId, Snapshot> = HashMap::new();
        let mut entered = HashSet::new();
        while let Some((id, expanded)) = stack.pop() {
            if self.written.contains_key(&id) {
                continue;
            }
            if !expanded {
                if !entered.insert(id) {
                    continue;
                }
                let snapshot = self.repo.read_snapshot(&id)?;
                // Already in Git: so is everything it reaches.
                let present = meta(&snapshot, keys::ID)
                    .and_then(|hex| GitId::from_hex(hex.as_bytes()).ok())
                    .filter(|git_id| self.git.has_object(git_id));
                if let Some(git_id) = present {
                    self.written.insert(id, git_id);
                    continue;
                }
                stack.push((id, true));
                stack.extend(snapshot.parents.iter().map(|p| (*p, false)));
                loaded.insert(id, snapshot);
                continue;
            }
            let snapshot = loaded
                .remove(&id)
                .ok_or_else(|| Self::invalid(id, "visited twice"))?;
            let git_id = self.write_snapshot(id, &snapshot)?;
            self.written.insert(id, git_id);
            self.done += 1;
            on_object(self.done)?;
        }
        Ok(())
    }

    fn write_snapshot(&mut self, id: ObjectId, snapshot: &Snapshot) -> Result<GitId> {
        let parents = snapshot
            .parents
            .iter()
            .map(|p| {
                self.written
                    .get(p)
                    .copied()
                    .ok_or_else(|| Self::invalid(id, "a parent was not written"))
            })
            .collect::<Result<Vec<_>>>()?;
        let headers = match (
            meta(snapshot, keys::HEADERS),
            meta(snapshot, keys::HEADERS_HEX),
        ) {
            (Some(text), _) => text.as_bytes().to_vec(),
            (None, Some(hex)) => {
                raw::from_hex(hex).ok_or_else(|| Self::invalid(id, "bad git.headers-hex"))?
            }
            (None, None) => synthetic_headers(snapshot),
        };
        let message: Option<Vec<u8>> = if meta(snapshot, keys::NO_MESSAGE).is_some() {
            None
        } else if let Some(hex) = meta(snapshot, keys::MESSAGE_HEX) {
            Some(raw::from_hex(hex).ok_or_else(|| Self::invalid(id, "bad git.message-hex"))?)
        } else {
            let mut text = snapshot.message.clone().unwrap_or_default();
            if meta(snapshot, keys::ID).is_none() && !text.ends_with('\n') {
                text.push('\n');
            }
            Some(text.into_bytes())
        };

        let (kind, bytes) = if meta(snapshot, keys::KIND) == Some("tag") {
            let [target] = parents.as_slice() else {
                return Err(Self::invalid(id, "a tag needs exactly one parent"));
            };
            self.report.tags += 1;
            let bytes = raw::write_tag(&target.to_string(), &headers, message.as_deref());
            (gix::object::Kind::Tag, bytes)
        } else {
            let gitlinks = parse_gitlinks(meta(snapshot, keys::GITLINKS))
                .ok_or_else(|| Self::invalid(id, "bad git.gitlinks"))?;
            let tree = self.write_tree(snapshot.tree, b"", &gitlinks)?;
            let parents: Vec<String> = parents.iter().map(ToString::to_string).collect();
            self.report.commits += 1;
            let bytes =
                raw::write_commit(&tree.to_string(), &parents, &headers, message.as_deref());
            (gix::object::Kind::Commit, bytes)
        };
        let git_id = self.write(kind, &bytes)?;
        if let Some(expected) = meta(snapshot, keys::ID) {
            if git_id.to_string() != expected {
                self.report.inexact.push(expected.to_owned());
            }
        }
        Ok(git_id)
    }

    /// Writes the tree at `path` (empty for the root), with the submodule
    /// entries that belong in it.
    fn write_tree(
        &mut self,
        tree: ObjectId,
        path: &[u8],
        gitlinks: &HashMap<Vec<u8>, GitId>,
    ) -> Result<GitId> {
        let inside =
            |p: &[u8]| path.is_empty() || p.starts_with(path) && p.get(path.len()) == Some(&b'/');
        let plain = !gitlinks.keys().any(|p| inside(p));
        if plain {
            if let Some(id) = self.trees.get(&tree) {
                return Ok(*id);
            }
        }
        let mut entries = Vec::new();
        for entry in self.repo.read_tree(&tree)?.entries {
            let child_path = join(path, &entry.name);
            let (mode, id) = match &entry.kind {
                EntryKind::Directory { tree } => (
                    TREE_MODE.to_vec(),
                    self.write_tree(*tree, &child_path, gitlinks)?,
                ),
                EntryKind::File { blob, .. } => (file_mode(entry.mode), self.write_blob(*blob)?),
                EntryKind::Symlink { target } => (
                    LINK_MODE.to_vec(),
                    self.write(gix::object::Kind::Blob, target)?,
                ),
            };
            entries.push(Entry {
                mode,
                name: entry.name,
                id: id.as_slice().to_vec(),
            });
        }
        for (link, id) in gitlinks {
            let Some(name) = link.strip_prefix(path) else {
                continue;
            };
            let name = if path.is_empty() {
                name
            } else {
                match name.strip_prefix(b"/") {
                    Some(name) => name,
                    None => continue,
                }
            };
            if !name.contains(&b'/') {
                entries.push(Entry {
                    mode: GITLINK_MODE.to_vec(),
                    name: name.to_vec(),
                    id: id.as_slice().to_vec(),
                });
            }
        }
        let id = self.write(gix::object::Kind::Tree, &raw::write_tree(entries))?;
        if plain {
            self.trees.insert(tree, id);
        }
        Ok(id)
    }

    fn write_blob(&mut self, blob: Option<ObjectId>) -> Result<GitId> {
        if let Some(id) = self.blobs.get(&blob) {
            return Ok(*id);
        }
        let mut content = Vec::new();
        if let Some(blob) = &blob {
            self.repo.read_file(blob, &mut content)?;
        }
        let id = self.write(gix::object::Kind::Blob, &content)?;
        self.blobs.insert(blob, id);
        Ok(id)
    }
}

fn join(path: &[u8], name: &[u8]) -> Vec<u8> {
    let mut out = path.to_vec();
    if !out.is_empty() {
        out.push(b'/');
    }
    out.extend_from_slice(name);
    out
}

/// `100644`, `100755` or the old `100664` as imported; other permission
/// bits, as a backup records them, become one of the first two.
fn file_mode(mode: Option<u16>) -> Vec<u8> {
    let bits = match mode {
        Some(m @ (0o644 | 0o755 | 0o664)) => m,
        Some(m) if m & 0o100 != 0 => 0o755,
        _ => 0o644,
    };
    format!("100{bits:o}").into_bytes()
}

/// Lines `<hex path> <commit id>`; `None` if one is malformed.
fn parse_gitlinks(value: Option<&str>) -> Option<HashMap<Vec<u8>, GitId>> {
    let mut out = HashMap::new();
    for line in value.unwrap_or("").lines().filter(|l| !l.is_empty()) {
        let (path, id) = line.split_once(' ')?;
        out.insert(raw::from_hex(path)?, GitId::from_hex(id.as_bytes()).ok()?);
    }
    Some(out)
}

/// Author and committer lines for a snapshot not made from Git.
fn synthetic_headers(snapshot: &Snapshot) -> Vec<u8> {
    let clean = |s: &str| s.replace(['\n', '<', '>'], " ").trim().to_owned();
    let who = match snapshot.author.as_deref() {
        Some(a) if a.contains('<') && a.ends_with('>') && !a.contains('\n') => a.to_owned(),
        Some(a) if !clean(a).is_empty() => format!("{} <apeiron@localhost>", clean(a)),
        _ => "Apeiron <apeiron@localhost>".to_owned(),
    };
    let seconds = snapshot.time.seconds;
    format!("author {who} {seconds} +0000\ncommitter {who} {seconds} +0000\n").into_bytes()
}
