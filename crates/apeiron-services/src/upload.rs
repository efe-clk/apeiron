//! A file added to a backup from outside, as through an upload link
//! (SHARE.md §7): a new snapshot that holds everything the last one did and
//! the new file beside it.

use std::io::Read;
use std::time::SystemTime;

use apeiron_core::{EntryKind, ObjectId, ObjectWriter, Repository, Snapshot, Tree, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::name::Name;
use crate::os;

/// How often adding starts again when another program moved the ref.
const ATTEMPTS: u32 = 3;

/// Where a file goes and what is written with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddFileOptions {
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// The folder inside the snapshot; made when missing.
    #[serde(default)]
    pub folder: Name,
    /// The file's name; a taken name gets ` (2)`, ` (3)` … before its
    /// extension.
    pub name: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AddFileOutcome {
    pub snapshot: ObjectId,
    /// Where the file is in it.
    pub path: Name,
    pub size: u64,
}

/// Stores what `reader` gives as a file in `options.folder` of the snapshot
/// `options.ref_name` points to, in a new snapshot whose parent that one is,
/// and moves the ref to it. Nothing is replaced or removed. The caller holds
/// the repository lock shared.
pub fn add_file(
    repo: &mut Repository,
    options: &AddFileOptions,
    reader: &mut dyn Read,
) -> Result<AddFileOutcome> {
    apeiron_core::validate_ref_name(&options.ref_name)?;
    let name = options.name.as_bytes();
    let bad_name = name.is_empty()
        || name.len() > 255
        || name == b"."
        || name == b".."
        || name.iter().any(|&b| b == b'/' || b == b'\\' || b < 0x20);
    if bad_name {
        return Err(Error::InvalidInput(format!(
            "{:?} cannot be a file name",
            options.name
        )));
    }
    let folder: Vec<Vec<u8>> = options
        .folder
        .as_bytes()
        .split(|&b| b == b'/')
        .filter(|p| !p.is_empty())
        .map(<[u8]>::to_vec)
        .collect();
    if folder.iter().any(|p| p == b"." || p == b"..") {
        return Err(Error::InvalidInput(format!(
            "{} is not a folder inside a backup",
            options.folder
        )));
    }

    let mut writer = repo.writer();
    let (size, blob) = writer.put_file(reader)?;
    writer.finish()?;
    let now = os::to_time(SystemTime::now());
    let file = TreeEntry {
        name: name.to_vec(),
        kind: EntryKind::File { size, blob },
        mode: Some(0o644),
        mtime: Some(now),
        uid: None,
        gid: None,
        xattrs: Vec::new(),
    };

    let mut attempt = 1;
    loop {
        let parent = repo.read_ref(&options.ref_name)?;
        let root = match parent {
            Some(id) => repo.read_tree(&repo.read_snapshot(&id)?.tree)?,
            None => Tree {
                entries: Vec::new(),
            },
        };
        let mut chain = vec![root];
        for part in &folder {
            let next = match chain
                .last()
                .and_then(|t| t.entries.iter().find(|e| &e.name == part))
            {
                Some(TreeEntry {
                    kind: EntryKind::Directory { tree },
                    ..
                }) => repo.read_tree(tree)?,
                Some(_) => {
                    return Err(Error::InvalidInput(format!(
                        "{} is a file, not a folder",
                        String::from_utf8_lossy(part)
                    )));
                }
                None => Tree {
                    entries: Vec::new(),
                },
            };
            chain.push(next);
        }

        let mut writer = repo.writer();
        // The file goes into the deepest folder, then each folder into the
        // one above it.
        let mut deepest = chain.pop().unwrap_or(Tree {
            entries: Vec::new(),
        });
        let placed = free(&deepest, name);
        let mut entry = file.clone();
        entry.name = placed.clone();
        deepest.entries.push(entry);
        let mut id = put_sorted(&mut writer, deepest)?;
        for (mut tree, part) in chain.into_iter().zip(&folder).rev() {
            match tree.entries.iter_mut().find(|e| &e.name == part) {
                Some(e) => e.kind = EntryKind::Directory { tree: id },
                None => tree.entries.push(TreeEntry {
                    name: part.clone(),
                    kind: EntryKind::Directory { tree: id },
                    mode: Some(0o755),
                    mtime: Some(now),
                    uid: None,
                    gid: None,
                    xattrs: Vec::new(),
                }),
            }
            id = put_sorted(&mut writer, tree)?;
        }
        let snapshot = writer.put_snapshot(&Snapshot {
            tree: id,
            parents: parent.into_iter().collect(),
            time: now,
            author: None,
            host: options.host.clone(),
            message: options.message.clone(),
            meta: Vec::new(),
        })?;
        writer.finish()?;
        match repo.update_ref(&options.ref_name, parent, Some(snapshot)) {
            Ok(()) => {
                let path = folder
                    .iter()
                    .chain([&placed])
                    .fold(Name::default(), |path, part| path.join(part));
                return Ok(AddFileOutcome {
                    snapshot,
                    path,
                    size,
                });
            }
            Err(apeiron_core::Error::RefConflict { .. }) if attempt < ATTEMPTS => attempt += 1,
            Err(e) => return Err(e.into()),
        }
    }
}

fn put_sorted(writer: &mut ObjectWriter<'_>, mut tree: Tree) -> Result<ObjectId> {
    tree.entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(writer.put_tree(&tree)?)
}

/// `name`, or `stem (2).ext`, `stem (3).ext` … if it is taken in `tree`.
fn free(tree: &Tree, name: &[u8]) -> Vec<u8> {
    let taken = |n: &[u8]| tree.entries.iter().any(|e| e.name == n);
    if !taken(name) {
        return name.to_vec();
    }
    let split = match name.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 => i,
        _ => name.len(),
    };
    let (stem, ext) = name.split_at(split);
    (2..)
        .map(|n| [stem, format!(" ({n})").as_bytes(), ext].concat())
        .find(|candidate| !taken(candidate))
        .unwrap_or_default()
}

