//! When a job runs (DAEMON.md §4).

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan};

use crate::jobs::{Every, Job};
use crate::state::{JobState, RunResult};

const RETRY_FAILED: SignedDuration = SignedDuration::from_hours(1);
const RETRY_LOCKED: SignedDuration = SignedDuration::from_mins(10);

/// When `job` runs next, given its last run; a time not after `now` means
/// it is due. `tz` gives the local time of day for `at`.
pub fn next_run(job: &Job, last: Option<&JobState>, now: Timestamp, tz: &TimeZone) -> Timestamp {
    let Some(last) = last else {
        // Never run: due now, or later today at `at`.
        return match job.at {
            Some((hour, minute)) => {
                let today = now.to_zoned(tz.clone()).date();
                local_time(today, hour, minute, tz).unwrap_or(now)
            }
            None => now,
        };
    };
    let regular = match (job.every, job.at) {
        (Every::Days(days), Some((hour, minute))) => {
            let started = last.last_start.to_zoned(tz.clone()).date();
            started
                .checked_add(i64::from(days).days())
                .ok()
                .and_then(|day| local_time(day, hour, minute, tz))
        }
        (Every::Days(days), None) => last
            .last_start
            .checked_add(SignedDuration::from_hours(i64::from(days) * 24))
            .ok(),
        (Every::Minutes(minutes), _) => last
            .last_start
            .checked_add(SignedDuration::from_mins(i64::from(minutes)))
            .ok(),
        (Every::Hours(hours), _) => last
            .last_start
            .checked_add(SignedDuration::from_hours(i64::from(hours)))
            .ok(),
    }
    .unwrap_or(Timestamp::MAX);
    let retry = match last.result {
        RunResult::Failed => last.last_end.checked_add(RETRY_FAILED).ok(),
        RunResult::Locked => last.last_end.checked_add(RETRY_LOCKED).ok(),
        RunResult::Ok | RunResult::Warnings => None,
    };
    retry.map_or(regular, |retry| retry.min(regular))
}

/// The instant of a local time of day: in a daylight saving gap an hour
/// later, in a repeated hour the first one.
fn local_time(day: jiff::civil::Date, hour: i8, minute: i8, tz: &TimeZone) -> Option<Timestamp> {
    day.at(hour, minute, 0, 0)
        .to_zoned(tz.clone())
        .ok()
        .map(|z| z.timestamp())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::jobs::Action;

    fn job(every: Every, at: Option<(i8, i8)>) -> Job {
        Job {
            name: "x".to_owned(),
            repo: PathBuf::from("/r"),
            password_file: None,
            action: Action::Verify,
            every,
            at,
        }
    }

    fn ran(start: &str, result: RunResult) -> JobState {
        let start: Timestamp = start.parse().unwrap();
        JobState {
            last_start: start,
            last_end: start + SignedDuration::from_mins(5),
            result,
            message: String::new(),
        }
    }

    fn t(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn jobs_run_every_period_and_catch_up_once() {
        let utc = TimeZone::UTC;
        let now = t("2026-10-29T12:00:00Z");
        let hourly = job(Every::Hours(6), None);
        assert_eq!(next_run(&hourly, None, now, &utc), now);
        let last = ran("2026-10-29T08:00:00Z", RunResult::Ok);
        assert_eq!(
            next_run(&hourly, Some(&last), now, &utc),
            t("2026-10-29T14:00:00Z")
        );

        // Off for a week: due at once, and only once.
        let old = ran("2026-10-20T08:00:00Z", RunResult::Ok);
        assert!(next_run(&hourly, Some(&old), now, &utc) <= now);
        let caught_up = ran("2026-10-29T12:00:00Z", RunResult::Ok);
        assert!(next_run(&hourly, Some(&caught_up), now, &utc) > now);
    }

    #[test]
    fn at_keeps_the_time_of_day_even_after_a_late_run() {
        let utc = TimeZone::UTC;
        let daily = job(Every::Days(1), Some((2, 30)));
        let now = t("2026-10-29T12:00:00Z");
        // Never run and 02:30 has passed today: due now.
        assert!(next_run(&daily, None, now, &utc) <= now);
        assert_eq!(
            next_run(&daily, None, t("2026-10-29T01:00:00Z"), &utc),
            t("2026-10-29T02:30:00Z")
        );
        // A late catch-up run at 09:00 does not move the time of day.
        let late = ran("2026-10-29T09:00:00Z", RunResult::Ok);
        assert_eq!(
            next_run(&daily, Some(&late), now, &utc),
            t("2026-10-30T02:30:00Z")
        );
        let weekly = job(Every::Days(7), Some((2, 30)));
        assert_eq!(
            next_run(&weekly, Some(&late), now, &utc),
            t("2026-11-05T02:30:00Z")
        );
    }

    #[test]
    fn at_follows_daylight_saving_time() {
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        let daily = job(Every::Days(1), Some((2, 30)));
        // Summer time ended on 2026-10-25: 02:30 is UTC+2 before, UTC+1 after.
        let before = ran("2026-10-24T00:30:00Z", RunResult::Ok);
        assert_eq!(
            next_run(&daily, Some(&before), t("2026-10-24T12:00:00Z"), &berlin),
            t("2026-10-25T00:30:00Z")
        );
        let after = ran("2026-10-25T01:30:00Z", RunResult::Ok);
        assert_eq!(
            next_run(&daily, Some(&after), t("2026-10-25T12:00:00Z"), &berlin),
            t("2026-10-26T01:30:00Z")
        );
        // On 2026-03-29 02:30 does not exist; the job runs an hour later.
        let spring = ran("2026-03-28T01:30:00Z", RunResult::Ok);
        assert_eq!(
            next_run(&daily, Some(&spring), t("2026-03-28T12:00:00Z"), &berlin),
            t("2026-03-29T01:30:00Z")
        );
    }

    #[test]
    fn failures_are_retried_before_the_next_period() {
        let utc = TimeZone::UTC;
        let now = t("2026-10-29T12:00:00Z");
        let weekly = job(Every::Days(7), None);
        let failed = ran("2026-10-29T10:00:00Z", RunResult::Failed);
        assert_eq!(
            next_run(&weekly, Some(&failed), now, &utc),
            t("2026-10-29T11:05:00Z")
        );
        let locked = ran("2026-10-29T10:00:00Z", RunResult::Locked);
        assert_eq!(
            next_run(&weekly, Some(&locked), now, &utc),
            t("2026-10-29T10:15:00Z")
        );
        let hourly = job(Every::Hours(1), None);
        assert_eq!(
            next_run(&hourly, Some(&failed), now, &utc),
            t("2026-10-29T11:00:00Z")
        );
    }
}
