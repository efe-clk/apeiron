//! From a Git repository to snapshots (GIT.md §4).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use apeiron_core::{
    EntryKind, ObjectId, ObjectWriter, Repository, Snapshot, Time, Tree, TreeEntry,
};
use gix::ObjectId as GitId;

use crate::raw::{self, GITLINK_MODE, LINK_MODE, TREE_MODE};
use crate::{
    Error, ImportReport, OnObject, RefUpdate, Result, Skipped, apeiron_name, git_error, keys,
    open_git,
};

/// Commits stored per batch; each batch is made durable on its own, so an
/// interrupted import keeps most of its work.
const BATCH: usize = 1000;

/// What a ref of the Git repository names.
struct Wanted {
    git: String,
    name: String,
    /// The commit or annotated tag.
    id: GitId,
}

/// Stores every branch and tag of the Git repository at `path` under
/// `prefix` (GIT.md §4). The caller holds the repository lock shared.
pub fn import(
    repo: &mut Repository,
    path: &Path,
    prefix: &str,
    on_object: OnObject<'_>,
) -> Result<ImportReport> {
    apeiron_core::validate_ref_name(prefix)?;
    let git = open_git(path)?;
    let mut report = ImportReport::default();
    let wanted = wanted_refs(&git, path, prefix, &mut report)?;

    let mut run = Run {
        git: &git,
        path,
        known: known_snapshots(repo, prefix)?,
        blobs: HashMap::new(),
        trees: HashMap::new(),
        done: 0,
    };
    // Parents before children, in batches.
    let order = run.missing(&wanted)?;
    for batch in order.chunks(BATCH) {
        let mut writer = repo.writer();
        for &(id, is_tag) in batch {
            let snapshot = if is_tag {
                run.store_tag(&mut writer, id)?
            } else {
                run.store_commit(&mut writer, id)?
            };
            run.known.insert(id, snapshot);
            if is_tag {
                report.tags += 1;
            } else {
                report.commits += 1;
            }
            run.done += 1;
            on_object(run.done)?;
        }
        writer.finish()?;
    }

    // Refs last, so none points to anything incomplete.
    let mut seen = HashSet::new();
    for w in wanted {
        seen.insert(w.name.clone());
        let (snapshot, _) = *run.known.get(&w.id).ok_or_else(|| Error::InvalidObject {
            id: w.id.to_string(),
            reason: "it was not stored".to_owned(),
        })?;
        let previous = repo.read_ref(&w.name)?;
        if previous == Some(snapshot) {
            continue;
        }
        match repo.update_ref(&w.name, previous, Some(snapshot)) {
            Ok(()) => report.refs.push(RefUpdate {
                git: w.git,
                ref_name: w.name,
                id: w.id.to_string(),
                snapshot,
                previous,
            }),
            Err(apeiron_core::Error::RefNameConflict(_)) => report.skipped.push(Skipped {
                name: w.git,
                reason: format!("{} clashes with another ref", w.name),
            }),
            Err(e) => return Err(e.into()),
        }
    }
    let own = format!("{prefix}/");
    report.stale = repo
        .list_refs()?
        .into_iter()
        .map(|(name, _)| name)
        .filter(|n| n.starts_with(&own) && !seen.contains(n))
        .collect();
    Ok(report)
}

/// The branches and tags to import; everything else is listed as skipped.
fn wanted_refs(
    git: &gix::Repository,
    path: &Path,
    prefix: &str,
    report: &mut ImportReport,
) -> Result<Vec<Wanted>> {
    let mut wanted = Vec::new();
    let platform = git.references().map_err(|e| git_error(path, e))?;
    for r in platform.all().map_err(|e| git_error(path, e))? {
        let r = r.map_err(|e| git_error(path, e))?;
        let full = r.name().as_bstr().to_vec();
        let shown = String::from_utf8_lossy(&full).into_owned();
        let skip = |report: &mut ImportReport, reason: &str| {
            report.skipped.push(Skipped {
                name: shown.clone(),
                reason: reason.to_owned(),
            });
        };
        let name = match apeiron_name(prefix, &full) {
            None => {
                skip(report, "not a branch, tag or remote-tracking branch");
                continue;
            }
            Some(Err(reason)) => {
                skip(report, &reason);
                continue;
            }
            Some(Ok(name)) => name,
        };
        let Some(id) = r.target().try_id().map(|id| id.to_owned()) else {
            skip(report, "a symbolic ref");
            continue;
        };
        let object = git.find_object(id).map_err(|e| git_error(path, e))?;
        match object.kind {
            gix::object::Kind::Commit => {}
            gix::object::Kind::Tag => {
                let target = raw::parse_tag(&object.data)
                    .and_then(|t| raw::header(t.headers, b"type").map(|ty| ty == b"commit"));
                if target != Some(true) {
                    skip(report, "a tag of something other than a commit");
                    continue;
                }
            }
            _ => {
                skip(report, "points to a tree or a blob");
                continue;
            }
        }
        wanted.push(Wanted {
            git: shown,
            name,
            id,
        });
    }
    Ok(wanted)
}

