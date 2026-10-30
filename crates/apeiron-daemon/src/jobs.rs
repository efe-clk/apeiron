//! The jobs file (DAEMON.md §3).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use apeiron::SyncMode;
use serde::Deserialize;

use crate::{Error, IoContext, Result};

/// One job of the jobs file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub name: String,
    pub repo: PathBuf,
    pub password_file: Option<PathBuf>,
    pub action: Action,
    pub every: Every,
    /// Earliest local time of day `(hour, minute)`; only with [`Every::Days`].
    pub at: Option<(i8, i8)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Backup {
        folder: PathBuf,
        ref_name: String,
        message: Option<String>,
        /// Remotes to push the ref to after a successful backup.
        push: Vec<String>,
    },
    Push {
        /// `None` for every ref (`ref = "*"`, REPLICATION.md §3).
        ref_name: Option<String>,
        remotes: Vec<String>,
    },
    Pull {
        remote: String,
        /// `None` for every ref of the remote (`ref = "*"`).
        ref_name: Option<String>,
    },
    Verify,
    RestoreTest {
        ref_name: String,
        sample_bytes: u64,
    },
    Gc,
    Sync {
        folder: PathBuf,
        remote: String,
        /// `None` for `sync/<folder name>`.
        ref_name: Option<String>,
        mode: SyncMode,
    },
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::Backup { .. } => "backup",
            Action::Push { .. } => "push",
            Action::Pull { .. } => "pull",
            Action::Verify => "verify",
            Action::RestoreTest { .. } => "restore-test",
            Action::Gc => "gc",
            Action::Sync { .. } => "sync",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Every {
    Minutes(u32),
    Hours(u32),
    Days(u32),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    job: BTreeMap<String, RawJob>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    repo: String,
    password_file: Option<String>,
    #[serde(rename = "do")]
    action: String,
    folder: Option<String>,
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    message: Option<String>,
    push: Option<Vec<String>>,
    every: String,
    at: Option<String>,
    sample_mb: Option<u64>,
    remote: Option<String>,
    mode: Option<String>,
}

/// Reads the jobs file; jobs come sorted by name, the order they run in.
/// A missing file has no jobs.
pub fn load_jobs(path: &Path) -> Result<Vec<Job>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).at(path),
    };
    parse_jobs(path, &text)
}

fn parse_jobs(path: &Path, text: &str) -> Result<Vec<Job>> {
    let invalid = |message: String| Error::Jobs {
        path: path.to_path_buf(),
        message,
    };
    let raw: RawFile = toml::from_str(text).map_err(|e| invalid(e.to_string()))?;
    raw.job
        .into_iter()
        .map(|(name, job)| to_job(&name, job).map_err(|e| invalid(format!("job {name:?}: {e}"))))
        .collect()
}

