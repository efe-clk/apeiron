//! Running jobs (DAEMON.md §2, §4, §6).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use apeiron::{
    BackupRequest, ErrorKind, GcRequest, Repository, RestoreTestRequest, SyncAction, SyncRequest,
    TransferRequest,
};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

use crate::jobs::{Action, Job, load_jobs};
use crate::schedule::next_run;
use crate::state::{JobState, RunResult, State};
use crate::{Error, IoContext, Result};

/// The daemon wakes at least this often to notice a changed jobs file or a
/// clock that jumped.
const MAX_SLEEP: SignedDuration = SignedDuration::from_mins(1);
const LOG_LIMIT: u64 = 1024 * 1024;

/// Where the jobs file and the state live.
#[derive(Clone, Debug)]
pub struct Paths {
    pub jobs: PathBuf,
    pub state_dir: PathBuf,
}

impl Paths {
    /// The usual folders of the system (DAEMON.md §3, §6).
    pub fn system() -> Result<Self> {
        let config = dirs::config_dir().ok_or(Error::NoFolder("configuration"))?;
        let state = dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .ok_or(Error::NoFolder("state"))?;
        Ok(Paths {
            jobs: config.join("apeiron").join("jobs.toml"),
            state_dir: state.join("apeiron"),
        })
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join("daemon.log")
    }
}

/// A job that ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunRecord {
    pub job: String,
    pub run: JobState,
}

/// What `apeiron daemon status` shows for a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct JobStatus {
    pub name: String,
    #[serde(rename = "do")]
    pub action: String,
    pub last: Option<JobState>,
    pub next_run: Timestamp,
}

/// Every job with its last run and its next run.
pub fn status(paths: &Paths) -> Result<Vec<JobStatus>> {
    let jobs = load_jobs(&paths.jobs)?;
    let state = State::load(&paths.state_file());
    let (now, tz) = (Timestamp::now(), TimeZone::system());
    Ok(jobs
        .into_iter()
        .map(|job| {
            let last = state.jobs.get(&job.name).cloned();
            JobStatus {
                next_run: next_run(&job, last.as_ref(), now, &tz),
                action: job.action.name().to_owned(),
                name: job.name,
                last,
            }
        })
        .collect())
}

/// The running daemon; only one per state folder.
#[derive(Debug)]
pub struct Daemon {
    paths: Paths,
    _lock: File,
}