/// Git ids already stored: the `git.id` of every snapshot that the refs
/// under `prefix` reach, with the snapshot and its tree.
fn known_snapshots(
    repo: &Repository,
    prefix: &str,
) -> Result<HashMap<GitId, (ObjectId, ObjectId)>> {
    let own = format!("{prefix}/");
    let mut known = HashMap::new();
    let mut seen = HashSet::new();
    let mut stack: Vec<ObjectId> = repo
        .list_refs()?
        .into_iter()
        .filter(|(name, _)| name.starts_with(&own))
        .map(|(_, id)| id)
        .collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let snapshot = repo.read_snapshot(&id)?;
        let git_id = snapshot
            .meta
            .iter()
            .find(|(k, _)| k == keys::ID)
            .and_then(|(_, v)| GitId::from_hex(v.as_bytes()).ok());
        if let Some(git_id) = git_id {
            known.insert(git_id, (id, snapshot.tree));
        }
        stack.extend(snapshot.parents);
    }
    Ok(known)
}

/// Submodule entries below a tree: path and commit id.
type Gitlinks = Vec<(Vec<u8>, String)>;

struct Run<'a> {
    git: &'a gix::Repository,
    path: &'a Path,
    /// Git commit or tag → its snapshot and that snapshot's tree.
    known: HashMap<GitId, (ObjectId, ObjectId)>,
    /// Git blob → size and Apeiron blob, for this run.
    blobs: HashMap<GitId, (u64, Option<ObjectId>)>,
    /// Git tree → Apeiron tree and the submodule entries below it.
    trees: HashMap<GitId, (ObjectId, Gitlinks)>,
    done: u64,
}