fn to_job(name: &str, raw: RawJob) -> std::result::Result<Job, String> {
    let valid_name = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '-'));
    if !valid_name {
        return Err("names use only a-z, 0-9, `.`, `_` and `-`".to_owned());
    }
    let ref_name = raw.ref_name.clone().unwrap_or_else(|| "main".to_owned());
    if raw.action != "sync" {
        not_here_for(&raw.action, "mode", raw.mode.is_some())?;
    }
    if raw.action != "sync" && raw.action != "pull" {
        not_here_for(&raw.action, "remote", raw.remote.is_some())?;
    }
    // `ref = "*"` names every ref, for `push` and `pull` only.
    let every_ref = ref_name == "*";
    if every_ref && raw.action != "push" && raw.action != "pull" {
        return Err(format!(
            "`ref = \"*\"` does not apply to `do = {:?}`",
            raw.action
        ));
    }
    let some_ref = (!every_ref).then(|| ref_name.clone());
    let not_here = |key: &str, present: bool| {
        if present {
            Err(format!("`{key}` does not apply to `do = {:?}`", raw.action))
        } else {
            Ok(())
        }
    };
    let action = match raw.action.as_str() {
        "backup" => {
            not_here("sample_mb", raw.sample_mb.is_some())?;
            let folder = raw.folder.as_deref().ok_or("`folder` is missing")?;
            Action::Backup {
                folder: expand(folder),
                ref_name,
                message: raw.message.clone(),
                push: raw.push.clone().unwrap_or_default(),
            }
        }
        "push" => {
            not_here("folder", raw.folder.is_some())?;
            not_here("message", raw.message.is_some())?;
            not_here("sample_mb", raw.sample_mb.is_some())?;
            let remotes = raw.push.clone().filter(|p| !p.is_empty());
            Action::Push {
                ref_name: some_ref,
                remotes: remotes.ok_or("`push` must name at least one remote")?,
            }
        }
        "pull" => {
            for (key, present) in [
                ("folder", raw.folder.is_some()),
                ("message", raw.message.is_some()),
                ("push", raw.push.is_some()),
                ("sample_mb", raw.sample_mb.is_some()),
            ] {
                not_here(key, present)?;
            }
            Action::Pull {
                remote: raw.remote.clone().ok_or("`remote` is missing")?,
                ref_name: some_ref,
            }
        }
        "verify" | "gc" => {
            for (key, present) in [
                ("folder", raw.folder.is_some()),
                ("ref", raw.ref_name.is_some()),
                ("message", raw.message.is_some()),
                ("push", raw.push.is_some()),
                ("sample_mb", raw.sample_mb.is_some()),
            ] {
                not_here(key, present)?;
            }
            if raw.action == "gc" {
                Action::Gc
            } else {
                Action::Verify
            }
        }
        "restore-test" => {
            not_here("folder", raw.folder.is_some())?;
            not_here("message", raw.message.is_some())?;
            not_here("push", raw.push.is_some())?;
            Action::RestoreTest {
                ref_name,
                sample_bytes: raw.sample_mb.unwrap_or(100).saturating_mul(1024 * 1024),
            }
        }
        "sync" => {
            not_here("message", raw.message.is_some())?;
            not_here("push", raw.push.is_some())?;
            not_here("sample_mb", raw.sample_mb.is_some())?;
            let mode = match raw.mode.as_deref() {
                None | Some("two-way") => SyncMode::TwoWay,
                Some("send") => SyncMode::Send,
                Some("receive") => SyncMode::Receive,
                Some(other) => {
                    return Err(format!("`mode = {other:?}`: use two-way, send or receive"));
                }
            };
            Action::Sync {
                folder: expand(raw.folder.as_deref().ok_or("`folder` is missing")?),
                remote: raw.remote.clone().ok_or("`remote` is missing")?,
                ref_name: raw.ref_name.clone(),
                mode,
            }
        }
        other => {
            return Err(format!(
                "unknown `do = {other:?}`; use backup, push, pull, verify, restore-test, gc or sync"
            ));
        }
    };
    let every = parse_every(&raw.every)?;
    let at = raw.at.as_deref().map(parse_at).transpose()?;
    if at.is_some() && matches!(every, Every::Hours(_) | Every::Minutes(_)) {
        return Err("`at` needs `every` in days or weeks".to_owned());
    }
    Ok(Job {
        name: name.to_owned(),
        repo: expand(&raw.repo),
        password_file: raw.password_file.as_deref().map(expand),
        action,
        every,
        at,
    })
}

fn parse_every(text: &str) -> std::result::Result<Every, String> {
    let bad = || format!("`every = {text:?}`: use hour, day, week, or a number with m, h or d");
    let every = match text {
        "hour" => Every::Hours(1),
        "day" => Every::Days(1),
        "week" => Every::Days(7),
        _ => {
            let (number, unit) = text.split_at(text.len().saturating_sub(1));
            let n: u32 = number.parse().map_err(|_| bad())?;
            if n == 0 {
                return Err(bad());
            }
            match unit {
                "m" => Every::Minutes(n),
                "h" => Every::Hours(n),
                "d" => Every::Days(n),
                _ => return Err(bad()),
            }
        }
    };
    Ok(every)
}

fn parse_at(text: &str) -> std::result::Result<(i8, i8), String> {
    let bad = || format!("`at = {text:?}`: use HH:MM, such as 02:30");
    let (hour, minute) = text.split_once(':').ok_or_else(bad)?;
    if hour.len() != 2 || minute.len() != 2 {
        return Err(bad());
    }
    let hour: i8 = hour.parse().map_err(|_| bad())?;
    let minute: i8 = minute.parse().map_err(|_| bad())?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return Err(bad());
    }
    Ok((hour, minute))
}

fn not_here_for(action: &str, key: &str, present: bool) -> std::result::Result<(), String> {
    if present {
        Err(format!("`{key}` does not apply to `do = {action:?}`"))
    } else {
        Ok(())
    }
}

