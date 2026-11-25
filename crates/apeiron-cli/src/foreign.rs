//! Importing the backups of restic and Borg (CLI.md §4.4): each snapshot or
//! archive is written as a tar archive by the program that made it, and
//! imported as a snapshot with its own time, into a history of its own.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};

use apeiron::{ImportFormat, ImportRequest, LogRequest, Repository, Time};
use serde_json::Value;

use crate::{Failure, Output, watch};

/// One snapshot of restic or archive of Borg.
pub(crate) struct Foreign {
    pub(crate) id: String,
    pub(crate) time: Time,
    pub(crate) host: Option<String>,
    /// The message of the imported snapshot.
    pub(crate) label: String,
    /// The last part of the ref, for restic: host and folder.
    pub(crate) group: Option<String>,
}

/// Lowercase letters, digits, `.`, `_` and `-`, as ref names allow.
pub(crate) fn ref_part(text: &str) -> String {
    let part: String = text
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '_' | '-' => c,
            _ => '-',
        })
        .collect();
    match part.trim_matches(['-', '.']) {
        "" => "root".to_owned(),
        p => p.to_owned(),
    }
}

fn missing(program: &str, e: &std::io::Error) -> Failure {
    Failure::Usage(format!(
        "cannot run {program}: {e}; it must be installed to import its backups"
    ))
}

/// Runs a program and reads the JSON it prints.
fn json(program: &str, args: &[&str]) -> Result<Value, Failure> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| missing(program, &e))?;
    if !output.status.success() {
        return Err(Failure::Usage(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| Failure::Usage(format!("{program} gave no JSON: {e}")))
}

/// Runs a program that writes a tar archive to its standard output into
/// `file`.
fn tar_into(program: &str, args: &[&str], file: &Path) -> Result<(), Failure> {
    let out = File::create(file).map_err(|e| Failure::Usage(format!("{}: {e}", file.display())))?;
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| missing(program, &e))?;
    if !output.status.success() {
        return Err(Failure::Usage(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn time_of(stamp: jiff::Timestamp) -> Time {
    Time {
        seconds: stamp.as_second(),
        nanos: u32::try_from(stamp.subsec_nanosecond()).unwrap_or(0),
    }
}

/// The snapshots of the restic repository `source` (`restic -r`); its
/// password comes from `RESTIC_PASSWORD` or `RESTIC_PASSWORD_FILE`.
pub(crate) fn restic_snapshots(source: &str) -> Result<Vec<Foreign>, Failure> {
    let list = json("restic", &["-r", source, "snapshots", "--json"])?;
    let mut out = Vec::new();
    for s in list.as_array().into_iter().flatten() {
        let text = |key: &str| s[key].as_str().unwrap_or_default().to_owned();
        let id = text("id");
        let stamp: jiff::Timestamp = text("time")
            .parse()
            .map_err(|e| Failure::Usage(format!("restic snapshot {id}: time: {e}")))?;
        let host = text("hostname");
        let folder = s["paths"][0]
            .as_str()
            .and_then(|p| Path::new(p).file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let short = id.get(..8).unwrap_or(&id).to_owned();
        out.push(Foreign {
            label: format!("restic snapshot {short}"),
            group: Some(format!("{}/{}", ref_part(&host), ref_part(&folder))),
            host: (!host.is_empty()).then_some(host),
            time: time_of(stamp),
            id,
        });
    }
    Ok(out)
}

pub(crate) fn restic_dump(source: &str, id: &str, file: &Path) -> Result<(), Failure> {
    tar_into(
        "restic",
        &["-r", source, "dump", "--archive", "tar", id, "/"],
        file,
    )
}

/// The archives of the Borg repository `source`; its passphrase comes from
/// `BORG_PASSPHRASE` or `BORG_PASSCOMMAND`.
pub(crate) fn borg_archives(source: &str) -> Result<Vec<Foreign>, Failure> {
    let list = json("borg", &["list", "--json", source])?;
    let mut out = Vec::new();
    for a in list["archives"].as_array().into_iter().flatten() {
        let name = a["name"].as_str().unwrap_or_default().to_owned();
        // Borg gives the local time of the computer that made it.
        let civil: jiff::civil::DateTime = a["time"]
            .as_str()
            .unwrap_or_default()
            .parse()
            .map_err(|e| Failure::Usage(format!("borg archive {name}: time: {e}")))?;
        let stamp = civil
            .to_zoned(jiff::tz::TimeZone::system())
            .map_err(|e| Failure::Usage(format!("borg archive {name}: time: {e}")))?
            .timestamp();
        out.push(Foreign {
            id: a["id"].as_str().unwrap_or(&name).to_owned(),
            label: format!("borg archive {name}"),
            host: None,
            group: None,
            time: time_of(stamp),
        });
    }
    Ok(out)
}

pub(crate) fn borg_dump(source: &str, name: &str, file: &Path) -> Result<(), Failure> {
    tar_into(
        "borg",
        &["export-tar", &format!("{source}::{name}"), "-"],
        file,
    )
}

/// Imports `snapshots`, oldest first, into `prefix` (or `prefix/<group>`),
/// skipping those an earlier import already brought; `dump` writes one as a
/// tar archive. Returns how many were imported.
pub(crate) fn import(
    out: &Output,
    repo: &Repository,
    kind: &str,
    mut snapshots: Vec<Foreign>,
    prefix: &str,
    dump: impl Fn(&Foreign, &Path) -> Result<(), Failure>,
) -> Result<Vec<(String, String)>, Failure> {
    snapshots.sort_by_key(|s| (s.time.seconds, s.time.nanos));
    let key = format!("import.{kind}");
    let mut known: HashMap<String, HashSet<String>> = HashMap::new();
    let tmp = repo.path().join("tmp");
    fs::create_dir_all(&tmp).map_err(|e| Failure::Usage(format!("{}: {e}", tmp.display())))?;
    let mut done = Vec::new();
    for s in &snapshots {
        let ref_name = match &s.group {
            Some(group) => format!("{prefix}/{group}"),
            None => prefix.to_owned(),
        };
        let seen = match known.get(&ref_name) {
            Some(seen) => seen,
            None => {
                let mut seen = HashSet::new();
                if let Ok(page) = repo.log(&LogRequest {
                    start: ref_name.clone(),
                    limit: usize::MAX,
                    grep: None,
                }) {
                    for snapshot in page.snapshots {
                        seen.extend(
                            snapshot
                                .meta
                                .into_iter()
                                .filter(|(k, _)| *k == key)
                                .map(|(_, v)| v),
                        );
                    }
                }
                known.entry(ref_name.clone()).or_insert(seen)
            }
        };
        if seen.contains(&s.id) {
            continue;
        }
        // The repository's own temporary folder is on the same disk and
        // cleaned up after an interruption (FORMAT.md §2).
        let file = tmp.join(format!("import-{kind}-{}.tar", ref_part(&s.id)));
        let result = dump(s, &file).and_then(|()| {
            let mut request = ImportRequest::new(&file, ref_name.clone());
            request.format = Some(ImportFormat::Tar);
            request.message = Some(s.label.clone());
            request.host = s.host.clone();
            request.time = Some(s.time);
            request.meta = vec![(key.clone(), s.id.clone())];
            watch(out, repo.import_archive(request)?)
        });
        let _ = fs::remove_file(&file);
        result?;
        say!("Imported {} into {ref_name}", s.label);
        known
            .entry(ref_name.clone())
            .or_default()
            .insert(s.id.clone());
        done.push((s.id.clone(), ref_name));
    }
    Ok(done)
}
