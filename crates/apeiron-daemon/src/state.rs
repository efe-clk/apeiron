//! The state file: the last run of every job (DAEMON.md §6).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{IoContext, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub jobs: BTreeMap<String, JobState>,
}

/// The last run of a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobState {
    pub last_start: Timestamp,
    pub last_end: Timestamp,
    pub result: RunResult,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunResult {
    Ok,
    /// Done, but some files were skipped.
    Warnings,
    Failed,
    /// The repository was locked by another program; tried again soon.
    Locked,
}

impl State {
    /// Reads the state; a missing or unreadable file is an empty state, so
    /// that every job runs again.
    pub fn load(path: &Path) -> State {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Replaces the state file atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        // Serializing plain strings and timestamps cannot fail.
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        let tmp = path.with_extension("json.tmp");
        let mut file = File::create(&tmp).at(&tmp)?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .at(&tmp)?;
        drop(file);
        fs::rename(&tmp, path).at(path)
    }
}
