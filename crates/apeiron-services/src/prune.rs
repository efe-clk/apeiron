//! Thinning out the history of a backup by rules (FORMAT.md §13.4).

use std::collections::BTreeMap;

use apeiron_core::{ObjectId, Repository, Snapshot, Time};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::sync::folder_ref;

const SECONDS_PER_DAY: i64 = 86_400;

/// Which snapshots of a ref to keep. The newest is always kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PruneOptions {
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// The newest snapshots.
    #[serde(default)]
    pub keep_last: u32,
    /// The newest snapshot of each of this many days that have one; UTC.
    #[serde(default)]
    pub keep_daily: u32,
    /// The same per week, from Monday.
    #[serde(default)]
    pub keep_weekly: u32,
    #[serde(default)]
    pub keep_monthly: u32,
    #[serde(default)]
    pub keep_yearly: u32,
    /// Only reports; moves no ref and stores nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// A snapshot that stays, with the rules that keep it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PrunedKept {
    pub snapshot: ObjectId,
    /// Its id now that its parent changed; `snapshot` when nothing older went.
    pub new: ObjectId,
    pub time: Time,
    pub reasons: Vec<String>,
}

/// A snapshot that left the history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PrunedRemoved {
    pub snapshot: ObjectId,
    pub time: Time,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PruneOutcome {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub dry_run: bool,
    /// The snapshot the ref pointed to before.
    pub previous: ObjectId,
    /// The snapshot it points to now (would point to, in a dry run).
    pub snapshot: ObjectId,
    /// Newest first.
    pub kept: Vec<PrunedKept>,
    /// Newest first.
    pub removed: Vec<PrunedRemoved>,
}

/// The rules that keep each snapshot of a history given newest first.
fn reasons(options: &PruneOptions, times: &[Time]) -> Vec<Vec<String>> {
    let mut kept = vec![Vec::new(); times.len()];
    if let Some(newest) = kept.first_mut() {
        newest.push("newest".to_owned());
    }
    for (i, reasons) in kept.iter_mut().enumerate() {
        if i < options.keep_last as usize {
            reasons.push("last".to_owned());
        }
    }
    type Period = fn(Time) -> i64;
    let rules: [(&str, u32, Period); 4] = [
        ("daily", options.keep_daily, day),
        ("weekly", options.keep_weekly, week),
        ("monthly", options.keep_monthly, month),
        ("yearly", options.keep_yearly, year),
    ];
    for (name, count, period) in rules {
        let mut seen = BTreeMap::new();
        for (i, time) in times.iter().enumerate() {
            let p = period(*time);
            if seen.len() < count as usize && !seen.contains_key(&p) {
                seen.insert(p, ());
                kept[i].push(name.to_owned());
            }
        }
    }
    kept
}

fn day(t: Time) -> i64 {
    t.seconds.div_euclid(SECONDS_PER_DAY)
}

/// Weeks from Monday: 1970-01-01 was a Thursday.
fn week(t: Time) -> i64 {
    (day(t) + 3).div_euclid(7)
}

/// Year and month from a day (Howard Hinnant's days-to-civil algorithm).
fn civil(t: Time) -> (i64, i64) {
    let z = day(t) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m)
}

fn month(t: Time) -> i64 {
    let (y, m) = civil(t);
    y * 12 + m
}

fn year(t: Time) -> i64 {
    civil(t).0
}

