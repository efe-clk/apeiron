//! Application-consistent sources (CONSISTENT.md): a database dump, a
//! read-only snapshot of a file system, or the content of a container
//! volume, prepared so that a backup sees one moment, and removed again.
//! Each is made by the system's own program.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, Result};

/// Where a backup comes from, as `<kind>:<target>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// `postgres:<database or connection string>`: `pg_dump`.
    Postgres(String),
    /// `mysql:<database>`: `mysqldump`, or `mariadb-dump`.
    Mysql(String),
    /// `btrfs:<subvolume>`: a read-only snapshot.
    Btrfs(PathBuf),
    /// `zfs:<dataset>`: a snapshot.
    Zfs(String),
    /// `lvm:<group>/<volume>`: a snapshot volume, mounted read-only.
    Lvm(String),
    /// `docker:<volume>`: the volume as a tar archive.
    Docker(String),
    /// `kubernetes:<namespace>/<claim>`: the persistent volume as a tar
    /// archive, read by a helper pod.
    Kubernetes { namespace: String, claim: String },
    /// `vss:<folder>`: a Volume Shadow Copy of the folder's drive.
    Vss(PathBuf),
}

impl Source {
    pub fn parse(spec: &str) -> std::result::Result<Self, String> {
        let (kind, target) = spec
            .split_once(':')
            .filter(|(_, t)| !t.is_empty())
            .ok_or_else(|| format!("{spec:?} is not <kind>:<target>, such as postgres:shop"))?;
        Ok(match kind {
            "postgres" => Source::Postgres(target.to_owned()),
            "mysql" => Source::Mysql(target.to_owned()),
            "btrfs" => Source::Btrfs(PathBuf::from(target)),
            "zfs" => Source::Zfs(target.to_owned()),
            "lvm" if target.contains('/') => Source::Lvm(target.to_owned()),
            "docker" => Source::Docker(target.to_owned()),
            "kubernetes" => {
                let (namespace, claim) = target
                    .split_once('/')
                    .ok_or("kubernetes:<namespace>/<claim>")?;
                Source::Kubernetes {
                    namespace: namespace.to_owned(),
                    claim: claim.to_owned(),
                }
            }
            "vss" => Source::Vss(PathBuf::from(target)),
            "lvm" => return Err("lvm:<group>/<volume>".to_owned()),
            other => {
                return Err(format!(
                    "unknown source {other:?}: postgres, mysql, btrfs, zfs, lvm, docker, kubernetes or vss"
                ));
            }
        })
    }

    fn kind(&self) -> &'static str {
        match self {
            Source::Postgres(_) => "postgres",
            Source::Mysql(_) => "mysql",
            Source::Btrfs(_) => "btrfs",
            Source::Zfs(_) => "zfs",
            Source::Lvm(_) => "lvm",
            Source::Docker(_) => "docker",
            Source::Kubernetes { .. } => "kubernetes",
            Source::Vss(_) => "vss",
        }
    }

    /// The ref a backup of it goes to unless another is given:
    /// `<kind>/<name>`, such as `postgres/shop`.
    pub fn default_ref(&self) -> String {
        let name = match self {
            Source::Postgres(t) => database_name(t),
            Source::Mysql(t) | Source::Zfs(t) | Source::Lvm(t) | Source::Docker(t) => t.clone(),
            Source::Kubernetes { namespace, claim } => format!("{namespace}-{claim}"),
            Source::Btrfs(p) | Source::Vss(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "root".to_owned()),
        };
        format!("{}/{}", self.kind(), part(&name))
    }
}

/// A database's name from a connection string, or the string itself.
fn database_name(target: &str) -> String {
    if let Some(rest) = target.split_once("://").map(|(_, r)| r) {
        return rest
            .rsplit('/')
            .next()
            .and_then(|d| d.split('?').next())
            .filter(|d| !d.is_empty())
            .unwrap_or("postgres")
            .to_owned();
    }
    target
        .split_whitespace()
        .find_map(|kv| kv.strip_prefix("dbname="))
        .unwrap_or(target)
        .to_owned()
}

