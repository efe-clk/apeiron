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
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use apeiron::{
    ArchiveFormat, BackupRequest, DiffRequest, Error, ErrorKind, ExportRequest, GcRequest, Job,
    ListRequest, LogRequest, Name, Repository, RestoreRequest, Warning,
};
use clap::{Parser, Subcommand};
use serde::Serialize;

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

    /// Print the result as one JSON document on standard output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an empty repository
    Init { path: PathBuf },
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

fn open(cli_repo: Option<PathBuf>) -> Result<Repository, Failure> {
    let path = cli_repo.ok_or_else(|| {
        Failure::Usage("no repository given; use --repo or set APEIRON_REPO".to_owned())
    })?;
    let repo = Repository::open(path)?;
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

fn run(cli: Cli, out: &Output) -> Result<u8, Failure> {
    match cli.command {
        Command::Init { path } => {
            let repo = Repository::init(&path)?;
            out.result(&repo.info(), |info| {
                say!("Created an empty repository at {}", info.path.display());
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
            let repo = open(cli.repo)?;
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
            let repo = open(cli.repo)?;
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
            let repo = open(cli.repo)?;
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
            let repo = open(cli.repo)?;
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
            let repo = open(cli.repo)?;
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
            let repo = open(cli.repo)?;
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
        Command::Refs => {
            let repo = open(cli.repo)?;
            let refs = repo.refs()?;
            out.result(&refs, |refs| {
                for r in refs {
                    say!("{}  {}", format::short_id(&r.snapshot), r.name);
                }
            });
            Ok(0)
        }
        Command::Verify => {
            let repo = open(cli.repo)?;
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
            let repo = open(cli.repo)?;
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
        Command::Repair => {
            let repo = open(cli.repo)?;
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
