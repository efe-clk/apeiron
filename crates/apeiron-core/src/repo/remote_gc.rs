//! Garbage collection of a remote copy (FORMAT.md §15.8).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

use super::gc::GcReport;
use super::transfer::{missing_objects, pack_file, read_remote, remote_index, write_pack};
use super::{CONFIG, INDEX, Repository, TMP};
use crate::config::utc_datetime;
use crate::error::{Error, Result};
use crate::id::{ObjectId, ObjectType};
use crate::index;
use crate::lock::RepoLock;
use crate::pack::{self, FileHash, PackEntry, PackWriter};
use crate::remote::Store;

/// The file that marks a cleanup in progress on a remote.
const GC_LOCK: &str = "locks/gc";
/// A lock older than this was left by an interrupted run.
const GC_LOCK_AGE: Duration = Duration::from_secs(24 * 3600);

/// Fails while a cleanup of `store` younger than 24 hours runs.
pub(super) fn check_gc_lock(store: &dyn Store) -> Result<()> {
    match lock_age(store)? {
        Some(age) if age < GC_LOCK_AGE => Err(Error::RemoteCleaning(store.location(GC_LOCK))),
        _ => Ok(()),
    }
}

/// How long ago the lock of `store` was written; `None` when there is none.
/// A lock whose time cannot be read counts as new.
fn lock_age(store: &dyn Store) -> Result<Option<Duration>> {
    let Some(bytes) = store.read(GC_LOCK)? else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&bytes);
    let written = text.split_whitespace().next().and_then(parse_utc);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(Some(match written {
        Some(at) => Duration::from_secs(now.saturating_sub(at)),
        None => Duration::ZERO,
    }))
}

/// Seconds since 1970 of a time written as `YYYY-MM-DDTHH:MM:SSZ`.
fn parse_utc(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| text.get(r)?.parse::<u64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era * 146_097 + doe).checked_sub(719_468)?;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

impl Repository {
    /// Removes from `store` the objects its refs cannot reach and rewrites
    /// its partly dead packs (FORMAT.md §15.8). Only for a remote this
    /// repository alone writes to. Takes the local repository lock
    /// exclusive; `host` is written into the remote's lock.
    pub fn gc_remote(
        &self,
        store: &dyn Store,
        dry_run: bool,
        host: Option<&str>,
    ) -> Result<GcReport> {
        let _lock = RepoLock::exclusive(&self.root)?;
        let config = store
            .read(CONFIG)?
            .ok_or_else(|| Error::InvalidRepository {
                path: store.location(""),
                reason: "no config file".to_owned(),
            })?;
        self.check_remote_config(store, &config)?;
        if !dry_run {
            match lock_age(store)? {
                Some(age) if age < GC_LOCK_AGE => {
                    return Err(Error::RemoteCleaning(store.location(GC_LOCK)));
                }
                Some(_) => store.delete(GC_LOCK)?,
                None => {}
            }
            let line = format!(
                "{} {}\n",
                utc_datetime(SystemTime::now()),
                host.unwrap_or("unknown")
            );
            store.write_new(GC_LOCK, line.as_bytes())?;
        }
        let result = self.collect_remote(store, dry_run);
        if !dry_run {
            // Kept on failure only if deleting it fails too.
            let removed = store.delete(GC_LOCK);
            let report = result?;
            removed?;
            return Ok(report);
        }
        result
    }

