//! The `apeiron` command (docs/CLI.md).

#![forbid(unsafe_code)]

/// Prints a line on standard output. If the reader went away (for example
/// `apeiron log | head`), the program ends quietly instead of failing.
macro_rules! say {
    ($($arg:tt)*) => {
        $crate::write_line(format_args!($($arg)*))
    };
}

mod format;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use apeiron::{
    ArchiveFormat, BackupRequest, ChangeKind, CloneRequest, ConflictKind, DiffRequest, Error,
    ErrorKind, ExportRequest, GcRequest, GitRequest, Job, KeyKind, ListRequest, LogRequest, Name,
    Repository, RestoreRequest, RestoreTestRequest, SyncAction, SyncMode, SyncReport, SyncRequest,
    TransferReport, TransferRequest, Warning,
};
use apeiron::{CreateRefRequest, DeleteRefRequest, MergeAction, MergeRequest};
use clap::{Parser, Subcommand};
use serde::Serialize;

const RECOVERY_WARNING: &str = "Write the recovery key down and keep it apart from the password. \
     It is shown only now. If every password and the recovery key are lost, \
     nobody can recover the data.";

/// Exit codes (docs/CLI.md §5).
const EXIT_ERROR: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_WARNINGS: u8 = 3;
const EXIT_DAMAGED: u8 = 4;
const EXIT_CANCELLED: u8 = 130;

#[derive(Parser)]
#[command(
    name = "apeiron",
    version,
    about = "Versioning, backup and archiving in one engine"
)]
struct Cli {
    /// Repository to use
    #[arg(long, global = true, env = "APEIRON_REPO", value_name = "PATH")]
    repo: Option<PathBuf>,

    /// File holding the password or recovery key of an encrypted repository
    #[arg(
        long,
        global = true,
        env = "APEIRON_PASSWORD_FILE",
        value_name = "PATH"
    )]
    password_file: Option<PathBuf>,

    /// Print the result as one JSON document on standard output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an empty repository
    Init {
        path: PathBuf,
        /// Encrypt the repository with a password; a recovery key is shown once
        #[arg(long)]
        encrypt: bool,
    },
    /// Store a folder as a new snapshot and move a ref to it
    Backup {
        folder: PathBuf,
        /// Ref to move to the new snapshot
        #[arg(long = "ref", default_value = "main")]
        ref_name: String,
        /// Description of the snapshot
        #[arg(short, long)]
        message: Option<String>,
        /// Who takes the snapshot
        #[arg(long)]
        author: Option<String>,
        /// Which device the snapshot is taken on
        #[arg(long)]
        host: Option<String>,
    },
    /// List the snapshots of a ref, newest first
    Log {
        #[arg(default_value = "main")]
        snapshot: String,
        /// Show at most this many snapshots
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,
    },
    /// List the files and folders inside a snapshot
    Ls {
        snapshot: String,
        #[arg(default_value = "")]
        path: String,
    },
    /// List files added, removed and changed between two snapshots
    Diff { from: String, to: String },
    /// Write a snapshot, or one path in it, into a new or empty folder
    Restore {
        snapshot: String,
        target: PathBuf,
        /// Path inside the snapshot to restore
        #[arg(long, default_value = "")]
        path: String,
    },
    /// Write a snapshot, or one path in it, into a new tar, zip or 7z file
    Export {
        snapshot: String,
        /// Archive file to create; its extension chooses the format
        file: PathBuf,
        /// Archive format, if the file extension does not say it
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
        /// Path inside the snapshot to export
        #[arg(long, default_value = "")]
        path: String,
    },
    /// Restore a random sample of a snapshot into a temporary folder and check it
    RestoreTest {
        #[arg(default_value = "main")]
        snapshot: String,
        /// How many MiB to restore; at least one file is always restored
        #[arg(long, default_value_t = 100)]
        sample_mb: u64,
    },
    /// List all refs and the snapshots they point to
    Refs,
    /// Read and check every pack, index and object
    Verify,
    /// Remove stale ref locks and add lost reflog lines
    Repair,
    /// Delete data that no ref and no recent reflog line can reach
    Gc {
        /// Snapshots a ref pointed to within this many days are kept; 0 keeps only what refs point to now
        #[arg(long, default_value_t = 90)]
        keep_days: u32,
        /// Only show what would be deleted
        #[arg(long)]
        dry_run: bool,
    },
    /// Manage the passwords and recovery keys of an encrypted repository
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    /// Create or delete a ref, such as a branch
    Ref {
        #[command(subcommand)]
        command: RefCommand,
    },
    /// Merge a snapshot, usually a branch, into a ref
    Merge {
        from: String,
        /// The ref that moves
        #[arg(long, default_value = "main")]
        into: String,
        #[arg(long)]
        message: Option<String>,
        /// Only show what the merge would do
        #[arg(long)]
        dry_run: bool,
    },
    /// Import a Git repository with its history, or export it back
    Git {
        #[command(subcommand)]
        command: GitCommand,
    },
    /// Manage the remote copies of the repository
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
    /// Run scheduled jobs: backups, pushes, checks and restore tests
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
        /// Jobs file instead of the usual one
        #[arg(long, global = true, value_name = "PATH")]
        jobs: Option<PathBuf>,
        /// Folder for the state and the log instead of the usual one
        #[arg(long, global = true, value_name = "PATH")]
        state_dir: Option<PathBuf>,
    },
    /// Make a new repository from a remote copy and pull all its refs
    Clone {
        /// A folder, sftp://[user@]host[:port]/path or s3://bucket/path
        url: String,
        /// The new repository; must not exist or be empty
        path: PathBuf,
    },
    /// Keep a folder equal on several devices through a remote
    Sync {
        folder: PathBuf,
        remote: String,
        /// Ref on both sides; default sync/<folder name>
        #[arg(long = "ref", value_name = "NAME")]
        ref_name: Option<String>,
        #[arg(long, value_enum, default_value = "two-way")]
        mode: ModeArg,
        /// Only show what would change
        #[arg(long)]
        dry_run: bool,
    },
    /// Copy the snapshots of a ref to a remote and move the ref there
    Push {
        remote: String,
        #[arg(default_value = "main")]
        ref_name: String,
    },
    /// Copy the snapshots of a ref from a remote and move the local ref forward
    Pull {
        remote: String,
        #[arg(default_value = "main")]
        ref_name: String,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Run until stopped, starting each job when it is due
    Run,
    /// Run every job that is due now, then exit
    Once,
    /// Show each job's last run, its result and the next run
    Status,
}

