//! Serving refs to `git clone` and `git fetch` (GIT.md §7): a Git copy
//! kept up to date by export, its refs, and packs of what they reach.

use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::Path;

use apeiron_core::Repository;
use gix::ObjectId;
use serde::{Deserialize, Serialize};

use crate::delta;
use crate::raw::{self, GITLINK_MODE};
use crate::{Error, Result, export, git_error, git_name, open_git};

/// A ref offered to Git clients.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServedRef {
    /// The full Git name, such as `refs/heads/main`.
    pub git: String,
    /// The Apeiron ref, such as `git/heads/main`.
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// The Git object id, 40 hex digits.
    pub id: String,
}

/// Brings the Git copy at `path` up to date with the branches and tags
/// under `prefix`, creating it bare if needed, and lists them with their
/// Git ids, sorted by Git name.
pub fn serve_refs(repo: &Repository, path: &Path, prefix: &str) -> Result<Vec<ServedRef>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| git_error(path, e))?;
    }
    export(repo, path, prefix, &mut |_| Ok(()))?;
    let git = open_git(path)?;
    let mut out = Vec::new();
    for (name, _) in repo.list_refs()? {
        let Some(git_ref) = git_name(prefix, &name) else {
            continue;
        };
        if !git_ref.starts_with("refs/heads/") && !git_ref.starts_with("refs/tags/") {
            continue;
        }
        // A ref export left out, such as one with a broken object, is not
        // offered.
        let Ok(Some(found)) = git.try_find_reference(git_ref.as_str()) else {
            continue;
        };
        if let Some(id) = found.target().try_id() {
            out.push(ServedRef {
                git: git_ref,
                ref_name: name,
                id: id.to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.git.cmp(&b.git));
    Ok(out)
}

/// The ids of `haves`, in their order, that are commits `wants` reach in
/// the Git copy at `path`: what the client and the server have in common.
/// Others, even if the copy holds them, do not count, so the answer tells
/// nothing about refs the client was not offered.
pub fn common_haves(path: &Path, wants: &[String], haves: &[String]) -> Result<Vec<String>> {
    let git = open_git(path)?;
    let starts = parse_ids(wants)?;
    let commits: HashSet<ObjectId> = reach(&git, path, &starts, &HashSet::new(), true)?
        .into_iter()
        .map(|r| r.id)
        .collect();
    Ok(haves
        .iter()
        .filter(|h| parse_id(h).is_ok_and(|id| commits.contains(&id)))
        .cloned()
        .collect())
}

/// Writes a Git pack, version 2 and without deltas, of every object `wants`
/// reach in the Git copy at `path` and `common` does not; returns the
/// number of objects. The caller checks that the client may have each of
/// `wants`, and takes `common` from [`common_haves`].
pub fn write_pack(
    path: &Path,
    wants: &[String],
    common: &[String],
    out: &mut dyn Write,
) -> Result<u64> {
    let git = open_git(path)?;
    let known: HashSet<ObjectId> = reach(&git, path, &parse_ids(common)?, &HashSet::new(), false)?
        .into_iter()
        .map(|r| r.id)
        .collect();
    let mut order = reach(&git, path, &parse_ids(wants)?, &known, false)?;
    // Trees and files of the same name, largest first: a changed version
    // is then usually next to the one it is written against, as Git does.
    let deltas =
        |k: gix::object::Kind| matches!(k, gix::object::Kind::Tree | gix::object::Kind::Blob);
    order.sort_by(|a, b| {
        (
            deltas(a.kind),
            a.kind as u8,
            &a.name,
            std::cmp::Reverse(a.size),
        )
            .cmp(&(
                deltas(b.kind),
                b.kind as u8,
                &b.name,
                std::cmp::Reverse(b.size),
            ))
    });

    let mut pack = Hashed {
        out,
        hasher: gix::hash::hasher(gix::hash::Kind::Sha1),
    };
    let io = |e: std::io::Error| git_error(path, e);
    pack.write_all(b"PACK").map_err(io)?;
    pack.write_all(&2u32.to_be_bytes()).map_err(io)?;
    let count = u32::try_from(order.len())
        .map_err(|_| Error::Unsupported("too many objects for one pack".to_owned()))?;
    pack.write_all(&count.to_be_bytes()).map_err(io)?;
    let mut window: VecDeque<Candidate> = VecDeque::new();
    for reached in &order {
        let object = git
            .find_object(reached.id)
            .map_err(|e| git_error(path, e))?;
        let data = &object.data;
        let mut best: Option<(ObjectId, Vec<u8>, u32)> = None;
        if deltas(reached.kind) && data.len() >= MIN_DELTA {
            for base in window
                .iter()
                .filter(|c| c.kind == reached.kind && c.depth < MAX_DEPTH)
            {
                let limit = best
                    .as_ref()
                    .map_or(data.len() / 2, |(_, d, _)| d.len().saturating_sub(1));
                if let Some(d) = delta::encode(&base.data, &base.index, data, limit) {
                    best = Some((base.id, d, base.depth + 1));
                }
            }
        }
        let z = |pack: &mut Hashed<'_>, bytes: &[u8]| -> Result<()> {
            let mut z = flate2::write::ZlibEncoder::new(pack, flate2::Compression::default());
            z.write_all(bytes).map_err(io)?;
            z.finish().map_err(io)?;
            Ok(())
        };
        let depth = match best {
            Some((base, d, depth)) => {
                pack.write_all(&entry_header(REF_DELTA, d.len() as u64))
                    .map_err(io)?;
                pack.write_all(base.as_bytes()).map_err(io)?;
                z(&mut pack, &d)?;
                depth
            }
            None => {
                pack.write_all(&entry_header(type_bits(reached.kind), data.len() as u64))
                    .map_err(io)?;
                z(&mut pack, data)?;
                0
            }
        };
        if deltas(reached.kind) && data.len() <= MAX_WINDOW_OBJECT {
            if window.len() == WINDOW {
                window.pop_front();
            }
            window.push_back(Candidate {
                id: reached.id,
                kind: reached.kind,
                index: delta::Index::new(data),
                data: data.to_vec(),
                depth,
            });
        }
    }
    let digest = pack.hasher.try_finalize().map_err(|e| git_error(path, e))?;
    pack.out.write_all(digest.as_bytes()).map_err(io)?;
    Ok(order.len() as u64)
}

/// How many objects before one are tried as its base.
const WINDOW: usize = 10;
/// Longer chains of deltas make reading slow.
const MAX_DEPTH: u32 = 50;
/// Smaller objects are not worth a delta.
const MIN_DELTA: usize = 64;
/// Larger objects are not kept in memory to serve as a base.
const MAX_WINDOW_OBJECT: usize = 16 << 20;
/// The pack type of an object written as a delta from the object named
/// after its header.
const REF_DELTA: u8 = 7;

/// An object recently written, to try as the base of the next ones.
struct Candidate {
    id: ObjectId,
    kind: gix::object::Kind,
    data: Vec<u8>,
    index: delta::Index,
    depth: u32,
}

fn type_bits(kind: gix::object::Kind) -> u8 {
    match kind {
        gix::object::Kind::Commit => 1,
        gix::object::Kind::Tree => 2,
        gix::object::Kind::Blob => 3,
        gix::object::Kind::Tag => 4,
    }
}

/// An object a pack holds, with the name of the tree entry it was found
/// under (empty for commits, tags and a commit's tree).
struct Reached {
    id: ObjectId,
    kind: gix::object::Kind,
    size: usize,
    name: Vec<u8>,
}

/// Every object `starts` reach, each once, leaving out `known` and what
/// only they reach; with `commits_only`, commits alone.
fn reach(
    git: &gix::Repository,
    path: &Path,
    starts: &[ObjectId],
    known: &HashSet<ObjectId>,
    commits_only: bool,
) -> Result<Vec<Reached>> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    let mut stack: Vec<(ObjectId, Vec<u8>)> = starts.iter().map(|id| (*id, Vec::new())).collect();
    while let Some((id, name)) = stack.pop() {
        if known.contains(&id) || !seen.insert(id) {
            continue;
        }
        let object = git.find_object(id).map_err(|e| git_error(path, e))?;
        let bad = |reason: &str| Error::InvalidObject {
            id: id.to_string(),
            reason: reason.to_owned(),
        };
        match object.kind {
            gix::object::Kind::Commit => {
                let commit = raw::parse_commit(&object.data).ok_or_else(|| bad("bad commit"))?;
                if !commits_only {
                    let tree = hex_id(commit.tree).ok_or_else(|| bad("bad tree id"))?;
                    stack.push((tree, Vec::new()));
                }
                for parent in commit.parents {
                    let parent = hex_id(parent).ok_or_else(|| bad("bad parent id"))?;
                    stack.push((parent, Vec::new()));
                }
            }
            gix::object::Kind::Tree => {
                let entries = raw::parse_tree(&object.data, 20).ok_or_else(|| bad("bad tree"))?;
                for entry in entries {
                    // A submodule's commit lives in another repository.
                    if entry.mode != GITLINK_MODE {
                        let child =
                            ObjectId::try_from(entry.id.as_slice()).map_err(|_| bad("bad id"))?;
                        stack.push((child, entry.name));
                    }
                }
            }
            gix::object::Kind::Tag => {
                let tag = raw::parse_tag(&object.data).ok_or_else(|| bad("bad tag"))?;
                let target = hex_id(tag.object).ok_or_else(|| bad("bad object id"))?;
                stack.push((target, Vec::new()));
            }
            gix::object::Kind::Blob => {}
        }
        if !commits_only || object.kind == gix::object::Kind::Commit {
            order.push(Reached {
                id,
                kind: object.kind,
                size: object.data.len(),
                name,
            });
        }
    }
    Ok(order)
}

