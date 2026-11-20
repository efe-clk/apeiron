//! Ransomware early warning (CLI.md §4.3): a backup that changed or removed
//! too many of the files the previous one had, or whose changed files turned
//! from ordinary content into what looks random, as encrypted data does.

use apeiron_core::{EntryKind, ObjectId, Repository, TreeEntry};
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// The share of the previous files that may change or go before the alarm,
/// unless the request names another.
pub const DEFAULT_PERCENT: u32 = 50;
/// Fewer touched files than this never raise the alarm.
const MIN_FILES: u64 = 20;
/// At most this many changed files are read to judge their content.
const SAMPLE: usize = 100;
/// Bytes read from the start of a file to judge it.
const SAMPLE_BYTES: usize = 64 * 1024;
/// Bits per byte above which content looks random.
const RANDOM: f64 = 7.8;
/// Bits per byte below which content is clearly not random.
const ORDINARY: f64 = 7.0;

/// Why a backup looks like the work of ransomware.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Alarm {
    /// Files of the previous snapshot.
    pub files_before: u64,
    /// Of them, files changed or removed.
    pub touched: u64,
    /// Changed files read to judge their content.
    pub sampled: u64,
    /// Of them, files whose content turned random.
    pub turned_random: u64,
    pub message: String,
}

#[derive(Default)]
struct Tally {
    added: u64,
    removed: u64,
    /// Blobs before and after of changed files, up to [`SAMPLE`].
    modified: Vec<(Option<ObjectId>, Option<ObjectId>)>,
    modified_count: u64,
}

/// Compares the trees `before` and `after`, which holds `files_after`
/// files, and returns the alarm, if any. `percent` 0 turns it off.
pub(crate) fn check(
    repo: &Repository,
    before: &ObjectId,
    after: &ObjectId,
    files_after: u64,
    percent: u32,
) -> Result<Option<Alarm>> {
    if percent == 0 || before == after {
        return Ok(None);
    }
    let mut t = Tally::default();
    compare(repo, before, after, &mut t)?;
    let files_before = (files_after + t.removed).saturating_sub(t.added);
    let touched = t.modified_count + t.removed;
    let mut sampled = 0;
    let mut turned_random = 0;
    for (old, new) in &t.modified {
        let (Some(old), Some(new)) = (old, new) else {
            continue;
        };
        sampled += 1;
        if entropy(repo, new)? >= RANDOM && entropy(repo, old)? < ORDINARY {
            turned_random += 1;
        }
    }
    let share = touched.saturating_mul(100) >= files_before.saturating_mul(u64::from(percent));
    let message = if touched >= MIN_FILES && share {
        format!("{touched} of {files_before} files changed or removed since the last backup")
    } else if turned_random >= MIN_FILES / 2 && turned_random * 2 >= sampled {
        format!("{turned_random} of {sampled} changed files now look encrypted")
    } else {
        return Ok(None);
    };
    Ok(Some(Alarm {
        files_before,
        touched,
        sampled,
        turned_random,
        message: format!("possible ransomware: {message}"),
    }))
}

fn compare(repo: &Repository, a: &ObjectId, b: &ObjectId, t: &mut Tally) -> Result<()> {
    if a == b {
        return Ok(());
    }
    let a = repo.read_tree(a)?.entries;
    let b = repo.read_tree(b)?.entries;
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) if x.name == y.name => {
                match (&x.kind, &y.kind) {
                    (EntryKind::Directory { tree: p }, EntryKind::Directory { tree: q }) => {
                        compare(repo, p, q, t)?;
                    }
                    (EntryKind::File { blob: p, .. }, EntryKind::File { blob: q, .. }) => {
                        if p != q {
                            t.modified_count += 1;
                            if t.modified.len() < SAMPLE {
                                t.modified.push((*p, *q));
                            }
                        }
                    }
                    _ => {
                        t.removed += files(repo, x)?;
                        t.added += files(repo, y)?;
                    }
                }
                i += 1;
                j += 1;
            }
            (Some(x), Some(y)) if x.name < y.name => {
                t.removed += files(repo, x)?;
                i += 1;
            }
            (Some(x), None) => {
                t.removed += files(repo, x)?;
                i += 1;
            }
            (_, Some(y)) => {
                t.added += files(repo, y)?;
                j += 1;
            }
            (None, None) => break,
        }
    }
    Ok(())
}

/// The files of an entry: 1 for a file, those below a folder.
fn files(repo: &Repository, entry: &TreeEntry) -> Result<u64> {
    Ok(match &entry.kind {
        EntryKind::File { .. } => 1,
        EntryKind::Directory { tree } => {
            let mut n = 0;
            for e in repo.read_tree(tree)?.entries {
                n += files(repo, &e)?;
            }
            n
        }
        EntryKind::Symlink { .. } => 0,
    })
}

/// Shannon entropy in bits per byte of the start of a blob; 0 when empty.
fn entropy(repo: &Repository, blob: &ObjectId) -> Result<f64> {
    let blob = repo.read_blob(blob)?;
    let Some((chunk, len)) = blob.chunks.first() else {
        return Ok(0.0);
    };
    let data = repo.read_chunk(chunk, *len)?;
    Ok(bits_per_byte(&data[..data.len().min(SAMPLE_BYTES)]))
}

fn bits_per_byte(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in data {
        counts[usize::from(b)] += 1;
    }
    let n = data.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_ordinary_and_noise_is_random() {
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(500);
        assert!(bits_per_byte(text.as_bytes()) < ORDINARY);
        // A simple generator stands in for encrypted bytes.
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        let noise: Vec<u8> = (0..SAMPLE_BYTES)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        assert!(bits_per_byte(&noise) >= RANDOM);
        assert_eq!(bits_per_byte(&[]), 0.0);
    }
}
