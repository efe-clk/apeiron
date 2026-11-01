//! The jobs file (DAEMON.md §3).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
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
    /// The most bytes per second moved to or from remotes, on average
    /// (`rate_kb`, DAEMON.md §3).
    pub rate: Option<u64>,
    /// For `backup` and `sync`: also run once the folder has had no change
    /// for this long after one (`on_change`, DAEMON.md §4).
    pub on_change: Option<std::time::Duration>,
}

impl Job {
    /// The folder a job reads, for watching it for changes.
    pub fn folder(&self) -> Option<&Path> {
        match &self.action {
            Action::Backup { folder, .. } | Action::Sync { folder, .. } => Some(folder),
            _ => None,
        }
    }
}

/// What the jobs of a tenant may reach (DAEMON.md §3.1): its repository
/// only, and folders inside its folders.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    pub repo: PathBuf,
    pub password_file: Option<PathBuf>,
    pub folders: Vec<PathBuf>,
}

impl Scope {
    /// Whether `folder` is one of the tenant's folders or inside one.
    fn holds(&self, folder: &Path) -> bool {
        let lexical = folder
            .components()
            .all(|c| !matches!(c, std::path::Component::ParentDir));
        lexical && folder.is_absolute() && self.folders.iter().any(|f| folder.starts_with(f))
    }
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
    repo: Option<String>,
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
    rate_kb: Option<u64>,
    on_change: Option<String>,
    remote: Option<String>,
    mode: Option<String>,
}

/// Reads the jobs file; jobs come sorted by name, the order they run in.
/// A missing file has no jobs.
pub fn load_jobs(path: &Path) -> Result<Vec<Job>> {
    load_scoped(path, None)
}

/// [`load_jobs`] for the jobs of a tenant, when `scope` is given.
pub fn load_scoped(path: &Path, scope: Option<&Scope>) -> Result<Vec<Job>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).at(path),
    };
    parse_jobs(path, &text, scope)
}

/// Replaces the jobs file atomically with `text` once it is a valid jobs
/// file, and returns its jobs; an invalid text leaves the file as it was.
pub fn save_jobs(path: &Path, text: &str) -> Result<Vec<Job>> {
    save_scoped(path, text, None)
}

/// [`save_jobs`] for the jobs of a tenant, when `scope` is given.
pub fn save_scoped(path: &Path, text: &str, scope: Option<&Scope>) -> Result<Vec<Job>> {
    let jobs = parse_jobs(path, text, scope)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).at(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    let mut file = File::create(&tmp).at(&tmp)?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .at(&tmp)?;
    drop(file);
    fs::rename(&tmp, path).at(path)?;
    Ok(jobs)
}

fn parse_jobs(path: &Path, text: &str, scope: Option<&Scope>) -> Result<Vec<Job>> {
    let invalid = |message: String| Error::Jobs {
        path: path.to_path_buf(),
        message,
    };
    let raw: RawFile = toml::from_str(text).map_err(|e| invalid(e.to_string()))?;
    raw.job
        .into_iter()
        .map(|(name, job)| {
            to_job(&name, job, scope).map_err(|e| invalid(format!("job {name:?}: {e}")))
        })
        .collect()
}

fn to_job(name: &str, raw: RawJob, scope: Option<&Scope>) -> std::result::Result<Job, String> {
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
    if matches!(
        action,
        Action::Verify | Action::Gc | Action::RestoreTest { .. }
    ) {
        not_here("rate_kb", raw.rate_kb.is_some())?;
    }
    let rate = match raw.rate_kb {
        Some(0) => return Err("`rate_kb` must be at least 1".to_owned()),
        Some(kb) => Some(kb.saturating_mul(1024)),
        None => None,
    };
    let on_change = match (&raw.on_change, &action) {
        (None, _) => None,
        (Some(text), Action::Backup { .. } | Action::Sync { .. }) => Some(parse_quiet(text)?),
        (Some(_), _) => not_here("on_change", true).map(|()| None)?,
    };
    let every = parse_every(&raw.every)?;
    let at = raw.at.as_deref().map(parse_at).transpose()?;
    if at.is_some() && matches!(every, Every::Hours(_) | Every::Minutes(_)) {
        return Err("`at` needs `every` in days or weeks".to_owned());
    }
    let (repo, password_file) = match scope {
        None => (
            expand(raw.repo.as_deref().ok_or("`repo` is missing")?),
            raw.password_file.as_deref().map(expand),
        ),
        // A tenant's jobs work on its repository, opened as the server
        // opens it, and read only its folders.
        Some(scope) => {
            if raw.repo.is_some() || raw.password_file.is_some() {
                return Err(
                    "a tenant's jobs use its repository; leave out `repo` and `password_file`"
                        .to_owned(),
                );
            }
            if let Action::Backup { folder, .. } | Action::Sync { folder, .. } = &action
                && !scope.holds(folder)
            {
                return Err(format!(
                    "{} is not inside a folder of this tenant",
                    folder.display()
                ));
            }
            (scope.repo.clone(), scope.password_file.clone())
        }
    };
    Ok(Job {
        name: name.to_owned(),
        repo,
        password_file,
        action,
        every,
        at,
        rate,
        on_change,
    })
}