/// Lowercase letters, digits, `.`, `_` and `-` of a ref name.
fn part(text: &str) -> String {
    let p: String = text
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '.' | '_' | '-' => c,
            _ => '-',
        })
        .collect();
    match p.trim_matches(['-', '.']) {
        "" => "source".to_owned(),
        p => p.to_owned(),
    }
}

fn failed(what: &str, detail: impl std::fmt::Display) -> Error {
    Error::Source(format!("{what}: {detail}"))
}

/// Runs a program; its standard output, or why it failed.
fn run(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| failed(program, format!("cannot run it: {e}")))?;
    if !output.status.success() {
        return Err(failed(
            &format!("{program} {}", args.join(" ")),
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    Ok(output.stdout)
}

/// Runs a program whose standard output goes into `file`.
fn run_into(program: &str, args: &[&str], file: &Path) -> Result<()> {
    let out = fs::File::create(file).map_err(|e| failed(&file.display().to_string(), e))?;
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| failed(program, format!("cannot run it: {e}")))?;
    if !output.status.success() {
        return Err(failed(
            &format!("{program} {}", args.join(" ")),
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    Ok(())
}

/// A word that names this run, for snapshots and helpers.
fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("apeiron-{secs}-{}", std::process::id())
}

/// One step that undoes what preparing did.
enum Undo {
    Remove(PathBuf),
    Command(String, Vec<String>),
    /// A read-only btrfs snapshot: deleted, or made writable and removed
    /// by its owner where deleting needs more rights.
    Subvolume(PathBuf),
}

/// What a backup reads: a folder, or a tar archive to import. Dropping it
/// removes the snapshot, dump or helper it was made with.
pub struct Prepared {
    pub path: PathBuf,
    pub tar: bool,
    undo: Vec<Undo>,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        for step in self.undo.drain(..).rev() {
            match step {
                Undo::Remove(path) => {
                    let _ = fs::remove_dir_all(&path).or_else(|_| fs::remove_file(&path));
                }
                Undo::Command(program, args) => {
                    let args: Vec<&str> = args.iter().map(String::as_str).collect();
                    if let Err(e) = run(&program, &args) {
                        eprintln!("cleaning up: {e}");
                    }
                }
                Undo::Subvolume(path) => {
                    let p = path.to_string_lossy().into_owned();
                    if run("btrfs", &["subvolume", "delete", &p]).is_err() {
                        let _ = run("btrfs", &["property", "set", "-ts", &p, "ro", "false"]);
                        if let Err(e) = fs::remove_dir_all(&path) {
                            eprintln!("cleaning up {p}: {e}");
                        }
                    }
                }
            }
        }
    }
}

/// Prepares `source` for a backup; dumps and archives are written into
/// `tmp`, a folder of the repository (FORMAT.md §2).
pub fn prepare(source: &Source, tmp: &Path, pause: bool) -> Result<Prepared> {
    fs::create_dir_all(tmp).map_err(|e| failed(&tmp.display().to_string(), e))?;
    let id = stamp();
    let mut prepared = Prepared {
        path: PathBuf::new(),
        tar: false,
        undo: Vec::new(),
    };
    match source {
        Source::Postgres(target) => {
            let dir = tmp.join(&id);
            fs::create_dir(&dir).map_err(|e| failed(&dir.display().to_string(), e))?;
            prepared.undo.push(Undo::Remove(dir.clone()));
            let file = dir.join(format!("{}.sql", part(&database_name(target))));
            let f = file.to_string_lossy().into_owned();
            // Plain SQL deduplicates from one dump to the next.
            run(
                "pg_dump",
                &["--no-password", "--file", &f, "--dbname", target],
            )?;
            prepared.path = dir;
        }
        Source::Mysql(database) => {
            let dir = tmp.join(&id);
            fs::create_dir(&dir).map_err(|e| failed(&dir.display().to_string(), e))?;
            prepared.undo.push(Undo::Remove(dir.clone()));
            let file = dir.join(format!("{}.sql", part(database)));
            let result = format!("--result-file={}", file.display());
            let args = [
                "--single-transaction",
                "--routines",
                "--triggers",
                &result,
                "--databases",
                database,
            ];
            let program = if which("mysqldump") {
                "mysqldump"
            } else {
                "mariadb-dump"
            };
            run(program, &args)?;
            prepared.path = dir;
        }
        Source::Btrfs(subvolume) => {
            let name = subvolume
                .file_name()
                .ok_or_else(|| failed("btrfs", "name a subvolume, not /"))?;
            let snapshot = subvolume.with_file_name(format!(".{}.{id}", name.to_string_lossy()));
            let (from, to) = (subvolume.to_string_lossy(), snapshot.to_string_lossy());
            run("btrfs", &["subvolume", "snapshot", "-r", &from, &to])?;
            prepared.undo.push(Undo::Subvolume(snapshot.clone()));
            prepared.path = snapshot;
        }
        Source::Zfs(dataset) => {
            let snapshot = format!("{dataset}@{id}");
            run("zfs", &["snapshot", &snapshot])?;
            prepared.undo.push(Undo::Command(
                "zfs".into(),
                vec!["destroy".into(), snapshot.clone()],
            ));
            let mountpoint = String::from_utf8_lossy(&run(
                "zfs",
                &["get", "-H", "-o", "value", "mountpoint", dataset],
            )?)
            .trim()
            .to_owned();
            prepared.path = Path::new(&mountpoint).join(".zfs/snapshot").join(&id);
        }
        Source::Lvm(volume) => {
            let group = volume.split('/').next().unwrap_or_default();
            run(
                "lvcreate",
                &[
                    "--snapshot",
                    "--extents",
                    "10%ORIGIN",
                    "--name",
                    &id,
                    volume,
                ],
            )?;
            let device = format!("/dev/{group}/{id}");
            prepared.undo.push(Undo::Command(
                "lvremove".into(),
                vec!["--yes".into(), format!("{group}/{id}")],
            ));
            let dir = tmp.join(&id);
            fs::create_dir(&dir).map_err(|e| failed(&dir.display().to_string(), e))?;
            prepared.undo.push(Undo::Remove(dir.clone()));
            let d = dir.to_string_lossy().into_owned();
            // `nouuid` lets XFS mount a snapshot of a mounted volume.
            if run("mount", &["-o", "ro,nouuid", &device, &d]).is_err() {
                run("mount", &["-o", "ro", &device, &d])?;
            }
            prepared.undo.push(Undo::Command("umount".into(), vec![d]));
            prepared.path = dir;
        }
        Source::Docker(volume) => {
            let file = tmp.join(format!("{id}.tar"));
            prepared.undo.push(Undo::Remove(file.clone()));
            let users = if pause {
                let ids = run(
                    "docker",
                    &["ps", "-q", "--filter", &format!("volume={volume}")],
                )?;
                String::from_utf8_lossy(&ids)
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect()
            } else {
                Vec::new()
            };
            for c in &users {
                run("docker", &["pause", c])?;
            }
            let mount = format!("{volume}:/v:ro");
            let result = run_into(
                "docker",
                &[
                    "run", "--rm", "-v", &mount, "busybox", "tar", "-C", "/v", "-cf", "-", ".",
                ],
                &file,
            );
            for c in &users {
                let _ = run("docker", &["unpause", c]);
            }
            result?;
            prepared.path = file;
            prepared.tar = true;
        }
        Source::Kubernetes { namespace, claim } => {
            let file = tmp.join(format!("{id}.tar"));
            prepared.undo.push(Undo::Remove(file.clone()));
            let pod = id.clone();
            let overrides = serde_json::json!({
                "spec": {
                    "containers": [{
                        "name": "read", "image": "busybox",
                        "command": ["sleep", "3600"],
                        "volumeMounts": [{"name": "v", "mountPath": "/v", "readOnly": true}],
                    }],
                    "volumes": [{"name": "v", "persistentVolumeClaim": {"claimName": claim, "readOnly": true}}],
                }
            })
            .to_string();
            let ns = format!("--namespace={namespace}");
            run(
                "kubectl",
                &[
                    &ns,
                    "run",
                    &pod,
                    "--image=busybox",
                    "--restart=Never",
                    "--overrides",
                    &overrides,
                ],
            )?;
            prepared.undo.push(Undo::Command(
                "kubectl".into(),
                vec![
                    ns.clone(),
                    "delete".into(),
                    "pod".into(),
                    pod.clone(),
                    "--wait=false".into(),
                ],
            ));
            run(
                "kubectl",
                &[
                    &ns,
                    "wait",
                    "--for=condition=Ready",
                    &format!("pod/{pod}"),
                    "--timeout=300s",
                ],
            )?;
            run_into(
                "kubectl",
                &[&ns, "exec", &pod, "--", "tar", "-C", "/v", "-cf", "-", "."],
                &file,
            )?;
            prepared.path = file;
            prepared.tar = true;
        }
        Source::Vss(folder) => {
            let folder = std::path::absolute(folder).map_err(|e| failed("vss", e))?;
            let text = folder.to_string_lossy().into_owned();
            let drive = text
                .get(..3)
                .filter(|d| d.ends_with(":\\"))
                .ok_or_else(|| failed("vss", "name a folder on a drive, such as C:\\Users\\me"))?;
            let link = tmp.join(&id);
            let script = format!(
                "$r = Invoke-CimMethod -ClassName Win32_ShadowCopy -MethodName Create -Arguments @{{Volume='{drive}'}}; \
                 if ($r.ReturnValue -ne 0) {{ throw \"shadow copy: $($r.ReturnValue)\" }}; \
                 $s = Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq $r.ShadowID; \
                 cmd /c mklink /d \"{}\" \"$($s.DeviceObject)\\\" | Out-Null; \
                 Write-Output $r.ShadowID",
                link.display()
            );
            let shadow =
                String::from_utf8_lossy(&run("powershell", &["-NoProfile", "-Command", &script])?)
                    .trim()
                    .to_owned();
            prepared.undo.push(Undo::Command(
                "powershell".into(),
                vec![
                    "-NoProfile".into(),
                    "-Command".into(),
                    format!("Get-CimInstance Win32_ShadowCopy | Where-Object ID -eq '{shadow}' | Remove-CimInstance"),
                ],
            ));
            prepared.undo.push(Undo::Command(
                "cmd".into(),
                vec![
                    "/c".into(),
                    "rmdir".into(),
                    link.to_string_lossy().into_owned(),
                ],
            ));
            prepared.path = link.join(&text[3..]);
        }
    }
    Ok(prepared)
}

fn which(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_named_and_get_their_refs() {
        let cases = [
            ("postgres:shop", "postgres/shop"),
            (
                "postgres:postgresql://u@db:5432/Shop?sslmode=require",
                "postgres/shop",
            ),
            ("postgres:host=db dbname=crm user=me", "postgres/crm"),
            ("mysql:wiki", "mysql/wiki"),
            ("btrfs:/srv/photos", "btrfs/photos"),
            ("zfs:tank/home", "zfs/tank-home"),
            ("lvm:vg0/data", "lvm/vg0-data"),
            ("docker:pgdata", "docker/pgdata"),
            ("kubernetes:prod/db-data", "kubernetes/prod-db-data"),
        ];
        for (spec, name) in cases {
            assert_eq!(Source::parse(spec).unwrap().default_ref(), name, "{spec}");
        }
        for bad in [
            "postgres",
            "postgres:",
            "tape:/dev/st0",
            "lvm:data",
            "kubernetes:x",
        ] {
            assert!(Source::parse(bad).is_err(), "{bad}");
        }
    }
}
