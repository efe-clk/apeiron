//! The three-way merge of trees, shared by sync and `merge` (SYNC.md §4,
//! BRANCH.md §3).

use std::collections::BTreeSet;
use std::time::SystemTime;

use apeiron_core::{EntryKind, ObjectId, ObjectWriter, Repository, Snapshot, Tree, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::diff::{Change, ChangeKind};
use crate::error::{Error, Result};
use crate::name::Name;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Both devices changed the file differently; the other version is the
    /// conflict copy.
    BothChanged,
    /// Deleted here and changed on the other device; the changed file is
    /// kept.
    DeletedHere,
    /// Changed here and deleted on the other device; the changed file is
    /// kept.
    DeletedThere,
    /// Changed here while the round ran; the incoming version is the
    /// conflict copy.
    ChangedDuringSync,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub path: Name,
    pub kind: ConflictKind,
    /// Where the other version was put.
    pub copy: Option<Name>,
}

/// Two entries are the same when their kind, content and permission bits
/// are; times and owners do not count (SYNC.md §4).
pub(crate) fn same(a: Option<&TreeEntry>, b: Option<&TreeEntry>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.kind == b.kind && a.mode == b.mode,
        _ => false,
    }
}

/// Whether two trees hold the same entries by [`same`], at every depth.
pub(crate) fn same_content(repo: &Repository, a: ObjectId, b: ObjectId) -> Result<bool> {
    if a == b {
        return Ok(true);
    }
    let (a, b) = (repo.read_tree(&a)?, repo.read_tree(&b)?);
    for [x, y] in by_name([&a, &b]) {
        let equal = match (x, y) {
            (
                Some(TreeEntry {
                    kind: EntryKind::Directory { tree: x },
                    mode: mx,
                    ..
                }),
                Some(TreeEntry {
                    kind: EntryKind::Directory { tree: y },
                    mode: my,
                    ..
                }),
            ) => mx == my && same_content(repo, *x, *y)?,
            _ => same(x, y),
        };
        if !equal {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn is_file(e: &TreeEntry) -> bool {
    matches!(e.kind, EntryKind::File { .. })
}

pub(crate) fn is_dir(e: Option<&TreeEntry>) -> bool {
    matches!(
        e,
        Some(TreeEntry {
            kind: EntryKind::Directory { .. },
            ..
        })
    )
}

pub(crate) fn tree_of(repo: &Repository, e: Option<&TreeEntry>) -> Result<Tree> {
    match e {
        Some(TreeEntry {
            kind: EntryKind::Directory { tree },
            ..
        }) => Ok(repo.read_tree(tree)?),
        _ => Ok(Tree {
            entries: Vec::new(),
        }),
    }
}

/// The names in any of `trees`, sorted, with each tree's entry. Tree
/// entries are sorted by name (FORMAT.md §6.3).
pub(crate) fn by_name<const N: usize>(trees: [&Tree; N]) -> Vec<[Option<&TreeEntry>; N]> {
    let mut at = [0usize; N];
    let mut out = Vec::new();
    loop {
        let next = (0..N)
            .filter_map(|i| trees[i].entries.get(at[i]).map(|e| e.name.as_slice()))
            .min();
        let Some(name) = next else {
            return out;
        };
        out.push(std::array::from_fn(|i| match trees[i].entries.get(at[i]) {
            Some(e) if e.name == name => {
                at[i] += 1;
                Some(e)
            }
            _ => None,
        }));
    }
}

/// An entry of the merged tree.
pub(crate) enum Item {
    Entry(TreeEntry),
    /// A folder whose content was merged; `entry` gives its name and
    /// attributes.
    Dir {
        entry: TreeEntry,
        items: Vec<Item>,
    },
}

pub(crate) struct Merge<'a> {
    pub(crate) repo: &'a Repository,
    pub(crate) suffix: &'a str,
    pub(crate) conflicts: Vec<Conflict>,
}

impl Merge<'_> {
    pub(crate) fn run(
        mut self,
        base: Option<ObjectId>,
        local: ObjectId,
        remote: ObjectId,
    ) -> Result<(Vec<Item>, Vec<Conflict>)> {
        let base = match base {
            Some(id) => self.repo.read_tree(&id)?,
            None => Tree {
                entries: Vec::new(),
            },
        };
        let local = self.repo.read_tree(&local)?;
        let remote = self.repo.read_tree(&remote)?;
        let items = self.trees(&base, &local, &remote, &Name::default())?;
        Ok((items, self.conflicts))
    }

    fn trees(&mut self, base: &Tree, local: &Tree, remote: &Tree, rel: &Name) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        let mut copies = Vec::new();
        for [b, l, r] in by_name([base, local, remote]) {
            let Some(any) = l.or(r).or(b) else { continue };
            let path = rel.join(&any.name);
            let changed_l = !same(b, l);
            let changed_r = !same(b, r);
            // Folders on both sides, or a folder deleted on one side and
            // changed on the other, are merged inside.
            let dir_or_gone = |e: Option<&TreeEntry>| is_dir(e) || e.is_none() && is_dir(b);
            if changed_l && changed_r && dir_or_gone(l) && dir_or_gone(r) && !same(l, r) {
                let inner = self.trees(
                    &tree_of(self.repo, b)?,
                    &tree_of(self.repo, l)?,
                    &tree_of(self.repo, r)?,
                    &path,
                )?;
                if (l.is_none() || r.is_none()) && inner.is_empty() {
                    continue;
                }
                let entry = l
                    .or(r)
                    .cloned()
                    .ok_or(Error::InvalidInput(path.to_string()))?;
                items.push(Item::Dir {
                    entry,
                    items: inner,
                });
                continue;
            }
            let (keep, conflict) = if !changed_r {
                (l, None)
            } else if !changed_l {
                (r, None)
            } else if same(l, r) {
                (l, None)
            } else {
                match (l, r) {
                    (None, Some(_)) => (r, Some((ConflictKind::DeletedHere, None))),
                    (Some(_), None) => (l, Some((ConflictKind::DeletedThere, None))),
                    _ => (l, Some((ConflictKind::BothChanged, r))),
                }
            };
            if let Some(entry) = keep {
                items.push(Item::Entry(entry.clone()));
            }
            if let Some((kind, other)) = conflict {
                if let Some(other) = other {
                    copies.push((self.conflicts.len(), other.clone()));
                }
                self.conflicts.push(Conflict {
                    path,
                    kind,
                    copy: None,
                });
            }
        }
        // Conflict copies get free names once every other name is known.
        let mut taken: BTreeSet<Vec<u8>> = items.iter().map(|i| item_name(i).to_vec()).collect();
        for (conflict, mut other) in copies {
            let name = free_name(&other.name, self.suffix, |n| taken.contains(n));
            taken.insert(name.clone());
            self.conflicts[conflict].copy = Some(rel.join(&name));
            other.name = name;
            items.push(Item::Entry(other));
        }
        Ok(items)
    }
}

