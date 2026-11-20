//! The folders a mounted repository shows (MOUNT.md §2), worked out from
//! the public API; snapshots never change, so what is read of them is kept.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use apeiron::{EntryInfo, Kind, ListRequest, LogRequest, Name, ObjectId, Repository, Time};

/// What a path of the mount stands for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    /// A folder of ref names, such as `backup` for `backup/documents`; the
    /// root is the empty one.
    Folder(String),
    /// A ref: `latest` and one folder per snapshot of its history.
    Ref(String),
    /// A path inside a snapshot; its root is the empty path.
    Snap { id: ObjectId, path: Name },
}

/// One entry of a folder of the mount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub name: Vec<u8>,
    pub node: Node,
    pub kind: Kind,
    pub size: u64,
    /// Permission bits; folders of the mount itself are 0o555.
    pub mode: u16,
    pub mtime: Time,
    pub target: Option<Vec<u8>>,
}

const EPOCH: Time = Time {
    seconds: 0,
    nanos: 0,
};

/// The name of a snapshot's folder: its time in UTC and the start of its id,
/// such as `2026-11-20T103000Z_3f9ae1c2`.
pub fn snapshot_name(time: Time, id: &ObjectId) -> String {
    let when = jiff::Timestamp::from_second(time.seconds)
        .map(|t| t.strftime("%Y-%m-%dT%H%M%SZ").to_string())
        .unwrap_or_else(|_| "0000-00-00T000000Z".to_owned());
    let hex = id.to_hex();
    format!("{when}_{}", &hex[..8])
}

/// Refs under these names belong to Apeiron itself, not to backups.
fn shown(name: &str) -> bool {
    !(name.starts_with("apeiron/") || name.starts_with("folder/"))
}

/// Listings of snapshot folders already read, by snapshot and path.
type Listings = HashMap<(ObjectId, Name), Arc<Vec<EntryInfo>>>;

/// The folders of a repository, read through the public API.
pub struct Tree {
    repo: Repository,
    listings: Mutex<Listings>,
}

impl Tree {
    pub fn new(repo: Repository) -> Self {
        Tree {
            repo,
            listings: Mutex::default(),
        }
    }

    fn folder(name: Vec<u8>, node: Node, mtime: Time) -> Child {
        Child {
            name,
            node,
            kind: Kind::Dir,
            size: 0,
            mode: 0o555,
            mtime,
            target: None,
        }
    }

    /// The entries of the folder `node`, sorted by name.
    pub fn children(&self, node: &Node) -> apeiron::Result<Vec<Child>> {
        let mut out = match node {
            Node::Folder(prefix) => {
                let refs = self.repo.refs()?;
                let mut names: Vec<(String, bool)> = Vec::new();
                for r in refs.iter().filter(|r| shown(&r.name)) {
                    let rest = match prefix.as_str() {
                        "" => Some(r.name.as_str()),
                        p => r.name.strip_prefix(p).and_then(|r| r.strip_prefix('/')),
                    };
                    let Some(rest) = rest else { continue };
                    let (first, deeper) = match rest.split_once('/') {
                        Some((first, _)) => (first, true),
                        None => (rest, false),
                    };
                    if !names.iter().any(|(n, _)| n == first) {
                        names.push((first.to_owned(), deeper));
                    }
                }
                names
                    .into_iter()
                    .map(|(name, deeper)| {
                        let full = match prefix.as_str() {
                            "" => name.clone(),
                            p => format!("{p}/{name}"),
                        };
                        let node = if deeper {
                            Node::Folder(full)
                        } else {
                            Node::Ref(full)
                        };
                        Self::folder(name.into_bytes(), node, EPOCH)
                    })
                    .collect()
            }
            Node::Ref(name) => {
                let mut out = Vec::new();
                let mut start = Some(name.clone());
                let mut first = true;
                while let Some(spec) = start.take() {
                    let page = match self.repo.log(&LogRequest {
                        start: spec,
                        limit: 1000,
                        grep: None,
                    }) {
                        Ok(page) => page,
                        // A ref deleted meanwhile has nothing in it.
                        Err(e) if e.kind == apeiron::ErrorKind::NotFound => break,
                        Err(e) => return Err(e),
                    };
                    for s in &page.snapshots {
                        let root = Node::Snap {
                            id: s.id,
                            path: Name::default(),
                        };
                        if first {
                            out.push(Self::folder(b"latest".to_vec(), root.clone(), s.time));
                            first = false;
                        }
                        let label = snapshot_name(s.time, &s.id).into_bytes();
                        out.push(Self::folder(label, root, s.time));
                    }
                    start = page.next.map(|id| id.to_hex());
                }
                out
            }
            Node::Snap { id, path } => self
                .listing(id, path)?
                .iter()
                .map(|e| Child {
                    name: e.name.as_bytes().to_vec(),
                    node: Node::Snap {
                        id: *id,
                        path: path.join(e.name.as_bytes()),
                    },
                    kind: e.kind,
                    size: e.size.unwrap_or(0),
                    mode: e.mode.unwrap_or(match e.kind {
                        Kind::Dir => 0o755,
                        _ => 0o644,
                    }),
                    mtime: e.mtime.unwrap_or(EPOCH),
                    target: e.target.as_ref().map(|t| t.as_bytes().to_vec()),
                })
                .collect(),
        };
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// The entry `name` of the folder `node`, if there is one.
    pub fn child(&self, node: &Node, name: &[u8]) -> apeiron::Result<Option<Child>> {
        Ok(self.children(node)?.into_iter().find(|c| c.name == name))
    }

    fn listing(&self, id: &ObjectId, path: &Name) -> apeiron::Result<Arc<Vec<EntryInfo>>> {
        let key = (*id, path.clone());
        if let Some(known) = self.lock().get(&key) {
            return Ok(known.clone());
        }
        let entries = Arc::new(self.repo.list(&ListRequest {
            snapshot: id.to_hex(),
            path: path.clone(),
        })?);
        self.lock().insert(key, entries.clone());
        Ok(entries)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Listings> {
        self.listings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn repo(&self) -> &Repository {
        &self.repo
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apeiron::BackupRequest;
    use std::fs;

    #[test]
    fn refs_snapshots_and_files_become_folders() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path().join("repo")).unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "first").unwrap();
        fs::write(src.join("sub/b.txt"), "second").unwrap();
        for _ in 0..2 {
            let request = BackupRequest::new(&src, "backup/documents");
            repo.backup(request).unwrap().wait().unwrap();
            fs::write(src.join("a.txt"), "changed").unwrap();
        }
        let tree = Tree::new(repo);
        let names = |node: &Node| -> Vec<String> {
            tree.children(node)
                .unwrap()
                .iter()
                .map(|c| String::from_utf8(c.name.clone()).unwrap())
                .collect()
        };
        assert_eq!(names(&Node::Folder(String::new())), ["backup"]);
        assert_eq!(names(&Node::Folder("backup".into())), ["documents"]);
        let snapshots = names(&Node::Ref("backup/documents".into()));
        assert_eq!(snapshots.len(), 3, "{snapshots:?}");
        assert_eq!(snapshots[2], "latest");
        let latest = tree
            .child(&Node::Ref("backup/documents".into()), b"latest")
            .unwrap()
            .unwrap();
        assert_eq!(names(&latest.node), ["a.txt", "sub"]);
        let file = tree.child(&latest.node, b"a.txt").unwrap().unwrap();
        assert_eq!((file.kind, file.size), (Kind::File, 7));
        assert!(tree.child(&latest.node, b"missing").unwrap().is_none());
    }
}