#[derive(Subcommand)]
enum RefCommand {
    /// Create a ref pointing to a snapshot
    Create {
        name: String,
        #[arg(default_value = "main")]
        snapshot: String,
    },
    /// Delete a ref; its reflog keeps the history until garbage collection
    Delete { name: String },
}

#[derive(Subcommand)]
enum GitCommand {
    /// Store every branch and tag of a Git repository as snapshots
    Import {
        path: PathBuf,
        /// Folder of refs for the repository
        #[arg(long, default_value = "git")]
        prefix: String,
    },
    /// Write the snapshots under the prefix into a Git repository
    Export {
        path: PathBuf,
        /// Folder of refs for the repository
        #[arg(long, default_value = "git")]
        prefix: String,
    },
}

#[derive(Subcommand)]
enum RemoteCommand {
    /// List the remotes
    List,
    /// Add a remote: a folder, such as one on an external disk, sftp://[user@]host[:port]/path or s3://bucket/path?endpoint=URL&region=NAME
    Add { name: String, url: String },
    /// Forget a remote; the copy itself is not touched
    Remove { name: String },
}

#[derive(Subcommand)]
enum KeyCommand {
    /// List the passwords and recovery keys that open the repository
    List,
    /// Add a password, or a new recovery key with --recovery
    Add {
        /// Add a new recovery key instead of a password
        #[arg(long)]
        recovery: bool,
        /// File holding the new password on its first line
        #[arg(long, value_name = "PATH", conflicts_with = "recovery")]
        new_password_file: Option<PathBuf>,
    },
    /// Remove a password or recovery key; the last one cannot be removed
    Remove {
        /// Key id from `apeiron key list`; its first characters are enough
        id: String,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ModeArg {
    TwoWay,
    Send,
    Receive,
}

impl From<ModeArg> for SyncMode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::TwoWay => SyncMode::TwoWay,
            ModeArg::Send => SyncMode::Send,
            ModeArg::Receive => SyncMode::Receive,
        }
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum FormatArg {
    Tar,
    Zip,
    #[value(name = "7z")]
    SevenZ,
}

impl From<FormatArg> for ArchiveFormat {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Tar => ArchiveFormat::Tar,
            FormatArg::Zip => ArchiveFormat::Zip,
            FormatArg::SevenZ => ArchiveFormat::SevenZ,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let out = Output { json: cli.json };
    match run(cli, &out) {
        Ok(code) => ExitCode::from(code),
        Err(Failure::Usage(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(EXIT_USAGE)
        }
        Err(Failure::Api(e)) => {
            if out.json {
                out.json_value(&ErrorDocument { error: &e });
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(if e.kind == ErrorKind::Cancelled {
                EXIT_CANCELLED
            } else {
                EXIT_ERROR
            })
        }
    }
}

enum Failure {
    Usage(String),
    Api(Error),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Api(e)
    }
}

#[derive(Serialize)]
struct ErrorDocument<'a> {
    error: &'a Error,
}

/// Result of `init`; the recovery key is only there for an encrypted repository.
#[derive(Serialize)]
struct InitResult {
    #[serde(flatten)]
    info: apeiron::RepoInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_key: Option<String>,
}

fn open(cli_repo: Option<PathBuf>, password_file: Option<&Path>) -> Result<Repository, Failure> {
    let path = cli_repo.ok_or_else(|| {
        Failure::Usage("no repository given; use --repo or set APEIRON_REPO".to_owned())
    })?;
    let repo = match Repository::open(&path) {
        Err(e) if e.kind == ErrorKind::Key => {
            let secret = match password_file {
                Some(file) => read_password_file(file)?,
                None => prompt("Password or recovery key: ")?,
            };
            Repository::unlock(&path, &secret)?
        }
        result => result?,
    };
    let damaged = repo.info().damaged_files;
    if !damaged.is_empty() {
        for path in &damaged {
            eprintln!("warning: damaged file {}", path.display());
        }
        eprintln!(
            "warning: run `apeiron verify` for details and `apeiron repair` to rebuild damaged indexes"
        );
    }
    Ok(repo)
}

/// Reads a password or recovery key from the first line of a file.
fn read_password_file(path: &Path) -> Result<String, Failure> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        Failure::Usage(format!("cannot read password file {}: {e}", path.display()))
    })?;
    let line = text.lines().next().unwrap_or_default();
    if line.is_empty() {
        return Err(Failure::Usage(format!(
            "password file {} is empty",
            path.display()
        )));
    }
    Ok(line.to_owned())
}

