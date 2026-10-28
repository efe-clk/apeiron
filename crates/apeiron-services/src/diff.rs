//! Differences between two snapshots.

use std::cmp::Ordering;

use apeiron_core::{EntryKind, ObjectId, Repository, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::name::Name;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    /// Content, link target or attributes changed.
    Modified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: Name,
    pub kind: ChangeKind,
}

/// Lists the paths that differ from `from` to `to`, sorted by path. A folder
/// that is added or removed is listed together with everything inside it.
pub fn diff(repo: &Repository, from: &ObjectId, to: &ObjectId) -> Result<Vec<Change>> {
    let a = repo.read_snapshot(from)?.tree;
    let b = repo.read_snapshot(to)?.tree;
    let mut out = Vec::new();
    diff_trees(repo, &a, &b, &Name::default(), &mut out)?;
    Ok(out)
}

fn diff_trees(
    repo: &Repository,
    a: &ObjectId,
    b: &ObjectId,
    prefix: &Name,
    out: &mut Vec<Change>,
) -> Result<()> {
    if a == b {
        return Ok(());
    }
    let a = repo.read_tree(a)?.entries;
    let b = repo.read_tree(b)?.entries;
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        let order = match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) => x.name.cmp(&y.name),
            (Some(_), None) => Ordering::Less,
            _ => Ordering::Greater,
        };
        match order {
            Ordering::Less => {
                whole(repo, &a[i], prefix, ChangeKind::Removed, out)?;
                i += 1;
            }
            Ordering::Greater => {
                whole(repo, &b[j], prefix, ChangeKind::Added, out)?;
                j += 1;
            }
            Ordering::Equal => {
                both(repo, &a[i], &b[j], prefix, out)?;
                i += 1;
                j += 1;
            }
        }
    }
    Ok(())
}

fn both(
    repo: &Repository,
    x: &TreeEntry,
    y: &TreeEntry,
    prefix: &Name,
    out: &mut Vec<Change>,
) -> Result<()> {
    let path = prefix.join(&x.name);
    match (&x.kind, &y.kind) {
        // Folder attributes change whenever their content does; only the content is compared.
        (EntryKind::Directory { tree: a }, EntryKind::Directory { tree: b }) => {
            diff_trees(repo, a, b, &path, out)
        }
        (EntryKind::Directory { .. }, _) | (_, EntryKind::Directory { .. }) => {
            whole(repo, x, prefix, ChangeKind::Removed, out)?;
            whole(repo, y, prefix, ChangeKind::Added, out)
        }
        _ => {
            if x != y {
                out.push(Change {
                    path,
                    kind: ChangeKind::Modified,
                });
            }
            Ok(())
        }
    }
}

/// Reports `entry` and, for a folder, everything below it.
fn whole(
    repo: &Repository,
    entry: &TreeEntry,
    prefix: &Name,
    kind: ChangeKind,
    out: &mut Vec<Change>,
) -> Result<()> {
    let path = prefix.join(&entry.name);
    out.push(Change {
        path: path.clone(),
        kind,
    });
    if let EntryKind::Directory { tree } = &entry.kind {
        for child in repo.read_tree(tree)?.entries {
            whole(repo, &child, &path, kind, out)?;
        }
    }
    Ok(())
}
