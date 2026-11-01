//! Taking `git push` (GIT.md §7): the objects of a pack go into the Git
//! copy, its commits become snapshots, and refs move forward.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use apeiron_core::Repository;
use gix::ObjectId as GitId;
use gix::objs::Write as _;
use serde::{Deserialize, Serialize};

use crate::import::{Run, Wanted, known_snapshots, store};
use crate::{Error, Result, apeiron_name, delta, export, git_error, open_git, raw};

/// One ref a client asks to change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushedRef {
    /// The full Git name, such as `refs/heads/main`.
    pub git: String,
    /// The Git id the client last saw, 40 zeros for a new ref.
    pub old: String,
    /// The Git id it is to point to.
    pub new: String,
}

/// What became of a [`PushedRef`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushResult {
    pub git: String,
    /// The Apeiron ref, when the Git name maps to one.
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
    /// Why the ref did not move; `None` when it moved.
    pub error: Option<String>,
}

/// Stores the objects of `pack` in the Git copy at `path`, the commits the
/// pushed refs reach as snapshots, and moves each ref under `prefix` that
/// `may` allows, that moves forward and that still is where the client saw
/// it. The caller holds the repository lock shared.
pub fn receive(
    repo: &mut Repository,
    path: &Path,
    prefix: &str,
    updates: &[PushedRef],
    pack: &[u8],
    may: &dyn Fn(&str) -> bool,
) -> Result<Vec<PushResult>> {
    // Where each ref is now, and the Git copy brought up to date with it,
    // so the ids the client saw can be checked against the copy.
    let mut before = Vec::new();
    for update in updates {
        before.push(match apeiron_name(prefix, update.git.as_bytes()) {
            Some(Ok(name)) => repo.read_ref(&name)?,
            _ => None,
        });
    }
    export(repo, path, prefix, &mut |_| Ok(()))?;
    let git = open_git(path)?;
    unpack(&git, path, pack)?;
    let known = known_snapshots(repo, prefix)?;

    let mut results = Vec::new();
    let mut wanted = Vec::new();
    for (update, before) in updates.iter().zip(before) {
        let name = match apeiron_name(prefix, update.git.as_bytes()) {
            Some(Ok(name)) if !update.git.starts_with("refs/remotes/") => Some(name),
            _ => None,
        };
        let checked = match &name {
            Some(name) => check(repo, &git, path, before, update, name, may),
            None => Err("only branches and tags are taken".to_owned()),
        };
        match (checked, name) {
            (Ok(id), Some(name)) => wanted.push((
                Wanted {
                    git: update.git.clone(),
                    name,
                    id,
                },
                before,
            )),
            (Err(reason), name) => results.push(PushResult {
                git: update.git.clone(),
                ref_name: name,
                error: Some(reason),
            }),
            (Ok(_), None) => {}
        }
    }

    let mut run = Run::new(&git, path, known);
    let only: Vec<Wanted> = wanted.iter().map(|(w, _)| w.clone()).collect();
    let order = run.missing(&only)?;
    store(repo, &mut run, &order, &mut |_| Ok(()))?;
    for (w, previous) in wanted {
        let error = match run.stored(w.id) {
            None => Some("its commit was not stored".to_owned()),
            Some(snapshot) => match repo.update_ref(&w.name, previous, Some(snapshot)) {
                Ok(()) => None,
                Err(apeiron_core::Error::RefConflict { .. }) => {
                    Some("the ref moved meanwhile; fetch first".to_owned())
                }
                Err(e) => Some(e.to_string()),
            },
        };
        results.push(PushResult {
            git: w.git,
            ref_name: Some(w.name),
            error,
        });
    }
    // In the order the client sent them.
    results.sort_by_key(|r| updates.iter().position(|u| u.git == r.git));
    Ok(results)
}

/// The commit or tag a ref may move to, or why it may not.
fn check(
    repo: &Repository,
    git: &gix::Repository,
    path: &Path,
    before: Option<apeiron_core::ObjectId>,
    update: &PushedRef,
    name: &str,
    may: &dyn Fn(&str) -> bool,
) -> std::result::Result<GitId, String> {
    if !may(name) {
        return Err(format!("you may not write {name}"));
    }
    let new = GitId::from_hex(update.new.as_bytes()).map_err(|_| "bad id".to_owned())?;
    let old = GitId::from_hex(update.old.as_bytes()).map_err(|_| "bad id".to_owned())?;
    if new.is_null() {
        return Err("deleting refs is not taken; use apeiron ref delete".to_owned());
    }
    let current = repo.read_ref(name).map_err(|e| e.to_string())?;
    let in_copy = git
        .try_find_reference(update.git.as_str())
        .ok()
        .flatten()
        .and_then(|r| r.target().try_id().map(|id| id.to_owned()));
    let seen = (!old.is_null()).then_some(old);
    if current != before || in_copy != seen {
        return Err("the ref is not where you saw it; fetch first".to_owned());
    }
    let object = git
        .find_object(new)
        .map_err(|_| "its object was not sent".to_owned())?;
    let commit = match object.kind {
        gix::object::Kind::Commit => new,
        gix::object::Kind::Tag => {
            let tag = raw::parse_tag(&object.data).ok_or("a broken tag")?;
            if raw::header(tag.headers, b"type") != Some(b"commit") {
                return Err("a tag of something other than a commit".to_owned());
            }
            GitId::from_hex(tag.object).map_err(|_| "a broken tag".to_owned())?
        }
        _ => return Err("only commits and tags are taken".to_owned()),
    };
    if !old.is_null() && !reaches(git, path, commit, old).map_err(|e| e.to_string())? {
        return Err("not a fast-forward; the history on the server is kept".to_owned());
    }
    Ok(new)
}