/// Replaces a leading `~` with the home folder.
fn expand(path: &str) -> PathBuf {
    let home = || dirs::home_dir().unwrap_or_default();
    match path.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') || rest.starts_with('\\') => home().join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> std::result::Result<Vec<Job>, String> {
        parse_jobs(Path::new("jobs.toml"), text).map_err(|e| e.to_string())
    }

    #[test]
    fn jobs_are_read_with_defaults_and_sorted_by_name() {
        let jobs = parse(
            r#"
            [job.documents]
            repo = "/r"
            password_file = "~/pw"
            do = "backup"
            folder = "~/Documents"
            every = "day"
            at = "02:30"
            push = ["nas"]

            [job.check]
            repo = "/r"
            do = "verify"
            every = "week"

            [job.test]
            repo = "/r"
            do = "restore-test"
            ref = "backup/x"
            every = "6h"
            sample_mb = 5
            "#,
        )
        .unwrap();
        let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, ["check", "documents", "test"]);
        let home = dirs::home_dir().unwrap();
        let documents = &jobs[1];
        assert_eq!(documents.password_file, Some(home.join("pw")));
        assert_eq!(
            documents.action,
            Action::Backup {
                folder: home.join("Documents"),
                ref_name: "main".to_owned(),
                message: None,
                push: vec!["nas".to_owned()],
            }
        );
        assert_eq!(
            (documents.every, documents.at),
            (Every::Days(1), Some((2, 30)))
        );
        assert_eq!(jobs[0].every, Every::Days(7));
        assert_eq!(
            (jobs[2].every, &jobs[2].action),
            (
                Every::Hours(6),
                &Action::RestoreTest {
                    ref_name: "backup/x".to_owned(),
                    sample_bytes: 5 * 1024 * 1024
                }
            )
        );
        assert!(parse("").unwrap().is_empty());

        let jobs = parse(
            r#"
            [job.mirror]
            repo = "/r"
            do = "push"
            ref = "*"
            push = ["dr"]
            every = "15m"

            [job.standby]
            repo = "/s"
            do = "pull"
            remote = "origin"
            ref = "*"
            every = "15m"

            [job.one]
            repo = "/s"
            do = "pull"
            remote = "origin"
            every = "1h"
            "#,
        )
        .unwrap();
        let actions: Vec<&Action> = jobs.iter().map(|j| &j.action).collect();
        assert_eq!(
            actions,
            [
                &Action::Push {
                    ref_name: None,
                    remotes: vec!["dr".to_owned()]
                },
                &Action::Pull {
                    remote: "origin".to_owned(),
                    ref_name: Some("main".to_owned())
                },
                &Action::Pull {
                    remote: "origin".to_owned(),
                    ref_name: None
                },
            ]
        );
    }

    #[test]
    fn mistakes_are_reported_with_the_job_name() {
        let bad = [
            ("do = \"backup\"\nevery = \"day\"", "`folder` is missing"),
            ("do = \"verify\"\nevery = \"0d\"", "use hour, day, week"),
            (
                "do = \"verify\"\nevery = \"day\"\nat = \"2:30\"",
                "use HH:MM",
            ),
            (
                "do = \"verify\"\nevery = \"hour\"\nat = \"02:30\"",
                "`at` needs",
            ),
            (
                "do = \"verify\"\nevery = \"day\"\nref = \"x\"",
                "`ref` does not apply",
            ),
            ("do = \"push\"\nevery = \"day\"", "at least one remote"),
            (
                "do = \"sync\"\nfolder = \"/f\"\nevery = \"15m\"",
                "`remote` is missing",
            ),
            (
                "do = \"sync\"\nfolder = \"/f\"\nremote = \"r\"\nmode = \"both\"\nevery = \"1h\"",
                "use two-way, send or receive",
            ),
            (
                "do = \"verify\"\nremote = \"r\"\nevery = \"day\"",
                "`remote` does not apply",
            ),
            ("do = \"copy\"\nevery = \"day\"", "unknown `do"),
            ("do = \"pull\"\nevery = \"day\"", "`remote` is missing"),
            (
                "do = \"verify\"\nref = \"*\"\nevery = \"day\"",
                "does not apply",
            ),
            (
                "do = \"backup\"\nfolder = \"/f\"\nref = \"*\"\nevery = \"day\"",
                "`ref = \"*\"` does not apply",
            ),
            ("do = \"gc\"\nevery = \"day\"\ncolour = 1", "unknown field"),
        ];
        for (body, expected) in bad {
            let error = parse(&format!("[job.x]\nrepo = \"/r\"\n{body}")).unwrap_err();
            assert!(error.contains(expected), "{body}: {error}");
        }
        let error = parse("[job.Big]\nrepo = \"/r\"\ndo = \"gc\"\nevery = \"day\"").unwrap_err();
        assert!(error.contains("job \"Big\""), "{error}");
    }
}