impl Daemon {
    /// Takes the daemon lock, or fails with [`Error::Locked`].
    pub fn start(paths: Paths) -> Result<Self> {
        fs::create_dir_all(&paths.state_dir).at(&paths.state_dir)?;
        let lock_path = paths.state_dir.join("daemon.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .at(&lock_path)?;
        match fs4::FileExt::try_lock(&lock) {
            Ok(()) => {}
            Err(fs4::TryLockError::WouldBlock) => return Err(Error::Locked(lock_path)),
            Err(fs4::TryLockError::Error(e)) => return Err(e).at(&lock_path),
        }
        Ok(Daemon { paths, _lock: lock })
    }

    /// Runs every job that is due now, one after another, and records each
    /// run. Stops early once `stop` is set.
    pub fn run_due(&self, jobs: &[Job], stop: &AtomicBool) -> Result<Vec<RunRecord>> {
        let tz = TimeZone::system();
        let mut records = Vec::new();
        for job in jobs {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let mut state = State::load(&self.paths.state_file());
            let start = Timestamp::now();
            if next_run(job, state.jobs.get(&job.name), start, &tz) > start {
                continue;
            }
            let (result, message) = run_job(job, stop);
            let run = JobState {
                last_start: start,
                last_end: Timestamp::now(),
                result,
                message,
            };
            self.log(&format!("{} {:?}: {}", job.name, run.result, run.message));
            state.jobs.insert(job.name.clone(), run.clone());
            state.save(&self.paths.state_file())?;
            records.push(RunRecord {
                job: job.name.clone(),
                run,
            });
        }
        Ok(records)
    }

    /// Runs jobs as they become due until `stop` is set. A changed jobs file
    /// is read again; if it is invalid, the previous jobs are kept.
    pub fn run(&self, stop: &AtomicBool) -> Result<()> {
        let mut jobs = load_jobs(&self.paths.jobs)?;
        let mut seen = modified(&self.paths.jobs);
        self.log(&format!("started with {} jobs", jobs.len()));
        while !stop.load(Ordering::Relaxed) {
            let current = modified(&self.paths.jobs);
            if current != seen {
                seen = current;
                match load_jobs(&self.paths.jobs) {
                    Ok(new) => {
                        jobs = new;
                        self.log(&format!("read the jobs file again: {} jobs", jobs.len()));
                    }
                    Err(e) => self.log(&format!("keeping the previous jobs: {e}")),
                }
            }
            self.run_due(&jobs, stop)?;

            let state = State::load(&self.paths.state_file());
            let (now, tz) = (Timestamp::now(), TimeZone::system());
            let next = jobs
                .iter()
                .map(|job| next_run(job, state.jobs.get(&job.name), now, &tz))
                .min()
                .unwrap_or(Timestamp::MAX);
            let wake = next.min(now.saturating_add(MAX_SLEEP).unwrap_or(now));
            while !stop.load(Ordering::Relaxed) && Timestamp::now() < wake {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        self.log("stopped");
        Ok(())
    }

    /// Appends a line to the log; at the size limit the log starts anew and
    /// the previous one becomes `daemon.log.1`. Logging never fails a job.
    fn log(&self, line: &str) {
        let path = self.paths.log_file();
        if fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
            let _ = fs::rename(&path, path.with_extension("log.1"));
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let now = Timestamp::now()
                .round(jiff::Unit::Second)
                .unwrap_or_default();
            let _ = writeln!(file, "{now} {line}");
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Runs one job; an error becomes a failed (or locked) result.
fn run_job(job: &Job, stop: &AtomicBool) -> (RunResult, String) {
    match try_run(job, stop) {
        Ok(outcome) => outcome,
        Err(e) if e.kind == ErrorKind::Locked => (RunResult::Locked, e.message),
        Err(e) => (RunResult::Failed, e.message),
    }
}

fn open(job: &Job) -> apeiron::Result<Repository> {
    match Repository::open(&job.repo) {
        Err(e) if e.kind == ErrorKind::Key => {
            let Some(file) = &job.password_file else {
                return Err(apeiron::Error::new(
                    ErrorKind::Key,
                    "the repository is encrypted; set `password_file` for this job",
                ));
            };
            let text = fs::read_to_string(file).map_err(|e| {
                apeiron::Error::new(ErrorKind::Io, format!("{}: {e}", file.display()))
            })?;
            Repository::unlock(&job.repo, text.lines().next().unwrap_or_default())
        }
        result => result,
    }
}

/// Waits for a job, cancelling it once `stop` is set.
fn wait<T>(mut job: apeiron::Job<T>, stop: &AtomicBool) -> apeiron::Result<T> {
    let mut cancelled = false;
    loop {
        if let Some(result) = job.wait_timeout(Duration::from_millis(200)) {
            return result;
        }
        if !cancelled && stop.load(Ordering::Relaxed) {
            job.cancel();
            cancelled = true;
        }
    }
}

fn short(id: &apeiron::ObjectId) -> String {
    id.to_hex()[..12].to_owned()
}

fn try_run(job: &Job, stop: &AtomicBool) -> apeiron::Result<(RunResult, String)> {
    let repo = open(job)?;
    Ok(match &job.action {
        Action::Backup {
            folder,
            ref_name,
            message,
            push,
        } => {
            let mut request = BackupRequest::new(folder.clone(), ref_name.clone());
            request.message = message.clone();
            let report = wait(repo.backup(request)?, stop)?;
            let mut text = format!(
                "snapshot {}: {} files, {} new bytes",
                short(&report.snapshot),
                report.files,
                report.new_bytes
            );
            let mut result = RunResult::Ok;
            if !report.warnings.is_empty() {
                result = RunResult::Warnings;
                text.push_str(&format!("; {} files skipped", report.warnings.len()));
            }
            match push_all(&repo, ref_name, push, stop) {
                Ok(pushed) => text.push_str(&pushed),
                Err(e) => {
                    text.push_str(&format!("; push failed: {}", e.message));
                    result = RunResult::Failed;
                }
            }
            (result, text)
        }
        Action::Push { ref_name, remotes } => {
            let pushed = push_all(&repo, ref_name, remotes, stop)?;
            (RunResult::Ok, pushed.trim_start_matches("; ").to_owned())
        }
        Action::Verify => {
            let report = wait(repo.verify()?, stop)?;
            if report.is_ok() {
                (
                    RunResult::Ok,
                    format!("{} objects checked", report.objects_checked),
                )
            } else {
                (
                    RunResult::Failed,
                    format!(
                        "{} damaged files and {} damaged objects; run `apeiron verify`",
                        report.damaged_files.len(),
                        report.damaged_objects.len()
                    ),
                )
            }
        }
        Action::RestoreTest {
            ref_name,
            sample_bytes,
        } => {
            let mut request = RestoreTestRequest::new(ref_name.clone());
            request.sample_bytes = *sample_bytes;
            let report = wait(repo.restore_test(request)?, stop)?;
            match report.problems.first() {
                None => (
                    RunResult::Ok,
                    format!(
                        "restored and checked {} files, {} bytes",
                        report.files, report.bytes
                    ),
                ),
                Some(first) => (
                    RunResult::Failed,
                    format!(
                        "{} of {} restored files are damaged, for example {}: {}",
                        report.problems.len(),
                        report.files,
                        first.path,
                        first.message
                    ),
                ),
            }
        }
        Action::Sync {
            folder,
            remote,
            ref_name,
            mode,
        } => {
            let mut request = SyncRequest::new(folder.clone(), remote.clone());
            request.ref_name = ref_name.clone();
            request.mode = *mode;
            let report = wait(repo.sync(request)?, stop)?;
            let action = match report.action {
                SyncAction::UpToDate => "up to date",
                SyncAction::Sent => "sent",
                SyncAction::Received => "received",
                SyncAction::Merged => "merged",
            };
            let mut text = format!(
                "{} {action}: {} changes",
                report.ref_name,
                report.changes.len()
            );
            let attention = report.conflicts.len() + report.skipped.len() + report.warnings.len();
            if !report.conflicts.is_empty() {
                text.push_str(&format!("; {} conflicts", report.conflicts.len()));
            }
            if !report.skipped.is_empty() {
                text.push_str(&format!(
                    "; {} left for the next round",
                    report.skipped.len()
                ));
            }
            if !report.warnings.is_empty() {
                text.push_str(&format!("; {} files skipped", report.warnings.len()));
            }
            let result = if attention > 0 {
                RunResult::Warnings
            } else {
                RunResult::Ok
            };
            (result, text)
        }
        Action::Gc => {
            let report = wait(repo.gc(GcRequest::default())?, stop)?;
            (
                RunResult::Ok,
                format!(
                    "removed {} objects ({} bytes)",
                    report.removed_objects, report.removed_bytes
                ),
            )
        }
    })
}

/// Pushes the ref to each remote; returns `; pushed to …` per remote.
fn push_all(
    repo: &Repository,
    ref_name: &str,
    remotes: &[String],
    stop: &AtomicBool,
) -> apeiron::Result<String> {
    let mut text = String::new();
    for remote in remotes {
        let report = wait(repo.push(TransferRequest::new(remote, ref_name))?, stop)?;
        text.push_str(&format!("; pushed to {remote}: {} objects", report.objects));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Setup {
        dir: tempfile::TempDir,
        paths: Paths,
    }

    fn setup(jobs: &str) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/a.txt"), b"hello").unwrap();
        Repository::init(root.join("repo")).unwrap();
        let jobs = jobs.replace("ROOT", &root.to_string_lossy().replace('\\', "/"));
        fs::write(root.join("jobs.toml"), jobs).unwrap();
        let paths = Paths {
            jobs: root.join("jobs.toml"),
            state_dir: root.join("state"),
        };
        Setup { dir, paths }
    }

    const JOBS: &str = r#"
        [job.a-backup]
        repo = "ROOT/repo"
        do = "backup"
        folder = "ROOT/src"
        every = "day"
        push = ["disk"]

        [job.b-test]
        repo = "ROOT/repo"
        do = "restore-test"
        every = "week"

        [job.c-check]
        repo = "ROOT/repo"
        do = "verify"
        every = "week"
    "#;

    #[test]
    fn due_jobs_run_once_and_their_results_are_kept() {
        let s = setup(JOBS);
        let repo = Repository::open(s.dir.path().join("repo")).unwrap();
        let disk = s.dir.path().join("disk");
        repo.add_remote("disk", disk.to_str().unwrap()).unwrap();
        let daemon = Daemon::start(s.paths.clone()).unwrap();
        assert!(matches!(
            Daemon::start(s.paths.clone()),
            Err(Error::Locked(_))
        ));

        let jobs = load_jobs(&s.paths.jobs).unwrap();
        let stop = AtomicBool::new(false);
        let runs = daemon.run_due(&jobs, &stop).unwrap();
        let results: Vec<_> = runs
            .iter()
            .map(|r| (r.job.as_str(), r.run.result))
            .collect();
        assert_eq!(
            results,
            [
                ("a-backup", RunResult::Ok),
                ("b-test", RunResult::Ok),
                ("c-check", RunResult::Ok)
            ],
            "{runs:?}"
        );
        assert!(runs[0].run.message.contains("pushed to disk"));
        assert!(Repository::open(&disk).unwrap().resolve("main").is_ok());
        assert!(runs[1].run.message.contains("restored and checked 1 files"));

        // Nothing is due any more, and the state survives a restart.
        assert!(daemon.run_due(&jobs, &stop).unwrap().is_empty());
        let status = status(&s.paths).unwrap();
        assert_eq!(status.len(), 3);
        assert!(status.iter().all(|j| j.next_run > Timestamp::now()));
        assert_eq!(status[0].last.as_ref().unwrap().result, RunResult::Ok);
        let log = fs::read_to_string(s.paths.log_file()).unwrap();
        assert_eq!(log.lines().count(), 3, "{log}");
    }

    #[test]
    fn failures_are_recorded_and_encrypted_repositories_need_a_password_file() {
        let s = setup(
            r#"
            [job.missing]
            repo = "ROOT/repo"
            do = "backup"
            folder = "ROOT/nothing"
            every = "day"

            [job.secret]
            repo = "ROOT/secret"
            do = "verify"
            every = "day"

            [job.unlocked]
            repo = "ROOT/secret"
            password_file = "ROOT/pw"
            do = "verify"
            every = "day"
            "#,
        );
        let root = s.dir.path();
        Repository::init_encrypted(root.join("secret"), "pass word").unwrap();
        fs::write(root.join("pw"), "pass word\n").unwrap();
        let daemon = Daemon::start(s.paths.clone()).unwrap();
        let jobs = load_jobs(&s.paths.jobs).unwrap();
        let runs = daemon.run_due(&jobs, &AtomicBool::new(false)).unwrap();
        let results: Vec<_> = runs.iter().map(|r| r.run.result).collect();
        assert_eq!(
            results,
            [RunResult::Failed, RunResult::Failed, RunResult::Ok],
            "{runs:?}"
        );
        assert!(runs[1].run.message.contains("password_file"));

        // A failed job is tried again after an hour, not before.
        let status = status(&s.paths).unwrap();
        let last_end = status[0].last.as_ref().unwrap().last_end;
        assert_eq!(status[0].next_run.duration_since(last_end).as_secs(), 3600);
    }

    #[test]
    fn sync_jobs_keep_two_folders_equal() {
        let s = setup(
            r#"
            [job.a]
            repo = "ROOT/repo"
            do = "sync"
            folder = "ROOT/src"
            remote = "hub"
            every = "15m"

            [job.b]
            repo = "ROOT/copy"
            do = "sync"
            folder = "ROOT/other"
            ref = "sync/src"
            remote = "origin"
            mode = "receive"
            every = "15m"
            "#,
        );
        let root = s.dir.path();
        let repo = Repository::open(root.join("repo")).unwrap();
        repo.add_remote("hub", root.join("hub").to_str().unwrap())
            .unwrap();
        // The hub must exist before the second device can clone it.
        repo.sync(apeiron::SyncRequest::new(root.join("src"), "hub"))
            .unwrap()
            .wait()
            .unwrap();
        let request = apeiron::CloneRequest {
            url: root.join("hub").to_str().unwrap().to_owned(),
            path: root.join("copy"),
            secret: None,
        };
        Repository::clone_from(request).unwrap().wait().unwrap();
        fs::create_dir(root.join("other")).unwrap();

        let daemon = Daemon::start(s.paths.clone()).unwrap();
        let jobs = load_jobs(&s.paths.jobs).unwrap();
        let runs = daemon.run_due(&jobs, &AtomicBool::new(false)).unwrap();
        let results: Vec<_> = runs.iter().map(|r| r.run.result).collect();
        assert_eq!(results, [RunResult::Ok, RunResult::Ok], "{runs:?}");
        assert!(runs[0].run.message.contains("sync/src up to date"));
        assert!(runs[1].run.message.contains("received: 1 changes"));
        assert_eq!(fs::read(root.join("other/a.txt")).unwrap(), b"hello");
        let status = status(&s.paths).unwrap();
        let last = status[0].last.as_ref().unwrap().last_start;
        assert_eq!(status[0].next_run.duration_since(last).as_secs(), 900);
    }

    #[test]
    fn run_stops_when_asked() {
        let s = setup(JOBS);
        let daemon = Daemon::start(s.paths.clone()).unwrap();
        let stop = AtomicBool::new(true);
        daemon.run(&stop).unwrap();
        let log = fs::read_to_string(s.paths.log_file()).unwrap();
        assert!(log.contains("started with 3 jobs") && log.contains("stopped"));
    }
}
