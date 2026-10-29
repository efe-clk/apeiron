//! Text output for people.

use apeiron::{ChangeKind, EntryInfo, Kind, ObjectId, Phase, Progress, Time};

pub(crate) fn short_id(id: &ObjectId) -> String {
    id.to_hex()[..12].to_owned()
}

pub(crate) fn short_key(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// `1 file`, `2 files`.
pub(crate) fn count(n: u64, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

pub(crate) fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `YYYY-MM-DD HH:MM:SS UTC`.
pub(crate) fn time(t: &Time) -> String {
    let days = t.seconds.div_euclid(86_400);
    let secs = t.seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// Days since 1970-01-01 to a calendar date (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

pub(crate) fn entry(e: &EntryInfo) -> String {
    let kind = match e.kind {
        Kind::File => '-',
        Kind::Dir => 'd',
        Kind::Symlink => 'l',
    };
    let mode = e.mode.map_or_else(|| "         ".to_owned(), permissions);
    let size = e.size.map_or_else(String::new, bytes);
    let time = e.mtime.as_ref().map_or_else(String::new, time);
    let mut line = format!("{kind}{mode}  {size:>10}  {time:23}  {}", e.name);
    if let Some(target) = &e.target {
        line.push_str(&format!(" -> {target}"));
    }
    line
}

fn permissions(mode: u16) -> String {
    let mut out = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

pub(crate) fn change(kind: ChangeKind) -> char {
    match kind {
        ChangeKind::Added => 'A',
        ChangeKind::Removed => 'D',
        ChangeKind::Modified => 'M',
    }
}

pub(crate) fn progress(p: &Progress) -> String {
    let phase = match p.phase {
        Phase::Starting => "starting",
        Phase::Scanning => "scanning",
        Phase::Storing => "storing",
        Phase::Restoring => "restoring",
        Phase::Exporting => "exporting",
        Phase::Copying => "copying",
        Phase::Verifying => "verifying",
        Phase::Finishing => "finishing",
        Phase::Done => "done",
    };
    let of = |done: u64, total: Option<u64>, f: fn(u64) -> String| match total {
        Some(total) => format!("{} / {}", f(done), f(total)),
        None => f(done),
    };
    let items = if p.phase == Phase::Copying {
        "objects"
    } else {
        "files"
    };
    let mut line = format!(
        "{phase}: {} {items}, {}",
        of(p.items_done, p.items_total, |n| n.to_string()),
        of(p.bytes_done, p.bytes_total, bytes)
    );
    if let Some(current) = &p.current {
        let name = current.to_string();
        let tail: String = name
            .chars()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        line.push_str("  ");
        line.push_str(&tail);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
        assert_eq!(
            time(&Time {
                seconds: 0,
                nanos: 0
            }),
            "1970-01-01 00:00:00 UTC"
        );
        assert_eq!(
            time(&Time {
                seconds: 1_700_000_000,
                nanos: 0
            }),
            "2023-11-14 22:13:20 UTC"
        );
        assert_eq!(
            time(&Time {
                seconds: -1,
                nanos: 0
            }),
            "1969-12-31 23:59:59 UTC"
        );
        assert_eq!(permissions(0o754), "rwxr-xr--");
    }
}
