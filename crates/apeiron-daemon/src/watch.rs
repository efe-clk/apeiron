//! Watching the folders of jobs with `on_change` (DAEMON.md §4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::jobs::Job;

/// When the folder of a job last changed, since its last run.
type Mark = Arc<Mutex<Option<Instant>>>;

/// The watchers of the jobs that run on change, by job name.
#[derive(Default)]
pub(crate) struct Watches {
    watched: HashMap<String, (Duration, Mark, RecommendedWatcher)>,
}

impl Watches {
    /// Watches the folder of every job with `on_change`; returns the jobs
    /// whose folder cannot be watched, with the reason.
    pub(crate) fn new(jobs: &[Job]) -> (Self, Vec<(String, String)>) {
        let mut watches = Watches::default();
        let mut failed = Vec::new();
        for job in jobs {
            let (Some(quiet), Some(folder)) = (job.on_change, job.folder()) else {
                continue;
            };
            match watch(folder, &job.repo) {
                Ok((mark, watcher)) => {
                    watches
                        .watched
                        .insert(job.name.clone(), (quiet, mark, watcher));
                }
                Err(e) => failed.push((job.name.clone(), e.to_string())),
            }
        }
        (watches, failed)
    }

    /// The jobs whose folder changed and has then been quiet long enough;
    /// their marks are cleared, so a change during the run counts again.
    pub(crate) fn take_ready(&self, now: Instant) -> Vec<String> {
        let mut ready = Vec::new();
        for (name, (quiet, mark, _)) in &self.watched {
            let mut changed = mark.lock().unwrap_or_else(PoisonError::into_inner);
            if changed.is_some_and(|at| now.duration_since(at) >= *quiet) {
                *changed = None;
                ready.push(name.clone());
            }
        }
        ready.sort();
        ready
    }

    /// Whether a job would be ready by now.
    pub(crate) fn any_ready(&self, now: Instant) -> bool {
        self.watched.values().any(|(quiet, mark, _)| {
            mark.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_some_and(|at| now.duration_since(at) >= *quiet)
        })
    }
}

/// Starts watching `folder`; changes inside the repository, when it lies in
/// the folder, are left out, since a backup writes there.
fn watch(folder: &Path, repo: &Path) -> notify::Result<(Mark, RecommendedWatcher)> {
    let mark: Mark = Arc::default();
    let marked = mark.clone();
    let repo: PathBuf = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if !event.paths.is_empty() && event.paths.iter().all(|p| p.starts_with(&repo)) {
            return;
        }
        *marked.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    })?;
    watcher.watch(folder, RecursiveMode::Recursive)?;
    Ok((mark, watcher))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{Action, Every};

    fn job(folder: &Path, repo: &Path, quiet: Duration) -> Job {
        Job {
            name: "docs".to_owned(),
            repo: repo.to_path_buf(),
            password_file: None,
            action: Action::Backup {
                folder: folder.to_path_buf(),
                ref_name: "main".to_owned(),
                message: None,
                push: Vec::new(),
            },
            every: Every::Days(1),
            at: None,
            rate: None,
            on_change: Some(quiet),
        }
    }

    /// Waits up to ten seconds for `ready` to hold.
    fn eventually(mut ready: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if ready() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn a_change_makes_the_job_ready_once_the_folder_is_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("docs");
        let repo = folder.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let quiet = Duration::from_millis(300);
        let (watches, failed) = Watches::new(&[job(&folder, &repo, quiet)]);
        assert!(failed.is_empty(), "{failed:?}");
        assert!(watches.take_ready(Instant::now()).is_empty());

        std::fs::write(folder.join("a.txt"), b"one").unwrap();
        assert!(eventually(|| watches.any_ready(Instant::now())));
        assert_eq!(watches.take_ready(Instant::now()), ["docs"]);
        // Taking it clears the mark.
        assert!(watches.take_ready(Instant::now() + quiet).is_empty());

        // Writes into the repository inside the folder do not count.
        std::fs::write(repo.join("pack"), b"data").unwrap();
        std::thread::sleep(Duration::from_secs(1));
        assert!(!watches.any_ready(Instant::now() + quiet));
    }

    #[test]
    fn a_folder_that_does_not_exist_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let (watches, failed) = Watches::new(&[job(&missing, &missing, Duration::from_secs(5))]);
        assert_eq!(failed.len(), 1);
        assert!(watches.take_ready(Instant::now()).is_empty());
    }
}