pub(crate) fn item_name(item: &Item) -> &[u8] {
    match item {
        Item::Entry(e) | Item::Dir { entry: e, .. } => &e.name,
    }
}

/// Writes the merged folders; returns the root tree.
pub(crate) fn write_items(writer: &mut ObjectWriter<'_>, items: Vec<Item>) -> Result<ObjectId> {
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        entries.push(match item {
            Item::Entry(e) => e,
            Item::Dir { mut entry, items } => {
                entry.kind = EntryKind::Directory {
                    tree: write_items(writer, items)?,
                };
                entry
            }
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(writer.put_tree(&Tree { entries })?)
}

/// `name` with `suffix` before its extension, and a number after it while
/// that is taken.
pub(crate) fn free_name(name: &[u8], suffix: &str, taken: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let split = match name.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 => i,
        _ => name.len(),
    };
    let (stem, ext) = name.split_at(split);
    for n in 1.. {
        let mut out = stem.to_vec();
        out.extend_from_slice(suffix.as_bytes());
        if n > 1 {
            // " (conflict …)" becomes " (conflict … 2)".
            out.pop();
            out.extend_from_slice(format!(" {n})").as_bytes());
        }
        out.extend_from_slice(ext);
        if !taken(&out) {
            return out;
        }
    }
    unreachable!("an unbounded range always yields")
}

/// What a merge did (BRANCH.md §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeAction {
    /// The ref already holds everything the other snapshot has.
    UpToDate,
    /// The ref moved forward to the other snapshot.
    FastForward,
    /// A new snapshot joins both lines of history.
    Merged,
}