/// Copies the snapshots of a ref that the rules keep, each with the one
/// before it as its parent, and moves the ref to the newest copy. The old
/// snapshots stay until garbage collection, as the reflog names them.
pub fn prune(repo: &mut Repository, options: &PruneOptions) -> Result<PruneOutcome> {
    let name = options.ref_name.as_str();
    apeiron_core::validate_ref_name(name)?;
    if options.keep_last == 0
        && options.keep_daily == 0
        && options.keep_weekly == 0
        && options.keep_monthly == 0
        && options.keep_yearly == 0
    {
        return Err(Error::InvalidInput(
            "give at least one rule of what to keep".to_owned(),
        ));
    }
    if name.starts_with("folder/")
        || name.starts_with("apeiron/")
        || repo.read_ref(&folder_ref(name))?.is_some()
    {
        return Err(Error::Unsupported(format!(
            "{name} belongs to sync or to Apeiron itself; other copies could not follow a shorter history"
        )));
    }
    if let Some(reason) = repo.ref_protection(name)? {
        return Err(apeiron_core::Error::Retained {
            name: name.to_owned(),
            reason,
        }
        .into());
    }
    let previous = repo
        .read_ref(name)?
        .ok_or_else(|| Error::NotFound(format!("ref {name}")))?;

    let mut history: Vec<(ObjectId, Snapshot)> = Vec::new();
    for item in repo.history(previous) {
        let (id, snapshot) = item?;
        if snapshot.parents.len() > 1 || snapshot.meta.iter().any(|(k, _)| k == "git.id") {
            return Err(Error::Unsupported(format!(
                "{name} holds merges or Git commits, whose history must stay whole"
            )));
        }
        history.push((id, snapshot));
    }
    let times: Vec<Time> = history.iter().map(|(_, s)| s.time).collect();
    let reasons = reasons(options, &times);

    // Oldest first: each copy names the copy before it. A snapshot nothing
    // older than which went gets its old id back, as ids follow from content.
    let mut writer = repo.writer();
    let mut parent = None;
    let mut new_ids = vec![None; history.len()];
    for i in (0..history.len()).rev() {
        if reasons[i].is_empty() {
            continue;
        }
        let copy = Snapshot {
            parents: parent.into_iter().collect(),
            ..history[i].1.clone()
        };
        let id = if options.dry_run {
            copy.encode().map(|_| ())?;
            writer.id_of_snapshot(&copy)?
        } else {
            writer.put_snapshot(&copy)?
        };
        new_ids[i] = Some(id);
        parent = Some(id);
    }
    writer.finish()?;
    let snapshot = new_ids[0].unwrap_or(previous);
    if !options.dry_run && snapshot != previous {
        repo.update_ref(name, Some(previous), Some(snapshot))?;
    }

    let mut kept = Vec::new();
    let mut removed = Vec::new();
    for (i, (id, s)) in history.iter().enumerate() {
        match new_ids[i] {
            Some(new) => kept.push(PrunedKept {
                snapshot: *id,
                new,
                time: s.time,
                reasons: reasons[i].clone(),
            }),
            None => removed.push(PrunedRemoved {
                snapshot: *id,
                time: s.time,
            }),
        }
    }
    Ok(PruneOutcome {
        ref_name: name.to_owned(),
        dry_run: options.dry_run,
        previous,
        snapshot,
        kept,
        removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(days: i64) -> Time {
        Time {
            seconds: days * SECONDS_PER_DAY + 3600,
            nanos: 0,
        }
    }

    fn kept(options: PruneOptions, times: &[Time]) -> Vec<usize> {
        reasons(&options, times)
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.is_empty())
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn the_newest_of_each_period_stays() {
        // Newest first, two per day: days 10, 10, 9, 9, 8, 8.
        let times = [at(10), at(10), at(9), at(9), at(8), at(8)];
        let daily = PruneOptions {
            keep_daily: 2,
            ..PruneOptions::default()
        };
        assert_eq!(kept(daily, &times), [0, 2]);
        let last = PruneOptions {
            keep_last: 3,
            ..PruneOptions::default()
        };
        assert_eq!(kept(last, &times), [0, 1, 2]);
    }

    #[test]
    fn days_without_a_snapshot_do_not_use_up_a_rule() {
        let times = [at(100), at(50), at(1)];
        let daily = PruneOptions {
            keep_daily: 3,
            ..PruneOptions::default()
        };
        assert_eq!(kept(daily, &times), [0, 1, 2]);
    }

    #[test]
    fn periods_fall_on_the_calendar() {
        // 1970-01-05 was the first Monday.
        assert_eq!(week(at(3)), 0);
        assert_eq!(week(at(4)), 1);
        assert_eq!(civil(at(0)), (1970, 1));
        assert_eq!(civil(at(31)), (1970, 2));
        assert_eq!(civil(at(365)), (1971, 1));
        assert_eq!(year(at(-1)), 1969);
        assert_eq!(month(at(30)) + 1, month(at(31)));
    }
}
