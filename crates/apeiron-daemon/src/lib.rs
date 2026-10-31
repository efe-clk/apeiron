//! The background service of Apeiron (docs/DAEMON.md): jobs from a jobs
//! file, run on a schedule through the public API.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod daemon;
mod jobs;
mod schedule;
mod state;

use std::io;
use std::path::PathBuf;

pub use daemon::{Daemon, JobStatus, Paths, RunRecord, status};
pub use jobs::{Action, Every, Job, load_jobs, save_jobs};
pub use schedule::next_run;
pub use state::{JobState, RunResult, State};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("{}: {message}", .path.display())]
    Jobs { path: PathBuf, message: String },
    #[error("I/O error at {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("another daemon is running ({})", .0.display())]
    Locked(PathBuf),
    #[error("cannot find the {0} folder")]
    NoFolder(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

trait IoContext<T> {
    fn at(self, path: &std::path::Path) -> Result<T>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn at(self, path: &std::path::Path) -> Result<T> {
        self.map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}