fn parse_ids(hex: &[String]) -> Result<Vec<ObjectId>> {
    hex.iter().map(|h| parse_id(h)).collect()
}

/// The type and size of a pack entry: 3 type bits and the size, 4 bits
/// first, then 7 bits per byte while the top bit is set.
fn entry_header(type_bits: u8, mut size: u64) -> Vec<u8> {
    let mut first = (type_bits << 4) | (size & 0x0f) as u8;
    size >>= 4;
    let mut out = Vec::new();
    while size > 0 {
        out.push(first | 0x80);
        first = (size & 0x7f) as u8;
        size >>= 7;
    }
    out.push(first);
    out
}

fn parse_id(hex: &str) -> Result<ObjectId> {
    ObjectId::from_hex(hex.as_bytes()).map_err(|_| Error::InvalidObject {
        id: hex.to_owned(),
        reason: "not a SHA-1 object id".to_owned(),
    })
}

fn hex_id(hex: &[u8]) -> Option<ObjectId> {
    ObjectId::from_hex(hex).ok()
}

/// Passes bytes on and hashes them for the pack's trailer.
struct Hashed<'a> {
    out: &'a mut dyn Write,
    hasher: gix::hash::Hasher,
}

impl Write for Hashed<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.out.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::entry_header;

    #[test]
    fn entry_sizes_are_written_as_git_reads_them() {
        assert_eq!(entry_header(3, 5), vec![0x35]);
        // 300 = 0b1_0010_1100: low four bits 1100, then 0b1_0010.
        assert_eq!(entry_header(1, 300), vec![0x9c, 0x12]);
    }
}