/// Whether `old` is `from` or one of its ancestors; `old` may be a tag.
fn reaches(git: &gix::Repository, path: &Path, from: GitId, old: GitId) -> Result<bool> {
    let mut target = old;
    let object = git.find_object(old).map_err(|e| git_error(path, e))?;
    if object.kind == gix::object::Kind::Tag
        && let Some(id) = raw::parse_tag(&object.data).and_then(|t| GitId::from_hex(t.object).ok())
    {
        target = id;
    }
    let mut seen = HashSet::new();
    let mut stack = vec![from];
    while let Some(id) = stack.pop() {
        if id == target {
            return Ok(true);
        }
        if !seen.insert(id) {
            continue;
        }
        let object = git.find_object(id).map_err(|e| git_error(path, e))?;
        if let Some(commit) = raw::parse_commit(&object.data) {
            for parent in commit.parents {
                if let Ok(p) = GitId::from_hex(parent) {
                    stack.push(p);
                }
            }
        }
    }
    Ok(false)
}

/// Writes every object of a pack, version 2 or 3, into the Git copy;
/// deltas are made whole against their base, in the pack or in the copy.
fn unpack(git: &gix::Repository, path: &Path, pack: &[u8]) -> Result<u64> {
    let broken = |reason: &str| Error::InvalidObject {
        id: "pack".to_owned(),
        reason: reason.to_owned(),
    };
    if pack.is_empty() {
        return Ok(0);
    }
    if pack.len() < 32 || &pack[..4] != b"PACK" || !matches!(pack[7], 2 | 3) {
        return Err(broken("not a Git pack"));
    }
    let (body, trailer) = pack.split_at(pack.len() - 20);
    let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha1);
    hasher.update(body);
    let digest = hasher.try_finalize().map_err(|e| git_error(path, e))?;
    if digest.as_bytes() != trailer {
        return Err(broken("its checksum is wrong"));
    }
    let count = u32::from_be_bytes([pack[8], pack[9], pack[10], pack[11]]);

    enum Base {
        Offset(usize),
        Id(GitId),
    }
    let mut at_offset: HashMap<usize, GitId> = HashMap::new();
    let mut waiting: Vec<(usize, Base, Vec<u8>)> = Vec::new();
    let mut pos = 12;
    for _ in 0..count {
        let start = pos;
        let mut byte = *body.get(pos).ok_or_else(|| broken("cut short"))?;
        pos += 1;
        let kind_bits = (byte >> 4) & 7;
        let mut size = usize::from(byte & 0x0f);
        let mut shift = 4;
        while byte & 0x80 != 0 {
            byte = *body.get(pos).ok_or_else(|| broken("cut short"))?;
            pos += 1;
            size |= usize::from(byte & 0x7f)
                .checked_shl(shift)
                .ok_or_else(|| broken("bad size"))?;
            shift += 7;
        }
        let base = match kind_bits {
            6 => {
                let mut byte = *body.get(pos).ok_or_else(|| broken("cut short"))?;
                pos += 1;
                let mut back = usize::from(byte & 0x7f);
                while byte & 0x80 != 0 {
                    byte = *body.get(pos).ok_or_else(|| broken("cut short"))?;
                    pos += 1;
                    back = ((back + 1) << 7) | usize::from(byte & 0x7f);
                }
                Some(Base::Offset(
                    start
                        .checked_sub(back)
                        .ok_or_else(|| broken("bad offset"))?,
                ))
            }
            7 => {
                let id = body.get(pos..pos + 20).ok_or_else(|| broken("cut short"))?;
                pos += 20;
                Some(Base::Id(
                    GitId::try_from(id).map_err(|_| broken("bad base id"))?,
                ))
            }
            _ => None,
        };
        let mut inflate = flate2::Decompress::new(true);
        let mut data = Vec::with_capacity(size.min(64 << 20));
        loop {
            let before = inflate.total_in();
            let status = inflate
                .decompress_vec(&body[pos..], &mut data, flate2::FlushDecompress::Finish)
                .map_err(|_| broken("bad compressed data"))?;
            pos += (inflate.total_in() - before) as usize;
            match status {
                flate2::Status::StreamEnd => break,
                _ if data.len() == data.capacity() => data.reserve(64 << 10),
                _ if inflate.total_in() == before => return Err(broken("cut short")),
                _ => {}
            }
        }
        if data.len() != size {
            return Err(broken("an object has the wrong size"));
        }
        match base {
            None => {
                let kind = match kind_bits {
                    1 => gix::object::Kind::Commit,
                    2 => gix::object::Kind::Tree,
                    3 => gix::object::Kind::Blob,
                    4 => gix::object::Kind::Tag,
                    _ => return Err(broken("an unknown object type")),
                };
                let id = git.write_buf(kind, &data).map_err(|e| git_error(path, e))?;
                at_offset.insert(start, id);
            }
            Some(base) => waiting.push((start, base, data)),
        }
    }
    // A delta may come before its base; go round until none is left.
    while !waiting.is_empty() {
        let before = waiting.len();
        let mut left = Vec::new();
        for (start, base, delta) in waiting {
            let base_id = match &base {
                Base::Offset(at) => at_offset.get(at).copied(),
                Base::Id(id) => Some(*id),
            };
            let found = base_id.and_then(|id| git.find_object(id).ok());
            let Some(found) = found else {
                left.push((start, base, delta));
                continue;
            };
            let whole =
                delta::apply(&found.data, &delta).ok_or_else(|| broken("a broken delta"))?;
            let id = git
                .write_buf(found.kind, &whole)
                .map_err(|e| git_error(path, e))?;
            at_offset.insert(start, id);
        }
        if left.len() == before {
            return Err(broken("a delta's base is missing"));
        }
        waiting = left;
    }
    Ok(u64::from(count))
}