impl Run<'_> {
    fn read(&self, id: GitId) -> Result<(gix::object::Kind, Vec<u8>)> {
        let object = self
            .git
            .find_object(id)
            .map_err(|e| git_error(self.path, e))?;
        Ok((object.kind, object.data.clone()))
    }

    fn invalid(id: impl ToString, reason: &str) -> Error {
        Error::InvalidObject {
            id: id.to_string(),
            reason: reason.to_owned(),
        }
    }

    fn parse_id(&self, hex: &[u8]) -> Result<GitId> {
        GitId::from_hex(hex).map_err(|_| Self::invalid(String::from_utf8_lossy(hex), "bad id"))
    }

    /// The commits and tags not stored yet, each after what it points to.
    /// `true` marks a tag.
    fn missing(&self, wanted: &[Wanted]) -> Result<Vec<(GitId, bool)>> {
        let mut order = Vec::new();
        let mut placed = HashSet::new();
        // (id, is_tag, children pushed)
        let mut stack: Vec<(GitId, bool, bool)> = Vec::new();
        for w in wanted {
            let (kind, _) = self.read(w.id)?;
            stack.push((w.id, kind == gix::object::Kind::Tag, false));
        }
        while let Some((id, is_tag, expanded)) = stack.pop() {
            if self.known.contains_key(&id) || placed.contains(&id) {
                continue;
            }
            if expanded {
                placed.insert(id);
                order.push((id, is_tag));
                continue;
            }
            stack.push((id, is_tag, true));
            let (_, data) = self.read(id)?;
            if is_tag {
                let tag = raw::parse_tag(&data).ok_or_else(|| Self::invalid(id, "bad tag"))?;
                stack.push((self.parse_id(tag.object)?, false, false));
            } else {
                let commit =
                    raw::parse_commit(&data).ok_or_else(|| Self::invalid(id, "bad commit"))?;
                for parent in commit.parents {
                    stack.push((self.parse_id(parent)?, false, false));
                }
            }
        }
        Ok(order)
    }

    fn snapshot_of(&self, id: GitId) -> Result<(ObjectId, ObjectId)> {
        self.known
            .get(&id)
            .copied()
            .ok_or_else(|| Self::invalid(id, "stored before what it points to"))
    }

    fn store_commit(
        &mut self,
        writer: &mut ObjectWriter<'_>,
        id: GitId,
    ) -> Result<(ObjectId, ObjectId)> {
        let (_, data) = self.read(id)?;
        let commit = raw::parse_commit(&data).ok_or_else(|| Self::invalid(id, "bad commit"))?;
        let (tree, gitlinks) = self.store_tree(writer, self.parse_id(commit.tree)?)?;
        let parents = commit
            .parents
            .iter()
            .map(|p| Ok(self.snapshot_of(self.parse_id(p)?)?.0))
            .collect::<Result<Vec<_>>>()?;
        let (author, _) = raw::signature(raw::header(commit.headers, b"author").unwrap_or(b""));
        let (_, seconds) = raw::signature(raw::header(commit.headers, b"committer").unwrap_or(b""));
        let mut snapshot = new_snapshot(id, "commit", commit.headers, commit.message);
        snapshot.tree = tree;
        snapshot.parents = parents;
        snapshot.time.seconds = seconds;
        snapshot.author = Some(author);
        if !gitlinks.is_empty() {
            let lines: Vec<String> = gitlinks
                .iter()
                .map(|(path, id)| format!("{} {id}", raw::to_hex(path)))
                .collect();
            snapshot
                .meta
                .push((keys::GITLINKS.to_owned(), lines.join("\n")));
            snapshot.meta.sort();
        }
        Ok((writer.put_snapshot(&snapshot)?, tree))
    }

    fn store_tag(
        &mut self,
        writer: &mut ObjectWriter<'_>,
        id: GitId,
    ) -> Result<(ObjectId, ObjectId)> {
        let (_, data) = self.read(id)?;
        let tag = raw::parse_tag(&data).ok_or_else(|| Self::invalid(id, "bad tag"))?;
        let (target, tree) = self.snapshot_of(self.parse_id(tag.object)?)?;
        let (tagger, seconds) = raw::header(tag.headers, b"tagger")
            .map(raw::signature)
            .unwrap_or_default();
        let mut snapshot = new_snapshot(id, "tag", tag.headers, tag.message);
        snapshot.tree = tree;
        snapshot.parents = vec![target];
        snapshot.time.seconds = seconds;
        snapshot.author = (!tagger.is_empty()).then_some(tagger);
        Ok((writer.put_snapshot(&snapshot)?, tree))
    }

    /// Stores a Git tree; returns it with the submodule entries below it as
    /// (path, commit id).
    fn store_tree(
        &mut self,
        writer: &mut ObjectWriter<'_>,
        id: GitId,
    ) -> Result<(ObjectId, Gitlinks)> {
        if let Some(stored) = self.trees.get(&id) {
            return Ok(stored.clone());
        }
        let (_, data) = self.read(id)?;
        let hash_len = id.as_slice().len();
        let entries =
            raw::parse_tree(&data, hash_len).ok_or_else(|| Self::invalid(id, "bad tree"))?;
        let mut out = Vec::with_capacity(entries.len());
        let mut gitlinks = Vec::new();
        for e in entries {
            let child =
                GitId::try_from(e.id.as_slice()).map_err(|_| Self::invalid(id, "bad entry id"))?;
            let (kind, mode) = match e.mode.as_slice() {
                TREE_MODE => {
                    let (tree, below) = self.store_tree(writer, child)?;
                    for (path, commit) in below {
                        let mut full = e.name.clone();
                        full.push(b'/');
                        full.extend_from_slice(&path);
                        gitlinks.push((full, commit));
                    }
                    (EntryKind::Directory { tree }, None)
                }
                GITLINK_MODE => {
                    gitlinks.push((e.name.clone(), child.to_string()));
                    continue;
                }
                LINK_MODE => {
                    let (_, target) = self.read(child)?;
                    (EntryKind::Symlink { target }, None)
                }
                mode if mode.starts_with(b"100") => {
                    let bits = std::str::from_utf8(&mode[3..])
                        .ok()
                        .and_then(|m| u16::from_str_radix(m, 8).ok())
                        .ok_or_else(|| Self::invalid(id, "bad file mode"))?;
                    let (size, blob) = self.store_blob(writer, child)?;
                    (EntryKind::File { size, blob }, Some(bits))
                }
                _ => return Err(Self::invalid(id, "unknown entry mode")),
            };
            out.push(TreeEntry {
                name: e.name,
                kind,
                mode,
                mtime: None,
                uid: None,
                gid: None,
                xattrs: Vec::new(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        let tree = writer.put_tree(&Tree { entries: out })?;
        self.trees.insert(id, (tree, gitlinks.clone()));
        Ok((tree, gitlinks))
    }

    fn store_blob(
        &mut self,
        writer: &mut ObjectWriter<'_>,
        id: GitId,
    ) -> Result<(u64, Option<ObjectId>)> {
        if let Some(stored) = self.blobs.get(&id) {
            return Ok(*stored);
        }
        let (_, data) = self.read(id)?;
        let stored = writer.put_file(data.as_slice())?;
        self.blobs.insert(id, stored);
        Ok(stored)
    }
}

/// A snapshot with the `meta` keys every Git object gets; tree, parents,
/// time and author are filled in by the caller.
fn new_snapshot(id: GitId, kind: &str, headers: &[u8], message: Option<&[u8]>) -> Snapshot {
    let mut meta = vec![
        (keys::ID.to_owned(), id.to_string()),
        (keys::KIND.to_owned(), kind.to_owned()),
    ];
    match std::str::from_utf8(headers) {
        Ok(text) => meta.push((keys::HEADERS.to_owned(), text.to_owned())),
        Err(_) => meta.push((keys::HEADERS_HEX.to_owned(), raw::to_hex(headers))),
    }
    let message = match message {
        None => {
            meta.push((keys::NO_MESSAGE.to_owned(), "1".to_owned()));
            None
        }
        Some(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => Some(text.to_owned()),
            Err(_) => {
                meta.push((keys::MESSAGE_HEX.to_owned(), raw::to_hex(bytes)));
                None
            }
        },
    };
    meta.sort();
    Snapshot {
        tree: ObjectId::from_bytes([0; 32]),
        parents: Vec::new(),
        time: Time {
            seconds: 0,
            nanos: 0,
        },
        author: None,
        host: None,
        message,
        meta,
    }
}