/// Asks for a password on the terminal without showing it.
fn prompt(text: &str) -> Result<String, Failure> {
    rpassword::prompt_password(text).map_err(|e| {
        Failure::Usage(format!(
            "cannot ask for the password ({e}); use --password-file or APEIRON_PASSWORD_FILE"
        ))
    })
}

fn new_password(password_file: Option<&Path>) -> Result<String, Failure> {
    if let Some(file) = password_file {
        return read_password_file(file);
    }
    let password = prompt("New password: ")?;
    if password.is_empty() {
        return Err(Failure::Usage("the password must not be empty".to_owned()));
    }
    if prompt("Repeat the password: ")? != password {
        return Err(Failure::Usage("the passwords do not match".to_owned()));
    }
    Ok(password)
}

fn run(cli: Cli, out: &Output) -> Result<u8, Failure> {
    let password_file = cli.password_file.as_deref();
    match cli.command {
        Command::Init { path, encrypt } => {
            let (repo, recovery_key) = if encrypt {
                let (repo, key) = Repository::init_encrypted(&path, &new_password(password_file)?)?;
                (repo, Some(key))
            } else {
                (Repository::init(&path)?, None)
            };
            let result = InitResult {
                info: repo.info(),
                recovery_key,
            };
            out.result(&result, |r| match &r.recovery_key {
                None => say!("Created an empty repository at {}", r.info.path.display()),
                Some(key) => {
                    say!(
                        "Created an empty encrypted repository at {}",
                        r.info.path.display()
                    );
                    say!("Recovery key: {key}");
                    say!("{RECOVERY_WARNING}");
                }
            });
            Ok(0)
        }
        Command::Backup {
            folder,
            ref_name,
            message,
            author,
            host,
        } => {
            let repo = open(cli.repo, password_file)?;
            let mut request = BackupRequest::new(folder, ref_name.clone());
            request.message = message;
            request.author = author;
            request.host = host;
            let report = watch(out, repo.backup(request)?)?;
            out.result(&report, |r| {
                say!(
                    "Snapshot {} ({ref_name}): {}, {}, {} read, {} new",
                    format::short_id(&r.snapshot),
                    format::count(r.files, "file"),
                    format::count(r.dirs, "folder"),
                    format::bytes(r.bytes),
                    format::bytes(r.new_bytes),
                );
            });
            Ok(warnings_exit(out, &report.warnings))
        }
        Command::Log { snapshot, limit } => {
            let repo = open(cli.repo, password_file)?;
            let page = repo.log(&LogRequest {
                start: snapshot,
                limit,
            })?;
            out.result(&page, |page| {
                for s in &page.snapshots {
                    say!(
                        "{}  {}  {}",
                        format::short_id(&s.id),
                        format::time(&s.time),
                        s.message.as_deref().unwrap_or("")
                    );
                }
                if let Some(next) = page.next {
                    say!("… more: apeiron log {}", format::short_id(&next));
                }
            });
            Ok(0)
        }
        Command::Ls { snapshot, path } => {
            let repo = open(cli.repo, password_file)?;
            let entries = repo.list(&ListRequest {
                snapshot,
                path: Name::from(path.as_str()),
            })?;
            out.result(&entries, |entries| {
                for e in entries {
                    say!("{}", format::entry(e));
                }
            });
            Ok(0)
        }
        Command::Diff { from, to } => {
            let repo = open(cli.repo, password_file)?;
            let changes = repo.diff(&DiffRequest { from, to })?;
            out.result(&changes, |changes| {
                for c in changes {
                    say!("{} {}", format::change(c.kind), c.path);
                }
            });
            Ok(0)
        }
        Command::Restore {
            snapshot,
            target,
            path,
        } => {
            let repo = open(cli.repo, password_file)?;
            let report = watch(
                out,
                repo.restore(RestoreRequest {
                    snapshot,
                    target: target.clone(),
                    path: Name::from(path.as_str()),
                })?,
            )?;
            out.result(&report, |r| {
                say!(
                    "Restored {}, {}, {} into {}",
                    format::count(r.files, "file"),
                    format::count(r.dirs, "folder"),
                    format::bytes(r.bytes),
                    target.display()
                );
            });
            Ok(warnings_exit(out, &report.warnings))
        }
        Command::Export {
            snapshot,
            file,
            format,
            path,
        } => {
            let repo = open(cli.repo, password_file)?;
            let report = watch(
                out,
                repo.export(ExportRequest {
                    snapshot,
                    output: file.clone(),
                    format: format.map(ArchiveFormat::from),
                    path: Name::from(path.as_str()),
                })?,
            )?;
            out.result(&report, |r| {
                say!(
                    "Exported {}, {}, {} into {}",
                    format::count(r.files, "file"),
                    format::count(r.dirs, "folder"),
                    format::bytes(r.bytes),
                    file.display()
                );
            });
            Ok(warnings_exit(out, &report.warnings))
        }
        Command::RestoreTest {
            snapshot,
            sample_mb,
        } => {
            let repo = open(cli.repo, password_file)?;
            let mut request = RestoreTestRequest::new(snapshot);
            request.sample_bytes = sample_mb.saturating_mul(1024 * 1024);
            let report = watch(out, repo.restore_test(request)?)?;
            out.result(&report, |r| {
                for p in &r.problems {
                    say!("damaged: {}: {}", p.path, p.message);
                }
                if r.is_ok() {
                    say!(
                        "OK: restored and checked {}, {}",
                        format::count(r.files, "file"),
                        format::bytes(r.bytes)
                    );
                }
            });
            Ok(if report.is_ok() { 0 } else { EXIT_DAMAGED })
        }
        Command::Refs => {
            let repo = open(cli.repo, password_file)?;
            let refs = repo.refs()?;
            out.result(&refs, |refs| {
                for r in refs {
                    say!("{}  {}", format::short_id(&r.snapshot), r.name);
                }
            });
            Ok(0)
        }
        Command::Verify => {
            let repo = open(cli.repo, password_file)?;
            let report = watch(out, repo.verify()?)?;
            out.result(&report, |r| {
                for path in &r.damaged_files {
                    say!("damaged file: {}", path.display());
                }
                for id in &r.damaged_objects {
                    say!("damaged object: {id}");
                }
                if r.is_ok() {
                    say!("OK: {} objects checked, no damage found", r.objects_checked);
                }
            });
            Ok(if report.is_ok() { 0 } else { EXIT_DAMAGED })
        }
        Command::Gc { keep_days, dry_run } => {
            let repo = open(cli.repo, password_file)?;
            let report = watch(
                out,
                repo.gc(GcRequest {
                    keep_reflog_days: keep_days,
                    dry_run,
                })?,
            )?;
            out.result(&report, |r| {
                let verb = if r.dry_run { "Would remove" } else { "Removed" };
                say!(
                    "{verb} {} ({}) in {}; {} live objects kept",
                    format::count(r.removed_objects, "object"),
                    format::bytes(r.removed_bytes),
                    format::count(r.removed_packs, "pack"),
                    r.live_objects
                );
            });
            Ok(0)
        }
        Command::Key { command } => {
            let repo = open(cli.repo, password_file)?;
            match command {
                KeyCommand::List => {
                    let keys = repo.keys()?;
                    out.result(&keys, |keys| {
                        for k in keys {
                            let kind = match k.kind {
                                KeyKind::Password => "password",
                                KeyKind::Recovery => "recovery",
                            };
                            let current = if k.current { "  (in use)" } else { "" };
                            say!(
                                "{}  {kind:8}  added {}{current}",
                                format::short_key(&k.id),
                                k.created.replace('T', " ").replace('Z', " UTC")
                            );
                        }
                    });
                }
                KeyCommand::Add {
                    recovery,
                    new_password_file,
                } => {
                    let added = if recovery {
                        repo.add_recovery_key()?
                    } else {
                        repo.add_password(&new_password(new_password_file.as_deref())?)?
                    };
                    out.result(&added, |a| {
                        say!("Added key {}", format::short_key(&a.id));
                        if let Some(key) = &a.recovery_key {
                            say!("Recovery key: {key}");
                            say!("{RECOVERY_WARNING}");
                        }
                    });
                }
                KeyCommand::Remove { id } => {
                    let removed = repo.remove_key(&id)?;
                    out.result(&serde_json::json!({ "removed": removed }), |_| {
                        say!("Removed key {}", format::short_key(&removed));
                    });
                }
            }
            Ok(0)
        }
        Command::Ref { command } => {
            let repo = open(cli.repo, password_file)?;
            match command {
                RefCommand::Create { name, snapshot } => {
                    let id = repo.create_ref(&CreateRefRequest {
                        name: name.clone(),
                        snapshot,
                    })?;
                    out.result(&id, |id| say!("Created {name} at {}", format::short_id(id)));
                }
                RefCommand::Delete { name } => {
                    let expected = repo.resolve(&name)?;
                    repo.delete_ref(&DeleteRefRequest {
                        name: name.clone(),
                        expected,
                    })?;
                    out.result(&expected, |id| {
                        say!(
                            "Deleted {name} (was {}); `apeiron reflog {name}` keeps it",
                            format::short_id(id)
                        );
                    });
                }
            }
            Ok(0)
        }
        Command::Merge {
            from,
            into,
            message,
            dry_run,
        } => {
            let repo = open(cli.repo, password_file)?;
            let mut request = MergeRequest::new(from.clone(), into.clone());
            request.message = message;
            request.dry_run = dry_run;
            let report = repo.merge(&request)?;
            out.result(&report, |r| {
                for change in &r.changes {
                    let mark = match change.kind {
                        ChangeKind::Added => '+',
                        ChangeKind::Removed => '-',
                        ChangeKind::Modified => '~',
                    };
                    say!("{mark} {}", change.path);
                }
                for c in &r.conflicts {
                    match &c.copy {
                        Some(copy) => {
                            say!("conflict: {}; the version from {from} is {copy}", c.path)
                        }
                        None => say!(
                            "conflict: {} (deleted on one side, changed on the other; kept)",
                            c.path
                        ),
                    }
                }
                let would = if dry_run { "would be " } else { "" };
                let id = format::short_id(&r.snapshot);
                match r.action {
                    MergeAction::UpToDate => say!("{into} already has everything in {from}"),
                    MergeAction::FastForward => {
                        say!("{into} {would}moved forward to {from} ({id})")
                    }
                    MergeAction::Merged => say!(
                        "{from} {would}merged into {into} ({}, {})",
                        format::count(r.changes.len() as u64, "change"),
                        format::count(r.conflicts.len() as u64, "conflict")
                    ),
                }
            });
            Ok(if report.conflicts.is_empty() {
                0
            } else {
                EXIT_WARNINGS
            })
        }
        Command::Git { command } => {
            let repo = open(cli.repo, password_file)?;
            match command {
                GitCommand::Import { path, prefix } => {
                    let request = GitRequest { path, prefix };
                    let report = watch(out, repo.git_import(request)?)?;
                    out.result(&report, |r| {
                        for skipped in &r.skipped {
                            say!("skipped {}: {}", skipped.name, skipped.reason);
                        }
                        for name in &r.stale {
                            say!("kept {name}: gone from the Git repository");
                        }
                        for update in &r.refs {
                            say!("{} → {}", update.git, update.ref_name);
                        }
                        say!(
                            "Imported {} and {}; {} moved",
                            format::count(r.commits, "commit"),
                            format::count(r.tags, "tag"),
                            format::count(r.refs.len() as u64, "ref")
                        );
                    });
                }
                GitCommand::Export { path, prefix } => {
                    let request = GitRequest { path, prefix };
                    let report = watch(out, repo.git_export(request)?)?;
                    out.result(&report, |r| {
                        for skipped in &r.skipped {
                            say!("skipped {}: {}", skipped.name, skipped.reason);
                        }
                        for id in &r.inexact {
                            say!("warning: commit {id} was written with another id");
                        }
                        say!(
                            "Wrote {} and {}; {} updated",
                            format::count(r.commits, "commit"),
                            format::count(r.tags, "tag"),
                            format::count(r.refs.len() as u64, "ref")
                        );
                    });
                    if !report.inexact.is_empty() {
                        return Ok(EXIT_WARNINGS);
                    }
                }
            }
            Ok(0)
        }
        Command::Remote { command } => {
            let repo = open(cli.repo, password_file)?;
            match command {
                RemoteCommand::List => {
                    let remotes = repo.remotes()?;
                    out.result(&remotes, |remotes| {
                        for r in remotes {
                            say!("{}  {}", r.name, r.url);
                        }
                    });
                }
                RemoteCommand::Add { name, url } => {
                    let remote = repo.add_remote(&name, &url)?;
                    out.result(&remote, |r| say!("Added remote {} at {}", r.name, r.url));
                }
                RemoteCommand::Remove { name } => {
                    repo.remove_remote(&name)?;
                    out.result(&serde_json::json!({ "removed": name }), |_| {
                        say!("Removed remote {name}");
                    });
                }
            }
            Ok(0)
        }
        Command::Daemon {
            command,
            jobs,
            state_dir,
        } => run_daemon(out, command, jobs, state_dir),
        Command::Clone { url, path } => {
            let mut request = CloneRequest {
                url,
                path,
                secret: password_file.map(read_password_file).transpose()?,
            };
            let report = match watch(out, Repository::clone_from(request.clone())?) {
                Err(Failure::Api(e)) if e.kind == ErrorKind::Key && request.secret.is_none() => {
                    request.secret = Some(prompt("Password or recovery key: ")?);
                    watch(out, Repository::clone_from(request.clone())?)?
                }
                result => result?,
            };
            out.result(&report, |r| {
                for cloned in &r.refs {
                    say!("{}", transferred(&cloned.report, &cloned.name, "origin"));
                }
                say!(
                    "Cloned {} into {}; the remote is named origin",
                    format::count(r.refs.len() as u64, "ref"),
                    request.path.display()
                );
            });
            Ok(0)
        }
        Command::Sync {
            folder,
            remote,
            ref_name,
            mode,
            dry_run,
        } => {
            let repo = open(cli.repo, password_file)?;
            let mut request = SyncRequest::new(folder, remote.clone());
            request.ref_name = ref_name;
            request.mode = mode.into();
            request.dry_run = dry_run;
            let report = watch(out, repo.sync(request)?)?;
            out.result(&report, |r| print_sync(r, &remote, dry_run));
            let attention = !report.conflicts.is_empty() || !report.skipped.is_empty();
            Ok(match warnings_exit(out, &report.warnings) {
                0 if attention => EXIT_WARNINGS,
                code => code,
            })
        }
        Command::Push { remote, ref_name } => {
            let repo = open(cli.repo, password_file)?;
            let request = TransferRequest::new(remote.clone(), ref_name.clone());
            let report = watch(out, repo.push(request)?)?;
            out.result(&report, |r| say!("{}", transferred(r, &ref_name, &remote)));
            Ok(0)
        }
        Command::Pull { remote, ref_name } => {
            let repo = open(cli.repo, password_file)?;
            let request = TransferRequest::new(remote.clone(), ref_name.clone());
            let report = watch(out, repo.pull(request)?)?;
            out.result(&report, |r| say!("{}", transferred(r, &ref_name, &remote)));
            Ok(0)
        }
        Command::Repair => {
            let repo = open(cli.repo, password_file)?;
            let report = repo.repair()?;
            out.result(&report, |r| {
                for lock in &r.removed_locks {
                    say!("removed stale lock {}", lock.display());
                }
                for name in &r.repaired_refs {
                    say!("added a repair line to the reflog of {name}");
                }
                if r.indexed_packs > 0 {
                    say!(
                        "wrote a new index for {}",
                        format::count(r.indexed_packs, "pack")
                    );
                }
                for path in &r.removed_index_files {
                    say!("removed damaged index {}", path.display());
                }
                if r.removed_locks.is_empty()
                    && r.repaired_refs.is_empty()
                    && r.indexed_packs == 0
                    && r.removed_index_files.is_empty()
                {
                    say!("Nothing to repair");
                }
            });
            Ok(0)
        }
    }
}