/// Puts `put` at the top of the snapshot `ref_name` points to, replacing
/// files of the same names, and takes the files named in `remove` away, in
/// a new snapshot whose parent that one is; then moves the ref. For files
/// the program keeps in a ref of its own, such as shared plugins
/// (PLUGIN.md §5). The caller holds the repository lock shared.
pub fn put_files(
    repo: &mut Repository,
    ref_name: &str,
    put: &[(Vec<u8>, Vec<u8>)],
    remove: &[Vec<u8>],
    message: &str,
) -> Result<ObjectId> {
    apeiron_core::validate_ref_name(ref_name)?;
    let mut writer = repo.writer();
    let mut stored = Vec::new();
    for (name, data) in put {
        let (size, blob) = writer.put_file(&mut data.as_slice())?;
        stored.push((name.clone(), size, blob));
    }
    writer.finish()?;
    let now = os::to_time(SystemTime::now());
    let mut attempt = 1;
    loop {
        let parent = repo.read_ref(ref_name)?;
        let mut root = match parent {
            Some(id) => repo.read_tree(&repo.read_snapshot(&id)?.tree)?,
            None => Tree {
                entries: Vec::new(),
            },
        };
        root.entries
            .retain(|e| !remove.contains(&e.name) && !stored.iter().any(|(n, _, _)| *n == e.name));
        for (name, size, blob) in &stored {
            root.entries.push(TreeEntry {
                name: name.clone(),
                kind: EntryKind::File {
                    size: *size,
                    blob: *blob,
                },
                mode: Some(0o644),
                mtime: Some(now),
                uid: None,
                gid: None,
                xattrs: Vec::new(),
            });
        }
        let mut writer = repo.writer();
        let tree = put_sorted(&mut writer, root)?;
        let snapshot = writer.put_snapshot(&Snapshot {
            tree,
            parents: parent.into_iter().collect(),
            time: now,
            author: None,
            host: None,
            message: Some(message.to_owned()),
            meta: Vec::new(),
        })?;
        writer.finish()?;
        match repo.update_ref(ref_name, parent, Some(snapshot)) {
            Ok(()) => return Ok(snapshot),
            Err(apeiron_core::Error::RefConflict { .. }) if attempt < ATTEMPTS => attempt += 1,
            Err(e) => return Err(e.into()),
        }
    }
}
