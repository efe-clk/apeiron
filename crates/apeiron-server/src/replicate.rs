//! Keeping replicas of a web server's repository up to date
//! (REPLICATION.md §4): every ref is pushed to each replica soon after a
//! backup and whenever the refs have changed.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

use apeiron::{BackupRequest, Error, ErrorKind, ListRequest, RefInfo, RemoteRequest, Repository};
use serde_json::json;

use crate::audit::{Actor, Audit};
use crate::web::WebState;
use crate::{App, Mode, lock};

/// How often the refs are compared with those last replicated.
const EVERY: Duration = Duration::from_secs(30);
/// The ref the primary keeps a copy of its audit log and share links in
/// (REPLICATION.md §6.1).
pub(crate) const WEB_REF: &str = "apeiron/web";
/// The shortest time between two such copies when only the log changed.
const COPY_EVERY: Duration = Duration::from_secs(300);
/// How often a standby looks for a new copy.
const FOLLOW_EVERY: Duration = Duration::from_secs(5);

/// The files of the audit log and the links, with their times and sizes,
/// as last copied.
type Seen = Vec<(PathBuf, Option<SystemTime>, u64)>;

/// Stores the audit log and the share links as a snapshot of [`WEB_REF`]:
/// at once when the links changed, and when only the log did, once the
/// last copy is five minutes old.
fn copy_state(
    repo: &Repository,
    web: &WebState,
    last: &mut Option<(Instant, Seen)>,
) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::new(ErrorKind::Io, e.to_string());
    let mut files = web.audit.files()?;
    let shares = web.shares.file().to_path_buf();
    if shares.exists() {
        files.push(shares.clone());
    }
    let seen: Seen = files
        .iter()
        .filter_map(|f| {
            fs::metadata(f)
                .ok()
                .map(|m| (f.clone(), m.modified().ok(), m.len()))
        })
        .collect();
    let links = |s: &Seen| s.iter().find(|(f, _, _)| *f == shares).cloned();
    let wait = match last.as_ref() {
        None => false,
        Some((_, before)) if *before == seen => return Ok(()),
        Some((at, before)) => links(before) == links(&seen) && at.elapsed() < COPY_EVERY,
    };
    if wait || seen.is_empty() {
        return Ok(());
    }
    // A folder beside the log, as private as the log itself.
    let Some(dir) = web.audit.file().parent().map(|d| d.join(".web-copy")) else {
        return Ok(());
    };
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("audit")).map_err(io)?;
    let copied = (|| {
        for file in &files {
            let name = file.file_name().unwrap_or_default();
            let to = if *file == shares {
                dir.join("shares.json")
            } else {
                dir.join("audit").join(name)
            };
            fs::copy(file, to).map_err(io)?;
        }
        let mut request = BackupRequest::new(dir.clone(), WEB_REF);
        request.message = Some("the web server's audit log and share links".to_owned());
        repo.backup(request).and_then(apeiron::Job::wait)
    })();
    let _ = fs::remove_dir_all(&dir);
    copied?;
    *last = Some((Instant::now(), seen));
    Ok(())
}

/// On a standby: takes the share links of the primary's last copy
/// (REPLICATION.md §6.1) whenever [`WEB_REF`] moves, so links made on the
/// primary work here too. Ends when the site does.
pub(crate) fn follow(app: Weak<App>) {
    let _ = std::thread::Builder::new()
        .name("apeiron-follow".to_owned())
        .spawn(move || {
            let mut taken = None;
            loop {
                let Some(app) = app.upgrade() else { return };
                if let (Mode::Web(web), Ok(repo)) = (&app.mode, app.repo())
                    && let Ok(id) = repo.resolve(WEB_REF)
                    && taken != Some(id)
                {
                    let mut shares = Vec::new();
                    let request = ListRequest {
                        snapshot: id.to_string(),
                        path: "shares.json".into(),
                    };
                    let read = repo.read_file(&request, &mut shares);
                    // A copy without links leaves the copied ones as they are.
                    let done = match read {
                        Ok(_) => web.shares.take_copies(&shares),
                        Err(e) if e.kind == ErrorKind::NotFound => Ok(()),
                        Err(e) => Err(e),
                    };
                    if done.is_ok() {
                        taken = Some(id);
                    }
                }
                drop(app);
                std::thread::sleep(FOLLOW_EVERY);
            }
        });
}

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
    let mut copied = None;
    loop {
        if let Err(RecvTimeoutError::Disconnected) = woken.recv_timeout(EVERY) {
            return;
        }
        let Some(app) = app.upgrade() else { return };
        let (Mode::Web(web), Ok(repo)) = (&app.mode, app.repo()) else {
            return;
        };
        // A copy that fails is tried again next round; replicas still get
        // the refs.
        if !web.standby() {
            let _ = copy_state(&repo, web, &mut copied);
        }
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
        rate: None,
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