    fn collect_remote(&self, store: &dyn Store, dry_run: bool) -> Result<GcReport> {
        // Live: everything the remote refs reach.
        let have = remote_index(store, &self.keys)?;
        let mut live: HashSet<ObjectId> = HashSet::new();
        for name in store.list_refs()? {
            let Some(target) = store.read_ref(&name)? else {
                continue;
            };
            let reached = missing_objects(
                target,
                |id| live.contains(id),
                |id, ty| read_remote(store, &have, &self.keys, id, ty),
            )?;
            for (id, ty) in reached {
                if ty == ObjectType::Chunk && !have.contains_key(&id) {
                    return Err(Error::ObjectNotFound(id));
                }
                live.insert(id);
            }
        }

        // The packs the index files list, with their entries.
        let mut index_files = Vec::new();
        let mut packs: HashMap<FileHash, Vec<PackEntry>> = HashMap::new();
        for name in store.list(INDEX)? {
            if !name.ends_with(".idx") {
                continue;
            }
            let path = format!("{INDEX}/{name}");
            let Some(bytes) = store.read(&path)? else {
                continue;
            };
            match index::decode_index(&store.location(&path), &bytes, &self.keys) {
                Ok(locations) => {
                    for location in locations {
                        let entries = packs.entry(location.pack).or_default();
                        if !entries.iter().any(|e| e.offset == location.entry.offset) {
                            entries.push(location.entry);
                        }
                    }
                }
                // A damaged index lists nothing; its packs are left alone.
                Err(Error::Corrupt { .. }) => continue,
                Err(e) => return Err(e),
            }
            index_files.push(path);
        }
        let mut packs: Vec<(FileHash, Vec<PackEntry>)> = packs.into_iter().collect();
        packs.sort_by_key(|(name, _)| *name);

        let mut report = GcReport {
            dry_run,
            live_objects: live.len() as u64,
            ..GcReport::default()
        };
        let mut covered: HashSet<ObjectId> = HashSet::new();
        let mut kept: Vec<(FileHash, Vec<PackEntry>)> = Vec::new();
        let mut dead: Vec<(FileHash, Vec<PackEntry>)> = Vec::new();
        for (name, entries) in packs {
            if !entries.is_empty() && entries.iter().all(|e| live.contains(&e.id)) {
                covered.extend(entries.iter().map(|e| e.id));
                kept.push((name, entries));
            } else {
                dead.push((name, entries));
            }
        }
        let mut copies: Vec<(FileHash, PackEntry)> = Vec::new();
        for (name, entries) in &dead {
            for entry in entries {
                if live.contains(&entry.id) && covered.insert(entry.id) {
                    copies.push((*name, *entry));
                } else {
                    report.removed_objects += 1;
                    report.removed_bytes += u64::from(entry.stored_len);
                }
            }
        }
        if let Some(missing) = live.iter().find(|id| !covered.contains(id)) {
            return Err(Error::ObjectNotFound(*missing));
        }
        report.removed_packs = dead.len() as u64;
        if dry_run || (dead.is_empty() && index_files.len() <= 1) {
            return Ok(report);
        }

        let mut writer: Option<PackWriter> = None;
        let mut written = Vec::new();
        for (name, entry) in &copies {
            let path = pack_file(name);
            let stored = store.read_range(&path, entry.offset, u64::from(entry.stored_len))?;
            let w = match &mut writer {
                Some(w) => w,
                None => writer.insert(PackWriter::new(&self.dir(TMP), &self.keys)?),
            };
            w.add_stored(entry, &stored)?;
            if w.len() >= pack::TARGET_SIZE
                && let Some(w) = writer.take()
            {
                written.push(write_pack(store, w)?);
            }
        }
        if let Some(w) = writer.take() {
            written.push(write_pack(store, w)?);
        }
        report.written_packs = written.len() as u64;
        kept.extend(written);

        let (file, bytes) = index::encode_index(&kept, &self.keys)?;
        let new_index = format!("{INDEX}/{file}");
        if !index_files.contains(&new_index) {
            store.write_new(&new_index, &bytes)?;
        }
        for path in index_files {
            if path != new_index {
                store.delete(&path)?;
            }
        }
        for (name, _) in &dead {
            store.delete(&pack_file(name))?;
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_utc;

    #[test]
    fn lock_times_are_read_back() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2026-10-27T12:00:00Z"), Some(1_793_102_400));
        assert_eq!(parse_utc("2000-02-29T00:00:00Z"), Some(951_782_400));
        assert_eq!(parse_utc("yesterday"), None);
    }
}
