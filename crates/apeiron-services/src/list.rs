//! Finding and listing paths inside a snapshot.

use apeiron_core::{EntryKind, ObjectId, Repository, Time, Tree, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::name::Name;

/// What a path inside a snapshot points to.
pub(crate) enum Node {
    /// The root, or a folder.
    Tree(Tree),
    /// A file or a symbolic link.
    Entry(TreeEntry),
}

/// Follows a `/`-separated `path` from the tree `root`; an empty path is the root.
pub(crate) fn lookup(repo: &Repository, root: &ObjectId, path: &[u8]) -> Result<Node> {
    let mut tree = repo.read_tree(root)?;
    let mut walked = Name::default();
    let mut parts = path
        .split(|&b| b == b'/')
        .filter(|p| !p.is_empty())
        .peekable();
    while let Some(part) = parts.next() {
        walked = walked.join(part);
        let not_found = || Error::NotFound(format!("path {walked}"));
        let index = tree
            .entries
            .binary_search_by(|e| e.name.as_slice().cmp(part))
            .map_err(|_| not_found())?;
        let entry = tree.entries.swap_remove(index);
        match entry.kind {
            EntryKind::Directory { tree: id } => tree = repo.read_tree(&id)?,
            _ if parts.peek().is_none() => return Ok(Node::Entry(entry)),
            _ => return Err(not_found()),
        }
    }
    Ok(Node::Tree(tree))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    File,
    Dir,
    Symlink,
}

/// One entry of a listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryInfo {
    pub name: Name,
    pub kind: Kind,
    /// Size of a file.
    pub size: Option<u64>,
    pub mode: Option<u16>,
    pub mtime: Option<Time>,
    /// Target of a symbolic link.
    pub target: Option<Name>,
}

impl From<&TreeEntry> for EntryInfo {
    fn from(e: &TreeEntry) -> Self {
        let (kind, size, target) = match &e.kind {
            EntryKind::File { size, .. } => (Kind::File, Some(*size), None),
            EntryKind::Directory { .. } => (Kind::Dir, None, None),
            EntryKind::Symlink { target } => (Kind::Symlink, None, Some(Name(target.clone()))),
        };
        EntryInfo {
            name: Name(e.name.clone()),
            kind,
            size,
            mode: e.mode,
            mtime: e.mtime,
            target,
        }
    }
}

/// Lists the folder at `path` in `snapshot`, or the single entry if `path` is a file.
pub fn list(repo: &Repository, snapshot: &ObjectId, path: &[u8]) -> Result<Vec<EntryInfo>> {
    let root = repo.read_snapshot(snapshot)?.tree;
    Ok(match lookup(repo, &root, path)? {
        Node::Tree(tree) => tree.entries.iter().map(EntryInfo::from).collect(),
        Node::Entry(entry) => vec![EntryInfo::from(&entry)],
    })
}
