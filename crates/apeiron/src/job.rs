//! Long operations running on their own thread (API.md §8.5).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use apeiron_services::{Control, Progress};

use crate::error::{Error, ErrorKind, Result};

/// A running long operation. Read its progress with [`Job::progress`], stop
/// it with [`Job::cancel`] and collect the result with [`Job::wait`].
/// Dropping a job that has not finished cancels it.
#[derive(Debug)]
pub struct Job<T> {
    ctl: Control,
    result: Receiver<Result<T>>,
    done: Option<Result<T>>,
    thread: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> Job<T> {
    pub(crate) fn spawn(
        name: &str,
        f: impl FnOnce(&Control) -> Result<T> + Send + 'static,
    ) -> Result<Self> {
        let ctl = Control::new();
        let (tx, rx) = mpsc::channel();
        let worker_ctl = ctl.clone();
        let thread = thread::Builder::new()
            .name(format!("apeiron-{name}"))
            .spawn(move || {
                let result =
                    catch_unwind(AssertUnwindSafe(|| f(&worker_ctl))).unwrap_or_else(|_| {
                        Err(Error::new(
                            ErrorKind::Internal,
                            "the operation panicked; please report this bug",
                        ))
                    });
                // The receiver is gone only if the job was dropped; nobody wants the result then.
                let _ = tx.send(result);
            })
            .map_err(|e| Error::new(ErrorKind::Internal, format!("cannot start a thread: {e}")))?;
        Ok(Job {
            ctl,
            result: rx,
            done: None,
            thread: Some(thread),
        })
    }
}

impl<T> Job<T> {
    /// A copy of the current progress.
    pub fn progress(&self) -> Progress {
        self.ctl.progress()
    }

    /// Asks the job to stop. It then finishes with [`ErrorKind::Cancelled`],
    /// unless it was already about to succeed.
    pub fn cancel(&self) {
        self.ctl.cancel();
    }

    pub fn is_finished(&mut self) -> bool {
        self.poll(Some(Duration::ZERO))
    }

    /// Waits for the result.
    pub fn wait(mut self) -> Result<T> {
        self.poll(None);
        self.take()
    }

    /// Waits at most `timeout`; returns `None` if the job is still running.
    /// The result is returned only once.
    pub fn wait_timeout(&mut self, timeout: Duration) -> Option<Result<T>> {
        if self.poll(Some(timeout)) {
            Some(self.take())
        } else {
            None
        }
    }

    /// Moves a finished result into `done`; `None` waits without limit.
    fn poll(&mut self, timeout: Option<Duration>) -> bool {
        if self.done.is_some() {
            return true;
        }
        let received = match timeout {
            None => self
                .result
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
            Some(t) => self.result.recv_timeout(t),
        };
        match received {
            Ok(result) => self.done = Some(result),
            Err(RecvTimeoutError::Timeout) => return false,
            Err(RecvTimeoutError::Disconnected) => {
                self.done = Some(Err(Error::new(
                    ErrorKind::Internal,
                    "the result was already taken",
                )));
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        true
    }

    fn take(&mut self) -> Result<T> {
        self.done.take().unwrap_or_else(|| {
            Err(Error::new(
                ErrorKind::Internal,
                "the result was already taken",
            ))
        })
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.ctl.cancel();
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_stops_a_running_job() {
        let mut job = Job::spawn("test", |ctl| {
            loop {
                ctl.check()
                    .map_err(|_| Error::new(ErrorKind::Cancelled, "cancelled"))?;
                ctl.update(|p| p.items_done += 1);
                thread::sleep(Duration::from_millis(1));
            }
        })
        .unwrap();
        assert!(job.wait_timeout(Duration::from_millis(20)).is_none());
        assert!(!job.is_finished());
        job.cancel();
        let result: Result<()> = job.wait();
        assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
    }

    #[test]
    fn result_is_returned_once_and_progress_is_visible() {
        let mut job = Job::spawn("test", |ctl| {
            ctl.update(|p| p.items_done = 7);
            Ok(42)
        })
        .unwrap();
        assert_eq!(
            job.wait_timeout(Duration::from_secs(10)).unwrap().unwrap(),
            42
        );
        assert_eq!(job.progress().items_done, 7);
        assert!(job.wait_timeout(Duration::ZERO).unwrap().is_err());
    }

    #[test]
    fn panic_becomes_internal_error() {
        let job: Job<()> = Job::spawn("test", |_| panic!("boom")).unwrap();
        assert_eq!(job.wait().unwrap_err().kind, ErrorKind::Internal);
    }

    #[test]
    fn dropping_a_job_cancels_it() {
        let ctl_seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen = ctl_seen.clone();
        let job: Job<()> = Job::spawn("test", move |ctl| {
            *seen.lock().unwrap() = Some(ctl.clone());
            while !ctl.is_cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        })
        .unwrap();
        while ctl_seen.lock().unwrap().is_none() {
            thread::sleep(Duration::from_millis(1));
        }
        drop(job);
        assert!(ctl_seen.lock().unwrap().as_ref().unwrap().is_cancelled());
    }
}
