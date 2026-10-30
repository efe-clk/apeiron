//! Keeping replicas of a web server's repository up to date
//! (REPLICATION.md §4): every ref is pushed to each replica soon after a
//! backup and whenever the refs have changed.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, Weak};
use std::time::Duration;

use apeiron::{RefInfo, RemoteRequest, Repository};
use serde_json::json;

use crate::audit::{Actor, Audit};
use crate::{App, Mode, lock};

/// How often the refs are compared with those last replicated.
const EVERY: Duration = Duration::from_secs(30);

/// Wakes the thread that pushes to the replicas.
pub(crate) struct Replicator {
    nudge: Mutex<Sender<()>>,
}

impl Replicator {
    /// Starts the thread for the site `app` will be; it ends when the site
    /// does, and its first round waits for [`Replicator::nudge`]. `None`
    /// when there are no replicas.
    pub(crate) fn start(remotes: Vec<String>, app: Weak<App>) -> Option<Replicator> {
        if remotes.is_empty() {
            return None;
        }
        let (nudge, woken) = mpsc::channel();
        std::thread::Builder::new()
            .name("apeiron-replicate".to_owned())
            .spawn(move || run(&remotes, &app, &woken))
            .ok()?;
        Some(Replicator {
            nudge: Mutex::new(nudge),
        })
    }

    /// Replicates now, without waiting for the next round.
    pub(crate) fn nudge(&self) {
        let _ = lock(&self.nudge).send(());
    }
}

fn run(remotes: &[String], app: &Weak<App>, woken: &Receiver<()>) {
    // The refs as last pushed to every replica.
    let mut done: Option<Vec<RefInfo>> = None;
    // The last failure recorded per replica, so a lasting one is recorded
    // once.
    let mut failing: HashMap<&str, String> = HashMap::new();
    loop {
        if let Err(RecvTimeoutError::Disconnected) = woken.recv_timeout(EVERY) {
            return;
        }
        let Some(app) = app.upgrade() else { return };
        let (Mode::Web(web), Ok(repo)) = (&app.mode, app.repo()) else {
            return;
        };
        let Ok(refs) = repo.refs() else { continue };
        if done.as_ref() == Some(&refs) {
            continue;
        }
        let mut all = true;
        for remote in remotes {
            let (outcome, detail) = push(&repo, remote);
            if outcome != "ok" {
                all = false;
                let message = detail.to_string();
                if failing.get(remote.as_str()) == Some(&message) {
                    continue;
                }
                failing.insert(remote, message);
            } else {
                failing.remove(remote.as_str());
            }
            record(&web.audit, detail, &outcome);
        }
        if all {
            done = Some(refs);
        }
    }
}

/// Pushes every ref to `remote`: `ok`, `conflict` when a ref was left, or
/// the kind of the error; and the detail for the audit log.
fn push(repo: &Repository, remote: &str) -> (String, serde_json::Value) {
    let request = RemoteRequest {
        remote: remote.to_owned(),
    };
    match repo.push_all(request).and_then(apeiron::Job::wait) {
        Ok(report) => {
            let objects: u64 = report
                .refs
                .iter()
                .filter_map(|r| r.report.as_ref())
                .map(|r| r.objects)
                .sum();
            let conflicts: Vec<&str> = report.conflicts().map(|r| r.name.as_str()).collect();
            let outcome = if conflicts.is_empty() {
                "ok"
            } else {
                "conflict"
            };
            let detail = json!({
                "remote": remote, "refs": report.refs.len(), "objects": objects,
                "conflicts": conflicts,
            });
            (outcome.to_owned(), detail)
        }
        Err(e) => (
            crate::outcome(Err(&e)),
            json!({"remote": remote, "error": e.message}),
        ),
    }
}

/// Records a round; a log that cannot be written stops nothing, the next
/// round tries again.
fn record(audit: &Audit, detail: serde_json::Value, outcome: &str) {
    let _ = audit.record(&Actor::default(), "replicate", detail, outcome);
}
