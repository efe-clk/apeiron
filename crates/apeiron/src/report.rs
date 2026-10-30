//! Compliance reports: what a repository keeps, how fresh and whole it is,
//! how it is protected and whether its copies are current (REPORTS.md).

use std::time::{SystemTime, UNIX_EPOCH};

use apeiron_core as core;
use apeiron_services as services;
use serde::{Deserialize, Serialize};

use crate::repo::open_store;
use crate::{
    Error, KeyKind, LogRequest, ObjectId, Phase, Repository, Result, RetentionRule, Time,
    VerifyReport,
};

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// What a report covers (REPORTS.md §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportRequest {
    /// Read and check every stored byte, as `verify` does.
    #[serde(default)]
    pub verify: bool,
    /// Compare every ref with each remote; needs the remotes to answer.
    #[serde(default)]
    pub remotes: bool,
    /// A ref whose newest snapshot is older than this many days is stale; 0
    /// checks nothing.
    #[serde(default = "default_stale_days")]
    pub stale_days: u32,
}

fn default_stale_days() -> u32 {
    7
}

impl Default for ReportRequest {
    fn default() -> Self {
        ReportRequest {
            verify: false,
            remotes: false,
            stale_days: default_stale_days(),
        }
    }
}

/// A compliance report of one repository (REPORTS.md §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceReport {
    /// When the report was made.
    pub time: Time,
    pub repository: RepositoryFacts,
    pub refs: Vec<RefFacts>,
    pub retention: Vec<RetentionRule>,
    /// Present when the request asked to verify.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<VerifyReport>,
    /// Every remote, and with `remotes` whether it answered.
    pub remotes: Vec<RemoteFacts>,
    /// What needs attention, most serious first.
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryFacts {
    pub path: std::path::PathBuf,
    pub format_version: u32,
    pub encrypted: bool,
    pub passwords: usize,
    pub recovery_keys: usize,
    /// Files found damaged when the repository was opened.
    pub damaged_files: Vec<std::path::PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefFacts {
    pub name: String,
    /// The snapshot the ref points to; its id fixes every byte under it and
    /// all its history.
    pub snapshot: ObjectId,
    /// The time of that snapshot.
    pub time: Time,
    /// Snapshots in its history, following first parents.
    pub snapshots: u64,
    /// Days a retention rule keeps it, if one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
    pub stale: bool,
    /// Its state on each remote, with `remotes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<CopyState>,
}

/// How a ref stands on a remote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyState {
    pub remote: String,
    pub state: CopyKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyKind {
    /// The remote has the same snapshot.
    Current,
    /// The remote has an older snapshot of the same history.
    Behind,
    /// The remote has no such ref.
    Missing,
    /// The remote has snapshots this copy does not.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFacts {
    pub name: String,
    pub url: String,
    /// With `remotes`: the error if it did not answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}

/// Something that needs attention (REPORTS.md §4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub level: Level,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Data is damaged, missing on a copy or cannot be checked.
    Problem,
    /// A weaker protection than expected.
    Warning,
}

impl ComplianceReport {
    /// Whether any finding is a problem.
    pub fn has_problems(&self) -> bool {
        self.findings.iter().any(|f| f.level == Level::Problem)
    }
}

fn now() -> Time {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    Time { seconds, nanos: 0 }
}

/// Makes the report of `repo`; runs inside a job.
pub(crate) fn make(
    repo: &Repository,
    request: &ReportRequest,
    ctl: &services::Control,
) -> Result<ComplianceReport> {
    let time = now();
    let info = repo.info();
    let keys = if info.encrypted {
        repo.keys()?
    } else {
        Vec::new()
    };
    let count = |kind: KeyKind| keys.iter().filter(|k| k.kind == kind).count();
    let repository = RepositoryFacts {
        path: info.path,
        format_version: info.format_version,
        encrypted: info.encrypted,
        passwords: count(KeyKind::Password),
        recovery_keys: count(KeyKind::Recovery),
        damaged_files: info.damaged_files,
    };
    let retention = repo.retention()?;
    let mut findings = Vec::new();
    let mut problem = |text: String| {
        findings.push(Finding {
            level: Level::Problem,
            text,
        })
    };

    let mut refs = Vec::new();
    for r in repo.refs()? {
        ctl.check()?;
        let mut snapshots = 0;
        let mut start = Some(r.snapshot);
        let mut newest = None;
        while let Some(from) = start {
            let page = repo.log(&LogRequest {
                start: from.to_string(),
                limit: 1024,
            })?;
            newest = newest.or_else(|| page.snapshots.first().map(|s| s.time));
            snapshots += page.snapshots.len() as u64;
            start = page.next;
        }
        let time_of = newest.unwrap_or(time);
        let stale = request.stale_days > 0
            && time.seconds - time_of.seconds > i64::from(request.stale_days) * SECONDS_PER_DAY;
        refs.push(RefFacts {
            retention_days: retention
                .iter()
                .filter(|rule| rule.matches(&r.name))
                .map(|rule| rule.days)
                .max(),
            name: r.name,
            snapshot: r.snapshot,
            time: time_of,
            snapshots,
            stale,
            copies: Vec::new(),
        });
    }

    let verify = if request.verify {
        ctl.update(|p| p.phase = Phase::Verifying);
        let report = repo.view()?.verify()?;
        if !report.is_ok() {
            problem(format!(
                "{} damaged files and {} damaged objects; run `apeiron verify`",
                report.damaged_files.len(),
                report.damaged_objects.len()
            ));
        }
        Some(report)
    } else {
        None
    };
    if !repository.damaged_files.is_empty() {
        problem(format!(
            "{} files were found damaged when the repository was opened",
            repository.damaged_files.len()
        ));
    }

    let mut remotes = Vec::new();
    for remote in repo.remotes()? {
        let mut facts = RemoteFacts {
            name: remote.name.clone(),
            url: remote.url.clone(),
            error: None,
        };
        if request.remotes {
            ctl.update(|p| p.phase = Phase::Copying);
            match compare(repo, &remote, &mut refs) {
                Ok(()) => {}
                Err(e) => {
                    problem(format!(
                        "the remote {} did not answer: {}",
                        remote.name, e.message
                    ));
                    facts.error = Some(e);
                }
            }
        }
        remotes.push(facts);
    }
    for r in &refs {
        for copy in &r.copies {
            let why = match copy.state {
                CopyKind::Current => continue,
                CopyKind::Behind => "is behind",
                CopyKind::Missing => "is missing",
                CopyKind::Other => "has snapshots this copy does not",
            };
            problem(format!("{} {why} on {}", r.name, copy.remote));
        }
    }

    let mut warning = |text: String| {
        findings.push(Finding {
            level: Level::Warning,
            text,
        })
    };
    for r in refs.iter().filter(|r| r.stale) {
        warning(format!(
            "{} has had no new snapshot for more than {} days",
            r.name, request.stale_days
        ));
    }
    if !repository.encrypted {
        warning("the repository is not encrypted".to_owned());
    } else if repository.recovery_keys == 0 {
        warning("there is no recovery key; a forgotten password loses everything".to_owned());
    }
    if remotes.is_empty() {
        warning("there is no remote: every backup exists in one place only".to_owned());
    }
    findings.sort_by_key(|f| f.level);
    ctl.update(|p| p.phase = Phase::Done);
    Ok(ComplianceReport {
        time,
        repository,
        refs,
        retention,
        verify,
        remotes,
        findings,
    })
}

/// Adds the state of every ref on `remote` to `refs`.
fn compare(repo: &Repository, remote: &core::Remote, refs: &mut [RefFacts]) -> Result<()> {
    let store = open_store(remote, repo.info().encrypted)?;
    for r in refs.iter_mut() {
        let state = match store.read_ref(&r.name)? {
            None => CopyKind::Missing,
            Some(id) if id == r.snapshot => CopyKind::Current,
            Some(id) => {
                let core = repo.view()?;
                if core.contains(&id) && core.is_ancestor(id, r.snapshot)? {
                    CopyKind::Behind
                } else {
                    CopyKind::Other
                }
            }
        };
        r.copies.push(CopyState {
            remote: remote.name.clone(),
            state,
        });
    }
    Ok(())
}
