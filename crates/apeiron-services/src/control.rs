//! Progress, cancellation and warnings shared by long operations.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::name::Name;

/// What a long operation is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Starting,
    /// Counting what will be done, to know the totals.
    Scanning,
    Storing,
    Restoring,
    Exporting,
    Verifying,
    /// Making the result durable and moving refs.
    Finishing,
    Done,
}

/// A copy of the progress of a long operation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub phase: Phase,
    pub items_done: u64,
    pub items_total: Option<u64>,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    /// The file being worked on.
    pub current: Option<Name>,
}

/// Shared between a running operation and whoever watches it.
#[derive(Clone, Debug, Default)]
pub struct Control {
    cancelled: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
}

impl Control {
    pub fn new() -> Self {
        Control::default()
    }

    /// Asks the operation to stop at the next safe point.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Returns [`Error::Cancelled`] once cancellation was requested.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    pub fn progress(&self) -> Progress {
        self.lock().clone()
    }

    pub fn update(&self, f: impl FnOnce(&mut Progress)) {
        f(&mut self.lock());
    }

    pub(crate) fn set_phase(&self, phase: Phase) {
        self.update(|p| p.phase = phase);
    }

    fn lock(&self) -> MutexGuard<'_, Progress> {
        // Progress is plain data; a panic elsewhere cannot leave it unusable.
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A problem that did not stop the operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    pub path: Name,
    pub kind: WarningKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WarningKind {
    /// The file or folder could not be read and was skipped.
    Unreadable,
    /// The file changed while it was read; it was stored as read.
    ChangedWhileReading,
    /// The entry type or name is not supported and was skipped.
    Unsupported,
    /// The entry or one of its attributes could not be restored.
    NotRestored,
}