fn run_daemon(
    out: &Output,
    command: DaemonCommand,
    jobs: Option<PathBuf>,
    state_dir: Option<PathBuf>,
) -> Result<u8, Failure> {
    use apeiron_daemon::{Daemon, Paths, RunResult};
    let daemon_error = |e: apeiron_daemon::Error| {
        let kind = match e {
            apeiron_daemon::Error::Locked(_) => ErrorKind::Locked,
            apeiron_daemon::Error::Jobs { .. } => ErrorKind::InvalidInput,
            _ => ErrorKind::Io,
        };
        Failure::Api(Error::new(kind, e.to_string()))
    };
    let mut paths = Paths::system().map_err(daemon_error)?;
    if let Some(jobs) = jobs {
        paths.jobs = jobs;
    }
    if let Some(state_dir) = state_dir {
        paths.state_dir = state_dir;
    }
    let failed = |result: RunResult| result == RunResult::Failed;
    match command {
        DaemonCommand::Run => {
            let daemon = Daemon::start(paths).map_err(daemon_error)?;
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let _ = ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed));
            daemon.run(&stop).map_err(daemon_error)?;
            Ok(0)
        }
        DaemonCommand::Once => {
            let jobs = apeiron_daemon::load_jobs(&paths.jobs).map_err(daemon_error)?;
            let daemon = Daemon::start(paths).map_err(daemon_error)?;
            let runs = daemon
                .run_due(&jobs, &AtomicBool::new(false))
                .map_err(daemon_error)?;
            out.result(&runs, |runs| {
                for r in runs {
                    say!(
                        "{}: {}: {}",
                        r.job,
                        result_name(r.run.result),
                        r.run.message
                    );
                }
                if runs.is_empty() {
                    say!("No job is due");
                }
            });
            Ok(if runs.iter().any(|r| failed(r.run.result)) {
                EXIT_DAMAGED
            } else {
                0
            })
        }
        DaemonCommand::Status => {
            let jobs = apeiron_daemon::status(&paths).map_err(daemon_error)?;
            out.result(&jobs, |jobs| {
                let now = jiff::Timestamp::now();
                for j in jobs {
                    let next = if j.next_run <= now {
                        "due now".to_owned()
                    } else {
                        format!("next {}", local_time(j.next_run))
                    };
                    match &j.last {
                        Some(last) => {
                            say!(
                                "{}  {}  {} {}  {next}",
                                j.name,
                                j.action,
                                result_name(last.result),
                                local_time(last.last_start)
                            );
                            say!("    {}", last.message);
                        }
                        None => say!("{}  {}  never run  {next}", j.name, j.action),
                    }
                }
                if jobs.is_empty() {
                    say!("No jobs; see docs/DAEMON.md for the jobs file");
                }
            });
            let any_failed = jobs
                .iter()
                .any(|j| j.last.as_ref().is_some_and(|l| failed(l.result)));
            Ok(if any_failed { EXIT_DAMAGED } else { 0 })
        }
    }
}

