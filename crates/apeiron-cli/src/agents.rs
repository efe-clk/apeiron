//! Checkpoints for AI agents (AGENTS.md): a folder recorded after each step
//! of an agent, and its work undone by turn or by session.

use std::io::Read;
use std::path::{Path, PathBuf};

use apeiron::{BackupRequest, LogRequest, Repository, SnapshotInfo, SwitchRequest};
use serde_json::Value;

use crate::{Failure, Output, watch};

/// What a checkpoint leaves out: Git's own folder, build output and what
/// `.gitignore` names (CLI.md §4.1).
pub(crate) const LEFT_OUT: [&str; 3] = [".git", "@dev", "@gitignore"];

const SESSION: &str = "checkpoint.session";
const EVENT: &str = "checkpoint.event";
/// The event of a checkpoint taken when the user asks something, which
/// starts a turn.
const TURN: &str = "UserPromptSubmit";

/// The repository of checkpoints: `--repo`, or one of the user's own that
/// is made when first needed.
pub(crate) fn repository(
    given: Option<PathBuf>,
    password_file: Option<&Path>,
) -> Result<Repository, Failure> {
    if given.is_some() {
        return crate::open(given, password_file);
    }
    let dir = dirs::data_local_dir()
        .ok_or_else(|| Failure::Usage("cannot find the data folder".to_owned()))?
        .join("apeiron")
        .join("checkpoints");
    if dir.join("config").exists() {
        Ok(Repository::open(&dir)?)
    } else {
        Ok(Repository::init(&dir)?)
    }
}

/// The ref of a folder's checkpoints.
pub(crate) fn ref_of(folder: &Path) -> String {
    let name = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("checkpoint/{}", crate::foreign::ref_part(&name))
}

fn folder_of(given: Option<PathBuf>) -> Result<PathBuf, Failure> {
    let folder = match given {
        Some(f) => f,
        None => std::env::current_dir().map_err(|e| Failure::Usage(e.to_string()))?,
    };
    folder
        .canonicalize()
        .map_err(|e| Failure::Usage(format!("{}: {e}", folder.display())))
}

/// What a hook of the agent says on standard input: Claude Code and others
/// that follow it send `session_id`, `cwd`, `hook_event_name` and
/// `tool_name`.
pub(crate) struct Hook {
    pub(crate) folder: Option<PathBuf>,
    pub(crate) session: Option<String>,
    pub(crate) event: Option<String>,
}

pub(crate) fn read_hook() -> Hook {
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    let value: Value = serde_json::from_str(&text).unwrap_or_default();
    let field = |k: &str| {
        value[k]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Hook {
        folder: field("cwd").map(PathBuf::from),
        session: field("session_id"),
        event: match (field("hook_event_name"), field("tool_name")) {
            (Some(e), Some(t)) => Some(format!("{e}:{t}")),
            (e, _) => e,
        },
    }
}

/// Records `folder` in its checkpoint ref; nothing when it did not change.
/// Returns the checkpoint and whether it is new.
pub(crate) fn save(
    out: &Output,
    repo: &Repository,
    folder: &Path,
    session: Option<&str>,
    event: Option<&str>,
    message: Option<String>,
) -> Result<(apeiron::ObjectId, bool), Failure> {
    let mut request = BackupRequest::new(folder, ref_of(folder));
    request.exclude = LEFT_OUT.iter().map(|p| (*p).to_owned()).collect();
    request.skip_same = true;
    // An agent rewriting many files is what checkpoints are for.
    request.alarm_percent = Some(0);
    request.message = Some(message.unwrap_or_else(|| match event {
        Some(e) => format!("checkpoint: {e}"),
        None => "checkpoint".to_owned(),
    }));
    if let Some(s) = session {
        request.meta.push((SESSION.to_owned(), s.to_owned()));
    }
    if let Some(e) = event {
        request.meta.push((EVENT.to_owned(), e.to_owned()));
    }
    let report = watch(out, repo.backup(request)?)?;
    Ok((report.snapshot, !report.same))
}

fn meta<'a>(s: &'a SnapshotInfo, key: &str) -> Option<&'a str> {
    s.meta
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The checkpoints of a folder, newest first.
pub(crate) fn history(repo: &Repository, folder: &Path) -> Result<Vec<SnapshotInfo>, Failure> {
    match repo.log(&LogRequest {
        start: ref_of(folder),
        limit: usize::MAX,
        grep: None,
    }) {
        Ok(page) => Ok(page.snapshots),
        Err(e) if e.kind == apeiron::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// The checkpoint `undo` goes back to: the start of the last turn of the
/// newest session, or of the whole session with `whole_session`, or of the
/// given session.
pub(crate) fn undo_target(
    checkpoints: &[SnapshotInfo],
    session: Option<&str>,
    whole_session: bool,
) -> Option<SnapshotInfo> {
    let session = match session {
        Some(s) => s.to_owned(),
        None => checkpoints
            .iter()
            .find_map(|c| meta(c, SESSION).filter(|s| *s != "undo"))?
            .to_owned(),
    };
    let of_session: Vec<&SnapshotInfo> = checkpoints
        .iter()
        .filter(|c| meta(c, SESSION) == Some(session.as_str()))
        .collect();
    let turn = (!whole_session)
        .then(|| of_session.iter().find(|c| meta(c, EVENT) == Some(TURN)))
        .flatten();
    turn.or(of_session.last()).map(|c| (*c).clone())
}

/// Makes `folder` hold `target` again, leaving alone what checkpoints leave
/// out; what it held is recorded first.
pub(crate) fn restore(
    out: &Output,
    repo: &Repository,
    folder: &Path,
    target: &SnapshotInfo,
    dry_run: bool,
) -> Result<apeiron::SwitchReport, Failure> {
    let request = SwitchRequest {
        snapshot: target.id.to_hex(),
        folder: folder.to_path_buf(),
        dry_run,
        exclude: LEFT_OUT.iter().map(|p| (*p).to_owned()).collect(),
    };
    watch(out, repo.switch(request)?)
}

pub(crate) fn folder(given: Option<PathBuf>) -> Result<PathBuf, Failure> {
    folder_of(given)
}

pub(crate) fn describe(c: &SnapshotInfo) -> String {
    format!(
        "{}  {}  {}{}",
        crate::format::short_id(&c.id),
        crate::format::time(&c.time),
        c.message.as_deref().unwrap_or(""),
        meta(c, SESSION)
            .map(|s| format!("  (session {})", s.get(..8).unwrap_or(s)))
            .unwrap_or_default()
    )
}

pub(crate) fn print_saved(id: &apeiron::ObjectId, new: bool, folder: &Path) {
    if new {
        say!(
            "Checkpoint {} of {}",
            crate::format::short_id(id),
            folder.display()
        );
    } else {
        say!(
            "{} has not changed since checkpoint {}",
            folder.display(),
            crate::format::short_id(id)
        );
    }
}