/// What to merge into which ref.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeOptions {
    /// The snapshot to merge.
    pub from: ObjectId,
    /// How the user named it; used in the message and in conflict copies.
    pub from_name: String,
    /// The ref that moves.
    pub into: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    /// Reports the result without moving the ref.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeOutcome {
    pub action: MergeAction,
    /// What the ref points to now (would point to, in a dry run).
    pub snapshot: ObjectId,
    /// What it pointed to before.
    pub previous: ObjectId,
    /// Paths that differ between the two.
    pub changes: Vec<Change>,
    pub conflicts: Vec<Conflict>,
}

/// Merges `options.from` into the ref `options.into` (BRANCH.md §2). The
/// caller holds the repository lock shared.
pub fn merge(repo: &mut Repository, options: &MergeOptions) -> Result<MergeOutcome> {
    let into = options.into.as_str();
    let ours = repo
        .read_ref(into)?
        .ok_or_else(|| Error::NotFound(format!("ref {into}")))?;
    let theirs = options.from;
    let mut outcome = MergeOutcome {
        action: MergeAction::UpToDate,
        snapshot: ours,
        previous: ours,
        changes: Vec::new(),
        conflicts: Vec::new(),
    };
    if ours == theirs || repo.is_ancestor(theirs, ours)? {
        return Ok(outcome);
    }
    if repo.is_ancestor(ours, theirs)? {
        outcome.action = MergeAction::FastForward;
        outcome.snapshot = theirs;
        let (a, b) = (
            repo.read_snapshot(&ours)?.tree,
            repo.read_snapshot(&theirs)?.tree,
        );
        changes(repo, a, b, &Name::default(), &mut outcome.changes)?;
        if !options.dry_run {
            repo.update_ref(into, Some(ours), Some(theirs))?;
        }
        return Ok(outcome);
    }

    let base = repo
        .merge_base(ours, theirs)?
        .map(|b| repo.read_snapshot(&b).map(|s| s.tree))
        .transpose()?;
    let (our_tree, their_tree) = (
        repo.read_snapshot(&ours)?.tree,
        repo.read_snapshot(&theirs)?.tree,
    );
    let suffix = format!(
        " (conflict {})",
        options.from_name.replace(['/', '\\'], "-")
    );
    let (items, conflicts) = Merge {
        repo,
        suffix: &suffix,
        conflicts: Vec::new(),
    }
    .run(base, our_tree, their_tree)?;
    let mut writer = repo.writer();
    let merged = write_items(&mut writer, items)?;
    writer.finish()?;
    changes(
        repo,
        our_tree,
        merged,
        &Name::default(),
        &mut outcome.changes,
    )?;

    let mut message = options
        .message
        .clone()
        .unwrap_or_else(|| format!("merge {} into {into}", options.from_name));
    for c in &conflicts {
        message.push_str(&format!("\nconflict: {} ({:?})", c.path, c.kind));
    }
    let snapshot = Snapshot {
        tree: merged,
        parents: vec![ours, theirs],
        time: crate::os::to_time(SystemTime::now()),
        author: options.author.clone(),
        host: options.host.clone(),
        message: Some(message),
        meta: Vec::new(),
    };
    outcome.action = MergeAction::Merged;
    outcome.conflicts = conflicts;
    if options.dry_run {
        return Ok(outcome);
    }
    let mut writer = repo.writer();
    let id = writer.put_snapshot(&snapshot)?;
    writer.finish()?;
    repo.update_ref(into, Some(ours), Some(id))?;
    outcome.snapshot = id;
    Ok(outcome)
}

/// The paths that differ from `a` to `b` by [`same`]; a folder added or
/// removed is one change.
fn changes(
    repo: &Repository,
    a: ObjectId,
    b: ObjectId,
    rel: &Name,
    out: &mut Vec<Change>,
) -> Result<()> {
    if a == b {
        return Ok(());
    }
    let (ta, tb) = (repo.read_tree(&a)?, repo.read_tree(&b)?);
    for [x, y] in by_name([&ta, &tb]) {
        let Some(any) = x.or(y) else { continue };
        let path = rel.join(&any.name);
        let kind = match (x, y) {
            (
                Some(TreeEntry {
                    kind: EntryKind::Directory { tree: x },
                    ..
                }),
                Some(TreeEntry {
                    kind: EntryKind::Directory { tree: y },
                    ..
                }),
            ) => {
                changes(repo, *x, *y, &path, out)?;
                continue;
            }
            _ if same(x, y) => continue,
            (Some(_), Some(_)) => ChangeKind::Modified,
            (Some(_), None) => ChangeKind::Removed,
            _ => ChangeKind::Added,
        };
        out.push(Change { path, kind });
    }
    Ok(())
}