/// A quiet time such as `30s`, `2m` or `1h`: from 5 seconds to a day.
fn parse_quiet(text: &str) -> std::result::Result<std::time::Duration, String> {
    let bad = || format!("`on_change = {text:?}`: use a number with s, m or h, from 5s to 24h");
    let (number, unit) = text.split_at(text.len().saturating_sub(1));
    let n: u64 = number.parse().map_err(|_| bad())?;
    let seconds = match unit {
        "s" => n,
        "m" => n.saturating_mul(60),
        "h" => n.saturating_mul(3600),
        _ => return Err(bad()),
    };
    if !(5..=86_400).contains(&seconds) {
        return Err(bad());
    }
    Ok(std::time::Duration::from_secs(seconds))
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
        parse_jobs(Path::new("jobs.toml"), text, None).map_err(|e| e.to_string())
    }

    /// A tenant's jobs use its repository and only its folders.
    #[cfg(unix)]
    #[test]
    fn a_tenants_jobs_stay_inside_it() {
        let scope = Scope {
            repo: PathBuf::from("/srv/acme"),
            password_file: Some(PathBuf::from("/etc/acme.key")),
            folders: vec![PathBuf::from("/data/acme")],
        };
        let parse = |text: &str| {
            parse_jobs(Path::new("jobs.toml"), text, Some(&scope)).map_err(|e| e.to_string())
        };
        let jobs = parse(
            "[job.home]\ndo = \"backup\"\nfolder = \"/data/acme/home\"\nref = \"backup/home\"\nevery = \"day\"",
        )
        .unwrap();
        assert_eq!(jobs[0].repo, Path::new("/srv/acme"));
        assert_eq!(
            jobs[0].password_file.as_deref(),
            Some(Path::new("/etc/acme.key"))
        );
        for bad in [
            "[job.x]\ndo = \"backup\"\nfolder = \"/data/other\"\nevery = \"day\"",
            "[job.x]\ndo = \"backup\"\nfolder = \"/data/acme/../other\"\nevery = \"day\"",
            "[job.x]\nrepo = \"/srv/other\"\ndo = \"verify\"\nevery = \"day\"",
            "[job.x]\npassword_file = \"/p\"\ndo = \"verify\"\nevery = \"day\"",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        // Without a scope a job names its repository.
        assert!(
            super::parse_jobs(
                Path::new("j"),
                "[job.x]\ndo = \"verify\"\nevery = \"day\"",
                None
            )
            .is_err()
        );
    }

    #[test]
    fn only_a_valid_file_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config/jobs.toml");
        let good = "[job.check]\nrepo = \"/r\"\ndo = \"verify\"\nevery = \"week\"\n";
        assert_eq!(save_jobs(&path, good).unwrap()[0].name, "check");
        let bad = save_jobs(&path, "[job.check]\ndo = \"dance\"\n").unwrap_err();
        assert!(matches!(bad, Error::Jobs { .. }), "{bad}");
        assert_eq!(fs::read_to_string(&path).unwrap(), good);
        assert_eq!(load_jobs(&path).unwrap().len(), 1);
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
            (
                "do = \"gc\"\nevery = \"day\"\non_change = \"1m\"",
                "`on_change` does not apply",
            ),
            (
                "do = \"backup\"\nfolder = \"/f\"\nevery = \"day\"\non_change = \"1s\"",
                "from 5s to 24h",
            ),
            (
                "do = \"gc\"\nevery = \"day\"\nrate_kb = 100",
                "`rate_kb` does not apply",
            ),
            (
                "do = \"push\"\npush = [\"nas\"]\nevery = \"day\"\nrate_kb = 0",
                "at least 1",
            ),
        ];
        for (body, expected) in bad {
            let error = parse(&format!("[job.x]\nrepo = \"/r\"\n{body}")).unwrap_err();
            assert!(error.contains(expected), "{body}: {error}");
        }
        let jobs = parse(
            "[job.x]\nrepo = \"/r\"\ndo = \"push\"\npush = [\"nas\"]\nevery = \"day\"\nrate_kb = 512",
        )
        .unwrap();
        assert_eq!(jobs[0].rate, Some(512 * 1024));
        let jobs = parse(
            "[job.x]\nrepo = \"/r\"\ndo = \"backup\"\nfolder = \"/f\"\nevery = \"day\"\non_change = \"2m\"",
        )
        .unwrap();
        assert_eq!(jobs[0].on_change, Some(std::time::Duration::from_secs(120)));
        assert_eq!(jobs[0].folder(), Some(Path::new("/f")));
        let error = parse("[job.Big]\nrepo = \"/r\"\ndo = \"gc\"\nevery = \"day\"").unwrap_err();
        assert!(error.contains("job \"Big\""), "{error}");
    }
}