fn result_name(result: apeiron_daemon::RunResult) -> &'static str {
    use apeiron_daemon::RunResult;
    match result {
        RunResult::Ok => "ok",
        RunResult::Warnings => "ok with warnings",
        RunResult::Failed => "FAILED",
        RunResult::Locked => "waiting for a lock",
    }
}

fn local_time(t: jiff::Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}

/// One line about a push or pull.
fn print_sync(r: &SyncReport, remote: &str, dry_run: bool) {
    for change in &r.changes {
        let mark = match change.kind {
            ChangeKind::Added => '+',
            ChangeKind::Removed => '-',
            ChangeKind::Modified => '~',
        };
        say!("{mark} {}", change.path);
    }
    for c in &r.conflicts {
        let what = match c.kind {
            ConflictKind::BothChanged => "changed on both devices",
            ConflictKind::DeletedHere => "deleted here, changed on the other device; kept",
            ConflictKind::DeletedThere => "changed here, deleted on the other device; kept",
            ConflictKind::ChangedDuringSync => "changed here during sync",
        };
        match &c.copy {
            Some(copy) => say!("conflict: {} ({what}); the other version is {copy}", c.path),
            None => say!("conflict: {} ({what})", c.path),
        }
    }
    for path in &r.skipped {
        say!("left alone, changed during sync: {path}");
    }
    let would = if dry_run { "would be " } else { "" };
    let short = r.snapshot.map_or("-".to_owned(), |s| format::short_id(&s));
    match r.action {
        SyncAction::UpToDate => say!("{} is up to date with {remote} ({short})", r.ref_name),
        SyncAction::Sent => say!(
            "{}: changes here {would}sent to {remote} ({short})",
            r.ref_name
        ),
        SyncAction::Received => say!(
            "{}: changes from {remote} {would}applied ({})",
            r.ref_name,
            format::count(r.changes.len() as u64, "change")
        ),
        SyncAction::Merged => say!(
            "{}: changes from both sides {would}merged ({}, {})",
            r.ref_name,
            format::count(r.changes.len() as u64, "change"),
            format::count(r.conflicts.len() as u64, "conflict")
        ),
    }
}

