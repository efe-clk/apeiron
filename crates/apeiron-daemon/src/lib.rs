//! The background service of Apeiron (docs/DAEMON.md): jobs from a jobs
//! file, run on a schedule through the public API.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod daemon;
mod jobs;
mod metrics;
mod notify;
mod schedule;
pub mod sources;
mod state;
mod watch;
mod webhook;

use std::io;
use std::path::PathBuf;

pub use daemon::{
    Daemon, JobStatus, Paths, RunRecord, WebhookTest, open_external, status, test_webhooks,
};
pub use jobs::{Action, Every, Job, Scope, load_jobs, load_scoped, save_jobs, save_scoped};
pub use metrics::{CONTENT_TYPE, Metrics, Sample, job_metrics, metrics, serve_metrics};
pub use schedule::next_run;
pub use state::{JobState, RunResult, State};
pub use webhook::{Event, Format, Webhook, load_webhooks};

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
    #[error("cannot lower the priority: {0}")]
    Priority(#[source] io::Error),
    /// Preparing an application-consistent source failed (CONSISTENT.md).
    #[error("{0}")]
    Source(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Lowers the CPU and disk priority of the jobs (DAEMON.md §2). Call it
/// before [`Daemon::start`], since on Linux only the threads started
/// afterwards run lower.
pub fn lower_priority() -> Result<()> {
    apeiron_sys::lower_priority().map_err(Error::Priority)
}

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