fn transferred(r: &TransferReport, ref_name: &str, remote: &str) -> String {
    if r.previous == Some(r.snapshot) || r.objects == 0 && r.previous.is_some() {
        return format!(
            "{ref_name} is up to date with {remote} ({})",
            format::short_id(&r.snapshot)
        );
    }
    let from = r
        .previous
        .map_or("new".to_owned(), |p| format::short_id(&p));
    format!(
        "{ref_name}: {from} → {}, {} ({}) copied",
        format::short_id(&r.snapshot),
        format::count(r.objects, "object"),
        format::bytes(r.bytes)
    )
}

fn warnings_exit(out: &Output, warnings: &[Warning]) -> u8 {
    if warnings.is_empty() {
        return 0;
    }
    if !out.json {
        for w in warnings {
            eprintln!("warning: {}: {}", w.path, w.message);
        }
    }
    EXIT_WARNINGS
}

/// Waits for a job, showing progress on a terminal and cancelling it on Ctrl+C.
fn watch<T>(out: &Output, mut job: Job<T>) -> Result<T, Failure> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = interrupted.clone();
    // Another handler may already be installed (only in tests); then Ctrl+C keeps its default.
    let _ = ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed));
    let show = !out.json && std::io::stderr().is_terminal();
    let mut last = Instant::now();
    let mut cancelled = false;
    loop {
        if let Some(result) = job.wait_timeout(Duration::from_millis(100)) {
            if show {
                eprint!("\r\x1b[K");
            }
            return Ok(result?);
        }
        if interrupted.load(Ordering::Relaxed) && !cancelled {
            cancelled = true;
            job.cancel();
            if show {
                eprint!("\r\x1b[Kcancelling…");
            }
        }
        if show && !cancelled && last.elapsed() >= Duration::from_millis(200) {
            last = Instant::now();
            eprint!("\r\x1b[K{}", format::progress(&job.progress()));
            let _ = std::io::stderr().flush();
        }
    }
}

fn write_line(args: std::fmt::Arguments<'_>) {
    let mut stdout = std::io::stdout().lock();
    if let Err(e) = stdout
        .write_fmt(args)
        .and_then(|()| stdout.write_all(b"\n"))
    {
        if e.kind() != std::io::ErrorKind::BrokenPipe {
            eprintln!("error: cannot write output: {e}");
        }
        std::process::exit(0);
    }
}

struct Output {
    json: bool,
}

impl Output {
    /// Prints `value` as JSON, or calls `human` to print it for people.
    fn result<T: Serialize>(&self, value: &T, human: impl FnOnce(&T)) {
        if self.json {
            self.json_value(value);
        } else {
            human(value);
        }
    }

    fn json_value<T: Serialize>(&self, value: &T) {
        match serde_json::to_string_pretty(value) {
            Ok(text) => say!("{text}"),
            Err(e) => eprintln!("error: cannot write JSON: {e}"),
        }
    }
}
