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
mod sign;
mod standard;

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
use apeiron::{DuplicatesRequest, Query, SearchRequest, Time};
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
        /// Only snapshots whose message, author, host or metadata hold this text
        #[arg(long)]
        grep: Option<String>,
    },
    /// List the files and folders inside a snapshot
    Ls {
        snapshot: String,
        #[arg(default_value = "")]
        path: String,
    },
    /// List files added, removed and changed between two snapshots
    Diff {
        from: String,
        to: String,
        /// Also show the changed lines of files a plugin handles, such as text
        #[arg(long)]
        text: bool,
    },
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
        /// Clean this remote instead, one only this repository writes to, such as a replica
        #[arg(long, value_name = "NAME")]
        remote: Option<String>,
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
    /// Make a compliance report of the repository and, with --web, of the web
    /// server (docs/REPORTS.md)
    #[command(args_conflicts_with_subcommands = true)]
    Report {
        #[command(subcommand)]
        command: Option<ReportCommand>,
        /// Write the report to this file instead of printing it
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Sign the written report with this key; the signature goes to
        /// <out>.sig
        #[arg(long, value_name = "KEY", requires = "out")]
        sign: Option<PathBuf>,
        /// Also read and check every stored byte, as `verify` does
        #[arg(long)]
        verify: bool,
        /// Also compare every ref with each remote
        #[arg(long)]
        remotes: bool,
        /// Days without a new snapshot after which a ref is stale; 0 checks nothing
        #[arg(long, default_value_t = 7)]
        stale_days: u32,
        /// Add the web server's users, identity provider and audit log
        #[arg(long)]
        web: bool,
        /// The tenant whose web server files to report on
        #[arg(long, requires = "web", value_name = "NAME")]
        tenant: Option<String>,
        /// One report for each tenant, on its own repository and files
        #[arg(long, conflicts_with = "tenant")]
        all_tenants: bool,
        /// Count audit events from this day on (YYYY-MM-DD); default 30 days ago
        #[arg(long, requires = "web", value_parser = parse_date)]
        since: Option<Time>,
        /// Map the facts to the controls of a standard
        #[arg(long, value_enum)]
        standard: Option<standard::Standard>,
    },
    /// Keep refs as an archive that cannot be changed (docs/ARCHIVE.md)
    Retention {
        #[command(subcommand)]
        command: RetentionCommand,
    },
    /// Show or change the tags of files on disk, which backups keep and
    /// search finds (Linux; docs/SEARCH.md)
    Tag {
        files: Vec<PathBuf>,
        /// A tag to add; give it more than once for several
        #[arg(long = "add", value_name = "TAG")]
        add: Vec<String>,
        /// A tag to remove
        #[arg(long = "remove", value_name = "TAG")]
        remove: Vec<String>,
    },
    /// Place, list and lift legal holds (docs/ARCHIVE.md)
    Hold {
        #[command(subcommand)]
        command: HoldCommand,
    },
    /// Find files by name, size, time and text in a snapshot or its history
    Search {
        /// A name with `*` and `?`; with `/`, a whole path
        pattern: Option<String>,
        /// Text a text file must hold
        #[arg(long)]
        text: Option<String>,
        /// Files of at least this size, such as 10M
        #[arg(long, value_parser = parse_size)]
        larger: Option<u64>,
        /// Files smaller than this size
        #[arg(long, value_parser = parse_size)]
        smaller: Option<u64>,
        /// Modified on or after this day (YYYY-MM-DD)
        #[arg(long, value_parser = parse_date)]
        after: Option<Time>,
        /// Modified before this day (YYYY-MM-DD)
        #[arg(long, value_parser = parse_date)]
        before: Option<Time>,
        /// A tag files must have; give it more than once for several
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// The snapshot searched (default main)
        #[arg(long = "in")]
        snapshot: Option<String>,
        /// Also search every earlier snapshot; every ref without --in
        #[arg(long)]
        history: bool,
        /// Show at most this many results (default 1000)
        #[arg(long)]
        limit: Option<usize>,
        /// Run the search saved under this name; options given here change it
        #[arg(long, value_name = "NAME")]
        saved: Option<String>,
        /// Save this search under a name, then run it
        #[arg(long, value_name = "NAME")]
        save: Option<String>,
        /// List the saved searches
        #[arg(long, conflicts_with_all = ["saved", "save", "forget"])]
        list_saved: bool,
        /// Remove the search saved under this name
        #[arg(long, value_name = "NAME", conflicts_with_all = ["saved", "save"])]
        forget: Option<String>,
    },
    /// List files whose content is at more than one path in a snapshot
    Duplicates {
        #[arg(default_value = "main")]
        snapshot: String,
        /// Leave out files smaller than this size, such as 1M
        #[arg(long, value_parser = parse_size, default_value = "1")]
        smallest: u64,
    },
    /// Install, list or remove plugins
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// Copy the snapshots of a branch that another snapshot lacks on top of
    /// it, and move the branch there
    Rebase {
        /// The ref that moves
        branch: String,
        /// The snapshot to build on, usually main
        #[arg(long, default_value = "main")]
        onto: String,
        /// Only show what the rebase would do
        #[arg(long)]
        dry_run: bool,
    },
    /// Copy the changes one snapshot made into a ref, as a new snapshot there
    CherryPick {
        snapshot: String,
        /// The ref that moves
        #[arg(long, default_value = "main")]
        into: String,
        #[arg(long)]
        message: Option<String>,
        /// Only show what the cherry-pick would do
        #[arg(long)]
        dry_run: bool,
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
    /// Open the desktop interface in the browser (docs/DESKTOP.md)
    Ui {
        /// Print the address instead of opening a browser
        #[arg(long)]
        no_browser: bool,
    },
    /// Serve the interface to other computers, with sign-in and TLS (docs/WEB.md)
    #[command(args_conflicts_with_subcommands = true)]
    Web {
        #[command(subcommand)]
        command: Option<WebCommand>,
        /// Address and port to listen on
        #[arg(long, default_value = "0.0.0.0:8443", value_name = "ADDRESS")]
        listen: std::net::SocketAddr,
        /// Certificate chain, PEM; default a self-signed certificate
        #[arg(long, value_name = "PATH", requires = "key")]
        cert: Option<PathBuf>,
        /// Private key of the certificate, PEM
        #[arg(long, value_name = "PATH", requires = "cert")]
        key: Option<PathBuf>,
        /// Plain HTTP, only behind a proxy that adds TLS
        #[arg(long, conflicts_with = "cert")]
        plain: bool,
        /// A folder users may be given; default the home folder
        #[arg(long = "folder", value_name = "PATH")]
        folders: Vec<PathBuf>,
        /// Serve every tenant at its own host instead of one repository
        /// (docs/TENANTS.md)
        #[arg(long, conflicts_with = "folders")]
        tenants: bool,
        /// A remote every ref is pushed to after each change, to keep a
        /// replica (docs/REPLICATION.md)
        #[arg(long = "replicate", value_name = "REMOTE", conflicts_with = "tenants")]
        replicas: Vec<String>,
        /// Serve a standby copy: everything but backups (docs/REPLICATION.md)
        #[arg(long)]
        standby: bool,
        /// Also send each audit event to syslog: udp://host:port or a local
        /// socket such as /dev/log (docs/AUDIT.md)
        #[arg(long, value_name = "TARGET")]
        audit_syslog: Option<apeiron_server::Syslog>,
        /// The tenant whose users, audit log, share links or identity
        /// provider to manage
        #[arg(long, global = true, value_name = "NAME")]
        tenant: Option<String>,
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
        /// CPU and disk priority of the jobs; low lets other programs go first
        #[arg(long, global = true, value_enum, default_value = "low")]
        priority: PriorityArg,
        /// Run the jobs of this tenant of the web server, which reach only
        /// its repository and folders (docs/DAEMON.md)
        #[arg(long, global = true, value_name = "NAME", conflicts_with_all = ["jobs", "state_dir"])]
        tenant: Option<String>,
    },
    /// Make a new repository from a remote copy and pull all its refs
    Clone {
        /// A folder, sftp://[user@]host[:port]/path or s3://bucket/path
        url: String,
        /// The new repository; must not exist or be empty
        path: PathBuf,
        /// Move at most this much per second on average, such as 500K or 2M
        #[arg(long, value_parser = parse_rate)]
        rate: Option<u64>,
    },
    /// Make a working folder hold a branch, recording what it held first
    Switch {
        /// The snapshot to switch to, usually a branch
        snapshot: String,
        folder: PathBuf,
        /// Only show what would change
        #[arg(long)]
        dry_run: bool,
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
        /// Move at most this much per second on average, such as 500K or 2M
        #[arg(long, value_parser = parse_rate)]
        rate: Option<u64>,
    },
    /// Copy the snapshots of a ref to a remote and move the ref there
    Push {
        remote: String,
        #[arg(default_value = "main")]
        ref_name: String,
        /// Push every ref, to keep a replica (docs/REPLICATION.md)
        #[arg(long, conflicts_with = "ref_name")]
        all: bool,
        /// Move at most this much per second on average, such as 500K or 2M
        #[arg(long, value_parser = parse_rate)]
        rate: Option<u64>,
    },
    /// Copy the snapshots of a ref from a remote and move the local ref forward
    Pull {
        remote: String,
        #[arg(default_value = "main")]
        ref_name: String,
        /// Pull every ref of the remote, to keep a standby (docs/REPLICATION.md)
        #[arg(long, conflicts_with = "ref_name")]
        all: bool,
        /// Move at most this much per second on average, such as 500K or 2M
        #[arg(long, value_parser = parse_rate)]
        rate: Option<u64>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum PriorityArg {
    Low,
    Normal,
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Run until stopped, starting each job when it is due
    Run {
        /// Show no desktop notification when a job fails
        #[arg(long)]
        no_notify: bool,
    },
    /// Run every job that is due now, then exit
    Once,
    /// Show each job's last run, its result and the next run
    Status,
}

#[derive(Subcommand)]
enum RetentionCommand {
    /// Keep the refs a name or a prefix followed by * matches: they then only
    /// move forward and are not deleted until the days have passed since their
    /// last change. A rule can never be removed or shortened.
    Add {
        /// A ref name, or a prefix followed by *, such as "archive/*"
        refs: String,
        #[arg(long)]
        days: u32,
    },
    /// List the retention rules
    List,
}

#[derive(Subcommand)]
enum ReportCommand {
    /// Make a key to sign reports with; prints its public key
    Key {
        /// The new key file; it must not exist
        file: PathBuf,
    },
    /// Check the signature of a report written with --out and --sign
    Check {
        /// The report; its signature is <file>.sig
        file: PathBuf,
        /// The public key the report must be signed with, hex
        #[arg(long)]
        public_key: Option<String>,
    },
}

fn report_command(out: &Output, command: ReportCommand) -> Result<u8, Failure> {
    match command {
        ReportCommand::Key { file } => {
            let public = sign::new_key(&file)?;
            out.result(&serde_json::json!({ "public_key": public }), |_| {
                say!("Made the key {}; keep it secret.", file.display());
                say!("Its public key, to give whoever checks the reports: {public}");
            });
            Ok(0)
        }
        ReportCommand::Check { file, public_key } => {
            let read = |path: &Path| {
                std::fs::read(path)
                    .map_err(|e| Failure::Usage(format!("cannot read {}: {e}", path.display())))
            };
            let bytes = read(&file)?;
            let sig_file = sign::signature_file(&file);
            let signature: sign::Signature = serde_json::from_slice(&read(&sig_file)?)
                .map_err(|e| Failure::Usage(format!("{}: {e}", sig_file.display())))?;
            match sign::check(&bytes, &signature, public_key.as_deref()) {
                Ok(()) => {
                    out.result(
                        &serde_json::json!({ "public_key": signature.public_key }),
                        |_| {
                            say!(
                                "The report is as it was signed, by the key {}",
                                signature.public_key
                            );
                        },
                    );
                    Ok(0)
                }
                Err(e) => {
                    eprintln!("error: {}", e.message);
                    Ok(EXIT_DAMAGED)
                }
            }
        }
    }
}

#[derive(Subcommand)]
enum HoldCommand {
    /// Keep the refs a name or a prefix followed by * matches, with every
    /// version they had, until the hold is lifted
    Add {
        /// A ref name, or a prefix followed by *, such as "backup/finance-*"
        refs: String,
        /// Why, such as a case number; one line
        #[arg(long)]
        reason: String,
    },
    /// List the holds, lifted ones too
    List,
    /// Lift a hold; this is recorded and cannot be undone
    Lift {
        id: String,
        #[arg(long)]
        reason: String,
    },
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
enum WebCommand {
    /// Manage the users who may sign in (docs/USERS.md)
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
    /// Read and check the audit log (docs/AUDIT.md)
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
    /// List and remove share links (docs/SHARE.md)
    Share {
        #[command(subcommand)]
        command: ShareCommand,
    },
    /// Sign in through an identity provider with OpenID Connect (docs/IDENTITY.md)
    Oidc {
        #[command(subcommand)]
        command: OidcCommand,
    },
    /// Manage the organizations served, each at its own host (docs/TENANTS.md)
    Tenant {
        #[command(subcommand)]
        command: TenantCommand,
    },
}

#[derive(Subcommand)]
enum TenantCommand {
    /// Add a tenant with its own repository, users, audit log and links
    Add {
        name: String,
        /// The host name browsers reach it at, such as acme.backup.example.com
        #[arg(long)]
        host: String,
        /// Its repository, made with `apeiron init`
        #[arg(long, value_name = "PATH")]
        repo: PathBuf,
        /// A folder its users may be given
        #[arg(long = "folder", value_name = "PATH")]
        folders: Vec<PathBuf>,
        /// Read the password of an encrypted repository from this file
        #[arg(long, value_name = "PATH")]
        password_file: Option<PathBuf>,
        /// A remote of its repository every ref is pushed to after each
        /// change (docs/REPLICATION.md)
        #[arg(long = "replicate", value_name = "REMOTE")]
        replicas: Vec<String>,
        /// The most its repository may hold, such as 500G
        #[arg(long, value_parser = parse_size, value_name = "SIZE")]
        max_bytes: Option<u64>,
        /// The most long operations it may run at once
        #[arg(long, value_name = "N")]
        max_jobs: Option<u32>,
        /// Its admins manage the tenants in the pages
        #[arg(long)]
        operator: bool,
    },
    /// Set or lift the limits of a tenant; 0 lifts one
    Limit {
        name: String,
        /// The most its repository may hold, such as 500G
        #[arg(long, value_parser = parse_size, value_name = "SIZE")]
        max_bytes: Option<u64>,
        /// The most long operations it may run at once
        #[arg(long, value_name = "N")]
        max_jobs: Option<u32>,
    },
    /// List the tenants
    List,
    /// Stop serving a tenant; its repository and files stay
    Remove { name: String },
}

/// Where the files of the web server, or of one of its tenants, are
/// (docs/TENANTS.md §2).
struct WebFiles {
    users: PathBuf,
    audit: apeiron_server::Audit,
    shares: apeiron_server::Shares,
}

/// The configuration and state folders of Apeiron, such as
/// `~/.config/apeiron` and `~/.local/state/apeiron`.
fn web_roots() -> Result<(PathBuf, PathBuf), Failure> {
    let config =
        dirs::config_dir().ok_or_else(|| Failure::Usage("no configuration folder".to_owned()))?;
    let state = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .ok_or_else(|| Failure::Usage("no state folder".to_owned()))?;
    Ok((config.join("apeiron"), state.join("apeiron")))
}

fn tenants_file() -> Result<PathBuf, Failure> {
    apeiron_server::Tenants::default_file()
        .ok_or_else(|| Failure::Usage("no configuration folder".to_owned()))
}

impl WebFiles {
    fn of(tenant: Option<&str>) -> Result<WebFiles, Failure> {
        let Some(name) = tenant else {
            let config = dirs::config_dir()
                .ok_or_else(|| Failure::Usage("no configuration folder".to_owned()))?;
            return Ok(WebFiles {
                users: config.join("apeiron").join("web.json"),
                audit: apeiron_server::Audit::default_file()
                    .map(apeiron_server::Audit::new)
                    .ok_or_else(|| {
                        Failure::Usage("no state folder for the audit log".to_owned())
                    })?,
                shares: apeiron_server::Shares::default_file()
                    .map(apeiron_server::Shares::new)
                    .ok_or_else(|| Failure::Usage("no state folder for share links".to_owned()))?,
            });
        };
        let tenants = apeiron_server::Tenants::load(&tenants_file()?)?;
        WebFiles::of_tenant(tenants.get(name)?)
    }

    fn of_tenant(t: &apeiron_server::Tenant) -> Result<WebFiles, Failure> {
        let (Some(config), Some(state)) = (t.config_dir(), t.state_dir()) else {
            return Err(Failure::Usage(
                "no configuration or state folder".to_owned(),
            ));
        };
        Ok(WebFiles {
            users: config.join("web.json"),
            audit: apeiron_server::Audit::new(state.join("audit.log")),
            shares: apeiron_server::Shares::new(state.join("shares.json")),
        })
    }
}

/// `audit`, sending a copy of each event to `syslog` if given.
fn copy_audit(
    audit: apeiron_server::Audit,
    syslog: &Option<apeiron_server::Syslog>,
    tenant: Option<&str>,
) -> apeiron_server::Audit {
    match syslog {
        Some(s) => audit.copying_to(s.clone(), tenant),
        None => audit,
    }
}

fn web_tenant(command: TenantCommand, out: &Output) -> Result<(), Failure> {
    let file = tenants_file()?;
    let mut tenants = apeiron_server::Tenants::load(&file)?;
    match command {
        TenantCommand::Add {
            name,
            host,
            repo,
            folders,
            password_file,
            replicas,
            max_bytes,
            max_jobs,
            operator,
        } => {
            let full = |p: PathBuf| std::path::absolute(&p).unwrap_or(p);
            let tenant = apeiron_server::Tenant {
                name,
                host: host.to_ascii_lowercase(),
                repo: full(repo),
                folders: folders.into_iter().map(full).collect(),
                password_file: password_file.map(full),
                replicas,
                max_bytes: max_bytes.filter(|&n| n > 0),
                max_jobs: max_jobs.filter(|&n| n > 0),
                operator,
            };
            tenants.add(tenant.clone())?;
            tenants.save(&file)?;
            WebFiles::of_tenant(&tenant)?.audit.record(
                &apeiron_server::Actor::default(),
                "tenant_add",
                serde_json::to_value(&tenant).unwrap_or_default(),
                "ok",
            )?;
            out.result(&tenant, |t| {
                say!("Added the tenant {} at {}", t.name, t.host);
                say!(
                    "Add its first admin with `apeiron web user add <name> --role admin --tenant {}`",
                    t.name
                );
            });
        }
        TenantCommand::List => {
            out.result(&tenants.tenants, |list| {
                for t in list {
                    say!("{}  {}  {}", t.name, t.host, t.repo.display());
                }
                if list.is_empty() {
                    say!("No tenants; add one with `apeiron web tenant add`");
                }
            });
        }
        TenantCommand::Limit {
            name,
            max_bytes,
            max_jobs,
        } => {
            let tenant = tenants.get_mut(&name)?;
            if let Some(n) = max_bytes {
                tenant.max_bytes = (n > 0).then_some(n);
            }
            if let Some(n) = max_jobs {
                tenant.max_jobs = (n > 0).then_some(n);
            }
            let tenant = tenant.clone();
            tenants.save(&file)?;
            WebFiles::of_tenant(&tenant)?.audit.record(
                &apeiron_server::Actor::default(),
                "tenant_limit",
                serde_json::json!({
                    "name": tenant.name,
                    "max_bytes": tenant.max_bytes,
                    "max_jobs": tenant.max_jobs,
                }),
                "ok",
            )?;
            out.result(&tenant, |t| {
                let bytes = t.max_bytes.map_or("none".to_owned(), format::bytes);
                let jobs = t.max_jobs.map_or("none".to_owned(), |n| n.to_string());
                say!(
                    "Limits of {}: storage {bytes}, jobs at once {jobs}; restart the server to apply them",
                    t.name
                );
            });
        }
        TenantCommand::Remove { name } => {
            let removed = tenants.remove(&name)?;
            tenants.save(&file)?;
            WebFiles::of_tenant(&removed)?.audit.record(
                &apeiron_server::Actor::default(),
                "tenant_remove",
                serde_json::json!({ "name": removed.name }),
                "ok",
            )?;
            out.result(&removed, |t| {
                say!(
                    "Removed the tenant {}; its repository and files stay",
                    t.name
                )
            });
        }
    }
    Ok(())
}

#[derive(Subcommand)]
enum OidcCommand {
    /// Set up the identity provider; asks for the client secret
    Set {
        /// The provider's issuer URL
        #[arg(long)]
        issuer: String,
        /// The id of this server at the provider
        #[arg(long)]
        client_id: String,
        /// The address browsers reach this server at, such as https://backup.example.com
        #[arg(long)]
        address: String,
        /// The claim of the ID token that names a user
        #[arg(long, default_value = "email")]
        claim: String,
        /// Read the client secret from this file instead of asking; an empty
        /// file means none
        #[arg(long, value_name = "PATH")]
        client_secret_file: Option<PathBuf>,
        /// The claim of the ID token that lists the user's groups
        #[arg(long, default_value = "groups")]
        groups_claim: String,
    },
    /// Make users from a group of the provider: its members sign in with
    /// this role, backups and folders; `{user}` is the user's name
    Group {
        group: String,
        #[arg(long, value_enum)]
        role: RoleArg,
        /// A backup they may use, such as backup/{user}-*
        #[arg(long = "backup", value_name = "PATTERN")]
        backups: Vec<String>,
        /// A folder of the server they may be given
        #[arg(long = "folder", value_name = "PATH")]
        folders: Vec<String>,
    },
    /// Stop making users from a group; users it made can no longer sign in
    /// unless another group's rule covers them
    Ungroup { group: String },
    /// Show the settings, without the secret
    Show,
    /// Read the provider's settings and keys, to see that it can be reached
    Check,
    /// Turn signing in through the provider off
    Remove,
}

fn web_oidc(files: &WebFiles, command: OidcCommand, out: &Output) -> Result<(), Failure> {
    let file = &files.users;
    let _lock = apeiron_server::Users::lock(file)?;
    let mut users = apeiron_server::Users::load(file)?;
    let not_set =
        || Failure::Usage("no identity provider is set up; use `apeiron web oidc set`".to_owned());
    // The secret is never shown or recorded.
    let view = |c: &apeiron_server::OidcConfig| {
        serde_json::json!({
            "issuer": c.issuer, "client_id": c.client_id, "address": c.address,
            "claim": c.claim, "secret": c.client_secret.is_some(),
            "groups_claim": c.groups_claim, "groups": c.groups,
        })
    };
    match command {
        OidcCommand::Set {
            issuer,
            client_id,
            address,
            claim,
            client_secret_file,
            groups_claim,
        } => {
            let secret = match client_secret_file {
                Some(path) => std::fs::read_to_string(&path)
                    .map_err(|e| Failure::Usage(format!("cannot read {}: {e}", path.display())))?
                    .trim()
                    .to_owned(),
                None => prompt("Client secret (empty if there is none): ")?,
            };
            let config = apeiron_server::OidcConfig {
                issuer,
                client_id,
                client_secret: Some(secret).filter(|s| !s.is_empty()),
                address,
                claim,
                groups_claim,
                // Setting the provider again keeps the group rules.
                groups: users
                    .oidc
                    .as_ref()
                    .map(|c| c.groups.clone())
                    .unwrap_or_default(),
            };
            config.check()?;
            say!(
                "Redirect address to register at the provider: {}",
                config.redirect()
            );
            say!(
                "Address to register for after signing out: {}",
                config.signed_out()
            );
            let detail = view(&config);
            users.oidc = Some(config);
            users.save(file)?;
            files
                .audit
                .record(&apeiron_server::Actor::default(), "oidc_set", detail, "ok")?;
        }
        OidcCommand::Group {
            group,
            role,
            backups,
            folders,
        } => {
            let config = users.oidc.as_mut().ok_or_else(not_set)?;
            let rule = apeiron_server::GroupRule {
                group,
                role: role.into(),
                backups,
                folders,
            };
            match config.groups.iter_mut().find(|r| r.group == rule.group) {
                Some(known) => *known = rule.clone(),
                None => config.groups.push(rule.clone()),
            }
            users.save(file)?;
            files.audit.record(
                &apeiron_server::Actor::default(),
                "oidc_group",
                serde_json::to_value(&rule).unwrap_or_default(),
                "ok",
            )?;
            out.result(&rule, |r| {
                say!(
                    "Members of {} sign in as {} users; rules apply in the order they were added",
                    r.group,
                    r.role.name()
                )
            });
        }
        OidcCommand::Ungroup { group } => {
            let config = users.oidc.as_mut().ok_or_else(not_set)?;
            let before = config.groups.len();
            config.groups.retain(|r| r.group != group);
            if config.groups.len() == before {
                return Err(Failure::Usage(format!("no rule for the group {group}")));
            }
            users.save(file)?;
            files.audit.record(
                &apeiron_server::Actor::default(),
                "oidc_ungroup",
                serde_json::json!({ "group": group }),
                "ok",
            )?;
            say!("Members of {group} are no longer made users.");
        }
        OidcCommand::Show => {
            let config = users.oidc.as_ref().ok_or_else(not_set)?;
            out.result(&view(config), |_| {
                say!("issuer: {}", config.issuer);
                say!("client id: {}", config.client_id);
                say!(
                    "client secret: {}",
                    if config.client_secret.is_some() {
                        "set"
                    } else {
                        "none"
                    }
                );
                say!("address: {}", config.address);
                say!("redirect address: {}", config.redirect());
                say!("address after signing out: {}", config.signed_out());
                say!("claim: {}", config.claim);
                say!("groups claim: {}", config.groups_claim);
                for r in &config.groups {
                    say!(
                        "group {}: {}, backups {}, folders {}",
                        r.group,
                        r.role.name(),
                        r.backups.join(" "),
                        r.folders.join(" ")
                    );
                }
            });
        }
        OidcCommand::Check => {
            let config = users.oidc.as_ref().ok_or_else(not_set)?;
            let (provider, keys) = apeiron_server::check_provider(config)?;
            out.result(&provider, |p| {
                say!("The identity provider {} answers.", p.issuer);
                say!("Sign-in page: {}", p.authorization_endpoint);
                say!("Token address: {}", p.token_endpoint);
                say!("Signing keys: {keys}");
            });
        }
        OidcCommand::Remove => {
            users.oidc.take().ok_or_else(not_set)?;
            users.save(file)?;
            files.audit.record(
                &apeiron_server::Actor::default(),
                "oidc_remove",
                serde_json::Value::Null,
                "ok",
            )?;
            say!("Signing in through the identity provider is off.");
        }
    }
    Ok(())
}

#[derive(Subcommand)]
enum ShareCommand {
    /// List the links that have not ended
    List,
    /// Remove a link, so it no longer works
    Remove { id: String },
}

fn web_share(files: &WebFiles, command: ShareCommand, out: &Output) -> Result<(), Failure> {
    let shares = &files.shares;
    match command {
        ShareCommand::List => {
            let list: Vec<apeiron_server::Share> = shares
                .list()?
                .iter()
                .map(apeiron_server::Share::public)
                .collect();
            out.result(&list, |list| {
                for s in list {
                    // An upload link names the backup it fills (SHARE.md §7).
                    let (what, counted) = match &s.upload {
                        Some(target) => (format!("upload into {target}"), "files"),
                        None => (s.snapshot.to_string()[..12].to_owned(), "downloads"),
                    };
                    say!(
                        "{}  {}  {what}:/{}  until {}{}{}",
                        s.id,
                        s.owner,
                        s.path,
                        s.expires,
                        if s.protected { "  (password)" } else { "" },
                        s.max_downloads
                            .map(|max| format!("  ({} of {max} {counted})", s.downloads))
                            .unwrap_or_default()
                    );
                }
                if list.is_empty() {
                    say!("No share links.");
                }
            });
        }
        ShareCommand::Remove { id } => {
            let removed = shares.remove(&id, |_| true)?;
            files.audit.record(
                &apeiron_server::Actor::default(),
                "share_remove",
                serde_json::json!({ "id": removed.id }),
                "ok",
            )?;
            out.result(&removed.public(), |s| say!("Removed link {}", s.id));
        }
    }
    Ok(())
}

#[derive(Subcommand)]
enum AuditCommand {
    /// Show the newest events, oldest first
    Show {
        /// Only the events of this user
        #[arg(long)]
        user: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Check that no event was changed, removed or reordered
    Verify,
}

fn web_audit(files: &WebFiles, command: AuditCommand, out: &Output) -> Result<u8, Failure> {
    let audit = &files.audit;
    match command {
        AuditCommand::Show { user, limit } => {
            let events = audit.read(limit, user.as_deref())?;
            out.result(&events, |events| {
                for e in events {
                    say!(
                        "{} {:>6}  {}  {}  {}  {}{}",
                        e.time,
                        e.seq,
                        e.user.as_deref().unwrap_or("-"),
                        e.event,
                        e.outcome,
                        if e.detail.is_null() {
                            String::new()
                        } else {
                            e.detail.to_string()
                        },
                        e.from
                            .as_deref()
                            .map(|f| format!("  from {f}"))
                            .unwrap_or_default()
                    );
                }
                if events.is_empty() {
                    say!("No events in {}", audit.file().display());
                }
            });
            Ok(0)
        }
        AuditCommand::Verify => match audit.verify() {
            Ok(v) => {
                out.result(&v, |v| {
                    if v.first > 1 {
                        say!(
                            "The audit log is whole from event {} on: {} events; the parts before it were removed.",
                            v.first,
                            v.events
                        );
                    } else {
                        say!("The audit log is whole: {} events.", v.events);
                    }
                    say!("Hash of the last event: {}", v.last);
                });
                Ok(0)
            }
            Err(e) if e.kind == ErrorKind::Corrupt => {
                eprintln!("error: {}", e.message);
                Ok(EXIT_DAMAGED)
            }
            Err(e) => Err(e.into()),
        },
    }
}

#[derive(Subcommand)]
enum UserCommand {
    /// Add a user; asks for the password
    Add {
        name: String,
        #[arg(long, value_enum)]
        role: RoleArg,
        /// A backup (ref) the user may use; `*` at the end for a prefix
        #[arg(long = "backup", value_name = "REF")]
        backups: Vec<String>,
        /// A folder of the server the user may use
        #[arg(long = "folder", value_name = "PATH")]
        folders: Vec<PathBuf>,
        /// The user's identity at the identity provider, such as an e-mail
        /// address; the user then needs no password
        #[arg(long, value_name = "IDENTITY")]
        oidc: Option<String>,
    },
    /// Replace the role, backups or folders of a user
    Change {
        name: String,
        #[arg(long, value_enum)]
        role: Option<RoleArg>,
        #[arg(long = "backup", value_name = "REF")]
        backups: Vec<String>,
        #[arg(long = "folder", value_name = "PATH")]
        folders: Vec<PathBuf>,
        /// The user's identity at the identity provider
        #[arg(long, value_name = "IDENTITY")]
        oidc: Option<String>,
        /// Remove the user's password, so it signs in only through the provider
        #[arg(long)]
        no_password: bool,
    },
    /// Set a new password for a user; ends the user's sessions
    Password { name: String },
    /// Make signing in with the user's password also ask for a code from an
    /// authenticator app; shows the secret to enter there. Ends the user's
    /// sessions
    SecondFactor {
        name: String,
        /// Turn it off again
        #[arg(long)]
        off: bool,
    },
    /// Remove a user
    Remove { name: String },
    /// List the users
    List,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum RoleArg {
    Admin,
    Member,
    Reader,
}

impl From<RoleArg> for apeiron_server::Role {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::Admin => apeiron_server::Role::Admin,
            RoleArg::Member => apeiron_server::Role::Member,
            RoleArg::Reader => apeiron_server::Role::Reader,
        }
    }
}

/// A user as listed, without the password hash.
#[derive(Serialize)]
struct UserView {
    name: String,
    role: apeiron_server::Role,
    backups: Vec<String>,
    folders: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oidc: Option<String>,
    /// Whether signing in with the password also needs an authenticator code.
    second_factor: bool,
}

/// A compliance report with, on request, the web server's part
/// (docs/REPORTS.md §3).
#[derive(Serialize)]
struct FullReport {
    /// The tenant reported on, with `--all-tenants`.
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant: Option<String>,
    #[serde(flatten)]
    report: apeiron::ComplianceReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    web: Option<WebFacts>,
    /// With `--standard`: the facts mapped to its controls.
    #[serde(skip_serializing_if = "Option::is_none")]
    standard: Option<standard::Mapping>,
}

#[derive(Serialize)]
struct WebFacts {
    users: Vec<UserView>,
    /// The issuer of the identity provider, when one is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    identity_provider: Option<String>,
    audit: AuditFacts,
}

#[derive(Serialize)]
struct AuditFacts {
    file: PathBuf,
    /// Whether the chain of hashes is whole (docs/AUDIT.md §3).
    whole: bool,
    /// The first event still in the log; more than 1 when older parts were
    /// removed (docs/AUDIT.md §3).
    first: u64,
    events: u64,
    /// The hash of the last event, to keep elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    last: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// The counts below are of events from this time on.
    since: String,
    sign_ins: u64,
    failed_sign_ins: u64,
    backups: u64,
    restores: u64,
    shares: u64,
    failed_replications: u64,
}

/// The web server's users and audit log; a broken audit log is a problem of
/// `report`.
fn web_facts(
    files: &WebFiles,
    since: jiff::Timestamp,
    report: &mut apeiron::ComplianceReport,
) -> Result<WebFacts, Failure> {
    let users = apeiron_server::Users::load(&files.users)?;
    let (whole, first, events, last, error) =
        match files.audit.verify() {
            Ok(v) => {
                if v.first > 1 {
                    report.findings.push(apeiron::Finding::new(
                        apeiron::FindingKind::AuditTrimmed { first: v.first },
                    ));
                }
                (true, v.first, v.events, Some(v.last), None)
            }
            Err(e) => {
                report.findings.insert(
                    0,
                    apeiron::Finding::new(apeiron::FindingKind::AuditBroken {
                        message: e.message.clone(),
                    }),
                );
                (false, 0, 0, None, Some(e.message))
            }
        };
    let recent: Vec<apeiron_server::Event> = files
        .audit
        .read(usize::MAX, None)?
        .into_iter()
        .filter(|e| e.time.parse::<jiff::Timestamp>().is_ok_and(|t| t >= since))
        .collect();
    let count = |event: &str, ok: Option<bool>| {
        recent
            .iter()
            .filter(|e| e.event == event && ok.is_none_or(|ok| (e.outcome == "ok") == ok))
            .count() as u64
    };
    Ok(WebFacts {
        users: users
            .users
            .iter()
            .map(|u| UserView {
                name: u.name.clone(),
                role: u.role,
                backups: u.backups.clone(),
                folders: u.folders.clone(),
                oidc: u.oidc.clone(),
                second_factor: u.totp.is_some(),
            })
            .collect(),
        identity_provider: users.oidc.map(|o| o.issuer),
        audit: AuditFacts {
            file: files.audit.file().to_path_buf(),
            whole,
            first,
            events,
            last,
            error,
            since: since.round(jiff::Unit::Second).unwrap_or(since).to_string(),
            sign_ins: count("sign_in", Some(true)),
            failed_sign_ins: count("sign_in", Some(false)),
            backups: count("start:backup", Some(true)),
            restores: count("start:restore", Some(true)),
            shares: count("share_add", Some(true)),
            failed_replications: count("replicate", Some(false)),
        },
    })
}

/// Prints a report as Markdown, ready to keep as a document.
/// A compliance report of `repo`, with the web server's part from `since`
/// on when it is given; `tenant` names whose web files are read.
fn make_report(
    out: &Output,
    repo: &Repository,
    request: apeiron::ReportRequest,
    since: Option<jiff::Timestamp>,
    tenant: Option<String>,
    standard: Option<standard::Standard>,
) -> Result<FullReport, Failure> {
    let mut report = watch(out, repo.report(request)?)?;
    let web = match since {
        Some(since) => Some(web_facts(
            &WebFiles::of(tenant.as_deref())?,
            since,
            &mut report,
        )?),
        None => None,
    };
    let mut full = FullReport {
        tenant,
        report,
        web,
        standard: None,
    };
    full.standard = standard.map(|s| standard::map(s, &full));
    Ok(full)
}

fn print_report(full: &FullReport) {
    let r = &full.report;
    let yes = |b: bool| if b { "yes" } else { "no" };
    say!("# Compliance report");
    say!("");
    let of = full
        .tenant
        .as_deref()
        .map(|t| format!(" of the tenant `{t}`"))
        .unwrap_or_default();
    say!(
        "Made {} for the repository `{}`{of}.",
        format::time(&r.time),
        r.repository.path.display()
    );
    say!("");
    say!("## Findings");
    say!("");
    for f in &r.findings {
        let level = match f.level {
            apeiron::Level::Problem => "Problem",
            apeiron::Level::Warning => "Warning",
        };
        say!("- **{level}:** {}", f.text);
    }
    if r.findings.is_empty() {
        say!("None.");
    }
    say!("");
    say!("## Repository");
    say!("");
    say!("| Item | Value |");
    say!("| --- | --- |");
    say!("| Format version | {} |", r.repository.format_version);
    say!("| Encrypted | {} |", yes(r.repository.encrypted));
    if r.repository.encrypted {
        say!("| Passwords | {} |", r.repository.passwords);
        say!("| Recovery keys | {} |", r.repository.recovery_keys);
    }
    match &r.verify {
        Some(v) => say!(
            "| Verified | {} objects checked, {} damaged files, {} damaged objects |",
            v.objects_checked,
            v.damaged_files.len(),
            v.damaged_objects.len()
        ),
        None => say!("| Verified | not in this report; add `--verify` |"),
    }
    say!("");
    say!("## Backups");
    say!("");
    say!("| Ref | Snapshot | Newest | Snapshots | Kept for | Copies |");
    say!("| --- | --- | --- | --- | --- | --- |");
    for f in &r.refs {
        let kept = f
            .retention_days
            .map_or_else(|| "—".to_owned(), |d| format::count(u64::from(d), "day"));
        let copies: Vec<String> = f
            .copies
            .iter()
            .map(|c| {
                let state = serde_json::to_value(c.state)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                format!("{}: {state}", c.remote)
            })
            .collect();
        say!(
            "| {}{} | `{}` | {} | {} | {} | {} |",
            f.name,
            if f.stale { " (stale)" } else { "" },
            f.snapshot,
            format::time(&f.time),
            f.snapshots,
            kept,
            if copies.is_empty() {
                "—".to_owned()
            } else {
                copies.join(", ")
            }
        );
    }
    say!("");
    say!("A snapshot id fixes every byte under it and all the history before it.");
    say!("");
    say!("## Retention rules");
    say!("");
    for rule in &r.retention {
        say!(
            "- `{}`: {}, added {}",
            rule.refs,
            format::count(u64::from(rule.days), "day"),
            format::time(&apeiron::Time {
                seconds: rule.added as i64,
                nanos: 0
            })
        );
    }
    if r.retention.is_empty() {
        say!("None.");
    }
    say!("");
    say!("## Legal holds");
    say!("");
    let when = |seconds: u64| {
        format::time(&apeiron::Time {
            seconds: seconds as i64,
            nanos: 0,
        })
    };
    for h in &r.holds {
        say!(
            "- `{}` ({}): placed {}: {}",
            h.hold.refs,
            h.id,
            when(h.hold.added),
            h.hold.reason
        );
        if let Some(lift) = &h.lift {
            say!("  lifted {}: {}", when(lift.lifted), lift.reason);
        }
    }
    if r.holds.is_empty() {
        say!("None.");
    }
    say!("");
    say!("## Remotes");
    say!("");
    for remote in &r.remotes {
        match &remote.error {
            Some(e) => say!(
                "- {}: `{}`, did not answer: {}",
                remote.name,
                remote.url,
                e.message
            ),
            None => say!("- {}: `{}`", remote.name, remote.url),
        }
    }
    if r.remotes.is_empty() {
        say!("None.");
    }
    if let Some(web) = &full.web {
        print_web(web);
    }
    if let Some(mapping) = &full.standard {
        standard::print(mapping);
    }
}

fn print_web(web: &WebFacts) {
    let yes = |b: bool| if b { "yes" } else { "no" };
    say!("");
    say!("## Web server");
    say!("");
    if let Some(issuer) = &web.identity_provider {
        say!("Identity provider: {issuer}");
        say!("");
    }
    say!("| User | Role | Signs in with | Backups | Folders |");
    say!("| --- | --- | --- | --- | --- |");
    for u in &web.users {
        say!(
            "| {} | {} | {} | {} | {} |",
            u.name,
            u.role.name(),
            if u.oidc.is_some() {
                "identity provider"
            } else {
                "password"
            },
            if u.role == apeiron_server::Role::Admin {
                "all".to_owned()
            } else {
                u.backups.join(", ")
            },
            u.folders
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let a = &web.audit;
    say!("");
    say!("Audit log `{}`:", a.file.display());
    say!("");
    say!("| Item | Value |");
    say!("| --- | --- |");
    say!("| Chain of hashes whole | {} |", yes(a.whole));
    say!("| Events | {} |", a.events);
    if a.first > 1 {
        say!("| First event kept | {} |", a.first);
    }
    if let Some(last) = &a.last {
        say!("| Hash of the last event | `{last}` |");
    }
    say!("| Counted from | {} |", a.since);
    say!("| Sign-ins | {} |", a.sign_ins);
    say!("| Failed sign-ins | {} |", a.failed_sign_ins);
    say!("| Backups started | {} |", a.backups);
    say!("| Restores started | {} |", a.restores);
    say!("| Share links made | {} |", a.shares);
    say!("| Failed replications | {} |", a.failed_replications);
}

/// Asks twice for a new sign-in password and hashes it.
fn new_login_password() -> Result<String, Failure> {
    let password = prompt("New sign-in password: ")?;
    if prompt("Again: ")? != password {
        return Err(Failure::Usage("the two passwords differ".to_owned()));
    }
    Ok(apeiron_server::hash_password(&password)?)
}

fn web_user(files: &WebFiles, command: UserCommand, out: &Output) -> Result<(), Failure> {
    let file = &files.users;
    // The pages may change the users at the same time (docs/USERS.md §4).
    let _lock = apeiron_server::Users::lock(file)?;
    let mut users = apeiron_server::Users::load(file)?;
    let audit = &files.audit;
    // Changes are recorded, never passwords (docs/AUDIT.md §1).
    let record = |event: &str, user: Option<&apeiron_server::User>, name: &str| {
        let detail = match user {
            Some(u) => serde_json::json!({
                "name": u.name, "role": u.role, "backups": u.backups, "folders": u.folders,
                "oidc": u.oidc,
            }),
            None => serde_json::json!({ "name": name }),
        };
        audit.record(&apeiron_server::Actor::default(), event, detail, "ok")
    };
    match command {
        UserCommand::List => {
            let list: Vec<UserView> = users
                .users
                .into_iter()
                .map(|u| UserView {
                    name: u.name,
                    role: u.role,
                    backups: u.backups,
                    folders: u.folders,
                    oidc: u.oidc,
                    second_factor: u.totp.is_some(),
                })
                .collect();
            out.result(&list, |list| {
                for u in list {
                    let folders: Vec<String> =
                        u.folders.iter().map(|f| f.display().to_string()).collect();
                    say!(
                        "{}  {}  backups: {}  folders: {}{}",
                        u.name,
                        u.role.name(),
                        if u.role == apeiron_server::Role::Admin {
                            "all".to_owned()
                        } else {
                            u.backups.join(" ")
                        },
                        folders.join(" "),
                        u.oidc
                            .as_deref()
                            .map(|i| format!("  identity: {i}"))
                            .unwrap_or_default()
                    );
                    if u.second_factor {
                        say!("  signs in with a password and an authenticator code");
                    }
                }
                if list.is_empty() {
                    say!("No users; add one with `apeiron web user add <name> --role admin`");
                }
            });
            return Ok(());
        }
        UserCommand::Add {
            name,
            role,
            backups,
            folders,
            oidc,
        } => {
            if users.get(&name).is_some() {
                return Err(Error::new(
                    ErrorKind::AlreadyExists,
                    format!("a user named {name} already exists"),
                )
                .into());
            }
            let password = match oidc {
                Some(_) => String::new(),
                None => new_login_password()?,
            };
            users.add(apeiron_server::User {
                name: name.clone(),
                password,
                role: role.into(),
                backups,
                folders,
                oidc,
                totp: None,
                from_groups: false,
            })?;
            users.need_admin()?;
            users.save(file)?;
            record("user_add", users.get(&name), &name)?;
            say!("Added {name}");
        }
        UserCommand::Change {
            name,
            role,
            backups,
            folders,
            oidc,
            no_password,
        } => {
            let user = users.get_mut(&name)?;
            if oidc.is_some() {
                user.oidc = oidc;
            }
            if no_password {
                if user.oidc.is_none() {
                    return Err(Failure::Usage(format!(
                        "{name} would have no way to sign in; give it an identity with --oidc"
                    )));
                }
                user.password.clear();
            }
            if let Some(role) = role {
                user.role = role.into();
            }
            if !backups.is_empty() {
                user.backups = backups;
            }
            if !folders.is_empty() {
                user.folders = folders;
            }
            users.need_admin()?;
            users.save(file)?;
            record("user_change", users.get(&name), &name)?;
            say!("Changed {name}");
        }
        UserCommand::Password { name } => {
            users.get_mut(&name)?;
            let password = new_login_password()?;
            users.get_mut(&name)?.password = password;
            users.save(file)?;
            record("user_password", None, &name)?;
            say!("Set a new password for {name}");
        }
        UserCommand::SecondFactor { name, off } => {
            let user = users.get_mut(&name)?;
            if off {
                user.totp = None;
                users.save(file)?;
                record("user_second_factor_off", None, &name)?;
                say!("{name} signs in with the password alone again");
            } else {
                if user.password.is_empty() {
                    return Err(Failure::Usage(format!(
                        "{name} has no password; it signs in through the identity provider"
                    )));
                }
                let secret = apeiron_server::new_totp_secret()?;
                user.totp = Some(secret.clone());
                users.save(file)?;
                record("user_second_factor_on", None, &name)?;
                say!("Enter this secret in {name}'s authenticator app: {secret}");
                say!(
                    "or open this address on the phone: {}",
                    apeiron_server::totp_uri(&secret, &name, "Apeiron")
                );
                say!("From now on {name} signs in with the password and the app's code.");
            }
        }
        UserCommand::Remove { name } => {
            users.remove(&name)?;
            users.save(file)?;
            record("user_remove", None, &name)?;
            say!("Removed {name}");
        }
    }
    Ok(())
}

#[derive(Subcommand)]
enum PluginCommand {
    /// Check a plugin (a WebAssembly component) and install it
    Add {
        #[arg(required_unless_present = "from_repo")]
        file: Option<PathBuf>,
        /// Install the plugin of this name that the repository carries
        #[arg(long, value_name = "NAME", conflicts_with = "file")]
        from_repo: Option<String>,
    },
    /// List the installed plugins
    List {
        /// List the plugins the repository carries instead
        #[arg(long)]
        shared: bool,
    },
    /// Remove an installed plugin
    Remove { name: String },
    /// Put an installed plugin into the repository, so its copies carry it
    Share { name: String },
    /// Take a plugin out of the repository; installed copies stay
    Unshare { name: String },
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

/// Stack of the thread the command runs on. Windows gives the main thread
/// 1 MiB, which the many commands outgrow in a debug build.
const STACK: usize = 16 << 20;

fn main() -> ExitCode {
    let worker = std::thread::Builder::new()
        .name("main".to_owned())
        .stack_size(STACK)
        .spawn(command_main);
    match worker {
        Ok(handle) => handle
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        Err(e) => {
            eprintln!("error: cannot start: {e}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn command_main() -> ExitCode {
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
    unlock(&path, password_file, "Password or recovery key: ")
}

/// The audit log a backup or restore of the CLI is recorded in (AUDIT.md
/// §1): that of the tenant whose repository it is, or else the web
/// server's, if there is one on this computer.
struct CliAudit {
    log: Option<(apeiron_server::Audit, PathBuf)>,
    actor: apeiron_server::Actor,
}

impl CliAudit {
    fn find(repo: Option<&Path>) -> Result<CliAudit, Failure> {
        let actor = apeiron_server::Actor {
            user: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok(),
            from: Some("cli".to_owned()),
        };
        let Some(repo) = repo.and_then(|r| std::fs::canonicalize(r).ok()) else {
            return Ok(CliAudit { log: None, actor });
        };
        let same = |path: &Path| std::fs::canonicalize(path).is_ok_and(|p| p == repo);
        let tenants = match tenants_file() {
            Ok(file) if file.exists() => apeiron_server::Tenants::load(&file)?.tenants,
            _ => Vec::new(),
        };
        let audit = match tenants.iter().find(|t| same(&t.repo)) {
            Some(t) => Some(WebFiles::of_tenant(t)?.audit),
            None => apeiron_server::Audit::default_file().map(apeiron_server::Audit::new),
        };
        let log = audit.filter(|a| a.file().exists()).map(|a| (a, repo));
        Ok(CliAudit { log, actor })
    }

    /// Records `start:<operation>` before it runs; if that fails, it does
    /// not run.
    fn start(&self, operation: &str, detail: &serde_json::Value) -> Result<(), Failure> {
        let Some((audit, repo)) = &self.log else {
            return Ok(());
        };
        let mut detail = detail.clone();
        detail["repo"] = serde_json::json!(repo);
        detail["via"] = serde_json::json!("cli");
        audit.record(&self.actor, &format!("start:{operation}"), detail, "ok")?;
        Ok(())
    }

    /// Records `done:<operation>` with what `result` says, then returns it.
    fn done<T>(
        &self,
        operation: &str,
        result: Result<T, Failure>,
        detail: impl Fn(&T) -> serde_json::Value,
    ) -> Result<T, Failure> {
        let Some((audit, repo)) = &self.log else {
            return result;
        };
        let (mut detail, outcome) = match &result {
            Ok(value) => (detail(value), "ok".to_owned()),
            Err(Failure::Api(e)) => (
                serde_json::json!({}),
                serde_json::to_value(e.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "error".to_owned()),
            ),
            Err(_) => (serde_json::json!({}), "internal".to_owned()),
        };
        detail["repo"] = serde_json::json!(repo);
        detail["via"] = serde_json::json!("cli");
        audit.record(&self.actor, &format!("done:{operation}"), detail, &outcome)?;
        result
    }
}

/// Opens the repository at `path`, asking with `asking` for the password of
/// an encrypted one unless `password_file` holds it.
fn unlock(path: &Path, password_file: Option<&Path>, asking: &str) -> Result<Repository, Failure> {
    let repo = match Repository::open(path) {
        Err(e) if e.kind == ErrorKind::Key => {
            let secret = match password_file {
                Some(file) => read_password_file(file)?,
                None => prompt(asking)?,
            };
            Repository::unlock(path, &secret)?
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
            let audit = CliAudit::find(cli.repo.as_deref())?;
            let repo = open(cli.repo, password_file)?;
            let mut request = BackupRequest::new(folder, ref_name.clone());
            request.message = message;
            request.author = author;
            request.host = host;
            let detail = serde_json::json!({
                "source": request.source,
                "ref": request.ref_name,
            });
            audit.start("backup", &detail)?;
            let report = audit.done(
                "backup",
                repo.backup(request)
                    .map_err(Failure::from)
                    .and_then(|job| watch(out, job)),
                |r| serde_json::json!({ "snapshot": r.snapshot }),
            )?;
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
        Command::Log {
            snapshot,
            limit,
            grep,
        } => {
            let repo = open(cli.repo, password_file)?;
            let more = grep.as_ref().map(|g| format!(" --grep {g:?}"));
            let page = repo.log(&LogRequest {
                start: snapshot,
                limit,
                grep,
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
                    say!(
                        "… more: apeiron log {}{}",
                        format::short_id(&next),
                        more.as_deref().unwrap_or("")
                    );
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
        Command::Diff {
            from,
            to,
            text: true,
        } => {
            let repo = open(cli.repo, password_file)?;
            let diffs = repo.diff_text(&DiffRequest { from, to })?;
            out.result(&diffs, |diffs| {
                for d in diffs {
                    say!("{} {}", format::change(d.kind), d.path);
                    if let Some(text) = &d.text {
                        for line in text.lines() {
                            say!("{line}");
                        }
                    }
                    if let Some(error) = &d.error {
                        say!("(no difference shown: {error})");
                    }
                }
            });
            Ok(0)
        }
        Command::Diff { from, to, .. } => {
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
            let audit = CliAudit::find(cli.repo.as_deref())?;
            let repo = open(cli.repo, password_file)?;
            let request = RestoreRequest {
                snapshot,
                target: target.clone(),
                path: Name::from(path.as_str()),
            };
            audit.start(
                "restore",
                &serde_json::json!({
                    "snapshot": request.snapshot,
                    "target": request.target,
                    "path": request.path,
                }),
            )?;
            let report = audit.done(
                "restore",
                repo.restore(request)
                    .map_err(Failure::from)
                    .and_then(|job| watch(out, job)),
                |r| serde_json::json!({ "files": r.files, "bytes": r.bytes }),
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
        Command::Gc {
            keep_days,
            dry_run,
            remote,
        } => {
            let repo = open(cli.repo, password_file)?;
            let job = match remote {
                Some(remote) => repo.gc_remote(apeiron::RemoteGcRequest { remote, dry_run })?,
                None => repo.gc(GcRequest {
                    keep_reflog_days: keep_days,
                    dry_run,
                })?,
            };
            let report = watch(out, job)?;
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
        Command::Report {
            verify,
            remotes,
            stale_days,
            web,
            tenant,
            all_tenants,
            since,
            standard,
            command,
            out: file,
            sign,
        } => {
            if let Some(command) = command {
                return report_command(out, command);
            }
            // Written to a file, the report is caught instead of printed.
            if file.is_some() {
                CAUGHT.with(|c| *c.borrow_mut() = Some(Vec::new()));
            }
            let code = (|| -> Result<u8, Failure> {
                let request = apeiron::ReportRequest {
                    verify,
                    remotes,
                    stale_days,
                };
                let since = web.then(|| {
                    since.map_or_else(
                        || jiff::Timestamp::now() - jiff::SignedDuration::from_hours(30 * 24),
                        |t| jiff::Timestamp::from_second(t.seconds).unwrap_or_default(),
                    )
                });
                let exit = |full: &FullReport| {
                    if full.report.verify.as_ref().is_some_and(|v| !v.is_ok()) {
                        EXIT_DAMAGED
                    } else if full.report.has_problems() {
                        EXIT_WARNINGS
                    } else {
                        0
                    }
                };
                if !all_tenants {
                    let repo = open(cli.repo, password_file)?;
                    let full = make_report(out, &repo, request, since, tenant, standard)?;
                    out.result(&full, print_report);
                    return Ok(exit(&full));
                }
                // One report per tenant (docs/REPORTS.md §2).
                let tenants = apeiron_server::Tenants::load(&tenants_file()?)?;
                if tenants.tenants.is_empty() {
                    return Err(Failure::Usage(
                        "there are no tenants; see docs/TENANTS.md".to_owned(),
                    ));
                }
                let mut reports = Vec::new();
                for t in &tenants.tenants {
                    let repo = open(Some(t.repo.clone()), t.password_file.as_deref())?;
                    let full = make_report(
                        out,
                        &repo,
                        request.clone(),
                        since,
                        Some(t.name.clone()),
                        standard,
                    )?;
                    reports.push(full);
                }
                out.result(&reports, |reports| {
                    for (i, full) in reports.iter().enumerate() {
                        if i > 0 {
                            say!("");
                            say!("---");
                            say!("");
                        }
                        print_report(full);
                    }
                });
                Ok(reports.iter().map(exit).max().unwrap_or(0))
            })();
            let Some(file) = file else { return code };
            let caught = CAUGHT.with(|c| c.borrow_mut().take()).unwrap_or_default();
            let code = code?;
            std::fs::write(&file, &caught)
                .map_err(|e| Failure::Usage(format!("cannot write {}: {e}", file.display())))?;
            match sign {
                Some(key) => {
                    let signature = sign::sign(&key, &caught)?;
                    let sig = sign::signature_file(&file);
                    let text = serde_json::to_vec_pretty(&signature).unwrap_or_default();
                    std::fs::write(&sig, text).map_err(|e| {
                        Failure::Usage(format!("cannot write {}: {e}", sig.display()))
                    })?;
                    eprintln!(
                        "Wrote {} and its signature {}, by the key {}",
                        file.display(),
                        sig.display(),
                        signature.public_key
                    );
                }
                None => eprintln!("Wrote {}", file.display()),
            }
            Ok(code)
        }
        Command::Retention { command } => {
            let repo = open(cli.repo, password_file)?;
            match command {
                RetentionCommand::Add { refs, days } => {
                    let rule = repo.add_retention(&refs, days)?;
                    out.result(&rule, |r| {
                        say!(
                            "{} is now kept {}; this cannot be undone",
                            r.refs,
                            format::count(u64::from(r.days), "day")
                        )
                    });
                }
                RetentionCommand::List => {
                    let rules = repo.retention()?;
                    out.result(&rules, |rules| {
                        for r in rules {
                            say!(
                                "{}  {}  added {}",
                                r.refs,
                                format::count(u64::from(r.days), "day"),
                                format::time(&apeiron::Time {
                                    seconds: r.added as i64,
                                    nanos: 0
                                })
                            );
                        }
                        if rules.is_empty() {
                            say!("No retention rules; see docs/ARCHIVE.md");
                        }
                    });
                }
            }
            Ok(0)
        }
        Command::Tag { files, add, remove } => {
            let mut shown = Vec::new();
            for file in files {
                let mut tags = apeiron::file_tags(&file)?;
                if !add.is_empty() || !remove.is_empty() {
                    tags.retain(|t| !remove.iter().any(|r| r.trim().eq_ignore_ascii_case(t)));
                    for tag in &add {
                        if !tags.iter().any(|t| t.eq_ignore_ascii_case(tag.trim())) {
                            tags.push(tag.trim().to_owned());
                        }
                    }
                    apeiron::set_file_tags(&file, &tags)?;
                }
                shown.push(serde_json::json!({ "file": file, "tags": tags }));
            }
            out.result(&shown, |shown| {
                for s in shown {
                    let tags: Vec<&str> = s["tags"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|t| t.as_str())
                        .collect();
                    say!(
                        "{}  [{}]",
                        s["file"].as_str().unwrap_or_default(),
                        tags.join(", ")
                    );
                }
            });
            Ok(0)
        }
        Command::Hold { command } => {
            let repo = open(cli.repo, password_file)?;
            let when = |seconds: u64| {
                format::time(&apeiron::Time {
                    seconds: seconds as i64,
                    nanos: 0,
                })
            };
            match command {
                HoldCommand::Add { refs, reason } => {
                    let hold = repo.add_legal_hold(&refs, &reason)?;
                    out.result(&hold, |h| {
                        say!("{} is held as {}: {}", h.hold.refs, h.id, h.hold.reason)
                    });
                }
                HoldCommand::Lift { id, reason } => {
                    let hold = repo.lift_legal_hold(&id, &reason)?;
                    out.result(&hold, |h| {
                        say!(
                            "The hold {} on {} is lifted; this cannot be undone",
                            h.id,
                            h.hold.refs
                        )
                    });
                }
                HoldCommand::List => {
                    let holds = repo.legal_holds()?;
                    out.result(&holds, |holds| {
                        for h in holds {
                            say!(
                                "{}  {}  placed {}: {}",
                                h.id,
                                h.hold.refs,
                                when(h.hold.added),
                                h.hold.reason
                            );
                            if let Some(lift) = &h.lift {
                                say!("    lifted {}: {}", when(lift.lifted), lift.reason);
                            }
                        }
                        if holds.is_empty() {
                            say!("No legal holds; see docs/ARCHIVE.md");
                        }
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
        Command::Search {
            pattern,
            text,
            larger,
            smaller,
            after,
            before,
            tags,
            snapshot,
            history,
            limit,
            saved,
            save,
            list_saved,
            forget,
        } => {
            if list_saved {
                let searches = apeiron::saved_searches(None)?;
                out.result(&searches, |searches| {
                    for s in searches {
                        say!("{}  {}", s.name, format::search(&s.request));
                    }
                    if searches.is_empty() {
                        say!("No saved searches");
                    }
                });
                return Ok(0);
            }
            if let Some(name) = forget {
                let gone = apeiron::forget_search(&name, None)?;
                out.result(&gone, |s| say!("Removed the saved search {}", s.name));
                return Ok(0);
            }
            let mut request = match &saved {
                Some(name) => apeiron::saved_search(name, None)?.request,
                None => SearchRequest {
                    snapshot: None,
                    history: false,
                    query: Query::default(),
                    limit: 1000,
                },
            };
            let q = &mut request.query;
            q.pattern = pattern.or(q.pattern.take());
            q.text = text.or(q.text.take());
            q.larger = larger.or(q.larger);
            q.smaller = smaller.or(q.smaller);
            q.after = after.or(q.after);
            q.before = before.or(q.before);
            if !tags.is_empty() {
                q.tags = tags;
            }
            request.snapshot = snapshot.or(request.snapshot);
            request.history |= history;
            request.limit = limit.unwrap_or(request.limit);
            if let Some(name) = save {
                apeiron::save_search(
                    &apeiron::SavedSearch {
                        name,
                        request: request.clone(),
                    },
                    None,
                )?;
            }
            let history = request.history;
            let repo = open(cli.repo, password_file)?;
            let report = watch(out, repo.search(request)?)?;
            out.result(&report, |r| {
                for hit in &r.hits {
                    let mut line = hit.path.to_string();
                    if !hit.entry.tags.is_empty() {
                        line.push_str(&format!("  [{}]", hit.entry.tags.join(", ")));
                    }
                    if let Some(text) = &hit.line {
                        line.push_str(&format!(":{}: {}", text.number, text.text));
                    }
                    if history {
                        say!(
                            "{}  {}  {line}",
                            format::short_id(&hit.snapshot),
                            format::time(&hit.time)
                        );
                    } else {
                        say!("{line}");
                    }
                }
                if r.hits.is_empty() {
                    say!("No matches in {}", format::count(r.snapshots, "snapshot"));
                }
                if r.truncated {
                    say!("… more matches; show them with --limit");
                }
                if r.skipped > 0 {
                    say!(
                        "{} over 16 MiB not searched for text",
                        format::count(r.skipped, "file")
                    );
                }
            });
            Ok(0)
        }
        Command::Duplicates { snapshot, smallest } => {
            let repo = open(cli.repo, password_file)?;
            let groups = repo.duplicates(&DuplicatesRequest { snapshot, smallest })?;
            out.result(&groups, |groups| {
                let mut extra = 0u64;
                for d in groups {
                    let copies = d.paths.len() as u64;
                    extra = extra.saturating_add(d.size.saturating_mul(copies - 1));
                    say!("{} copies of {}", copies, format::bytes(d.size));
                    for path in &d.paths {
                        say!("  {path}");
                    }
                }
                if groups.is_empty() {
                    say!("No files with the same content");
                } else {
                    say!(
                        "{} of copies; the extra copies take {}",
                        format::count(groups.len() as u64, "group"),
                        format::bytes(extra)
                    );
                }
            });
            Ok(0)
        }
        Command::Merge {
            from,
            into,
            message,
            dry_run,
        } => merge_command(
            out,
            cli.repo,
            password_file,
            Joining::Merge,
            from,
            into,
            message,
            dry_run,
        ),
        Command::CherryPick {
            snapshot,
            into,
            message,
            dry_run,
        } => merge_command(
            out,
            cli.repo,
            password_file,
            Joining::CherryPick,
            snapshot,
            into,
            message,
            dry_run,
        ),
        Command::Rebase {
            branch,
            onto,
            dry_run,
        } => merge_command(
            out,
            cli.repo,
            password_file,
            Joining::Rebase,
            onto,
            branch,
            None,
            dry_run,
        ),
        Command::Ui { no_browser } => {
            let mut options = apeiron_server::Options::with_memory();
            if cli.repo.is_some() {
                options.repo = Some(open(cli.repo, password_file)?);
            }
            let server = apeiron_server::Local::start(options)
                .map_err(|e| Failure::Api(Error::new(ErrorKind::Io, e.to_string())))?;
            let opened = !no_browser && open_browser(server.url()).is_ok();
            if opened {
                say!("The interface is open in your browser. Press Ctrl+C to stop it.");
            } else {
                say!("Open this address in a browser; press Ctrl+C to stop:");
                say!("{}", server.url());
            }
            server.wait();
            Ok(0)
        }
        Command::Web {
            listen,
            cert,
            key,
            plain,
            folders,
            tenants,
            replicas,
            standby,
            audit_syslog,
            tenant,
            command,
        } => {
            let config = dirs::config_dir()
                .map(|d| d.join("apeiron"))
                .ok_or_else(|| Failure::Usage("no configuration folder".to_owned()))?;
            let files = || WebFiles::of(tenant.as_deref());
            match command {
                Some(WebCommand::User { command }) => {
                    web_user(&files()?, command, out)?;
                    return Ok(0);
                }
                Some(WebCommand::Audit { command }) => return web_audit(&files()?, command, out),
                Some(WebCommand::Oidc { command }) => {
                    web_oidc(&files()?, command, out)?;
                    return Ok(0);
                }
                Some(WebCommand::Share { command }) => {
                    web_share(&files()?, command, out)?;
                    return Ok(0);
                }
                Some(WebCommand::Tenant { command }) => {
                    web_tenant(command, out)?;
                    return Ok(0);
                }
                None => {}
            }
            if tenant.is_some() {
                return Err(Failure::Usage(
                    "--tenant names whose users, audit log, links or provider to manage; \
                     to serve every tenant use --tenants"
                        .to_owned(),
                ));
            }
            let listed = if tenants {
                apeiron_server::Tenants::load(&tenants_file()?)?.tenants
            } else {
                Vec::new()
            };
            let tls = match (cert, key) {
                _ if plain => None,
                (Some(cert), Some(key)) => {
                    let read = |path: &Path| {
                        std::fs::read(path).map_err(|e| {
                            Failure::Usage(format!("cannot read {}: {e}", path.display()))
                        })
                    };
                    Some(apeiron_server::Tls {
                        cert_pem: read(&cert)?,
                        key_pem: read(&key)?,
                    })
                }
                _ => {
                    let mut names = vec!["localhost".to_owned()];
                    if !listen.ip().is_unspecified() {
                        names.push(listen.ip().to_string());
                    }
                    names.extend(listed.iter().map(|t| t.host.clone()));
                    let dir = if tenants {
                        config.join("tenants")
                    } else {
                        config.clone()
                    };
                    Some(apeiron_server::Tls::kept_in(&dir, names)?)
                }
            };
            let fingerprint = tls
                .as_ref()
                .map(apeiron_server::Tls::fingerprint)
                .transpose()?;
            let server = if tenants {
                let mut options = Vec::new();
                let roots = web_roots()?;
                for t in listed {
                    let files = WebFiles::of_tenant(&t)?;
                    let asking = format!("Password or recovery key of {}: ", t.name);
                    let audit = copy_audit(files.audit, &audit_syslog, Some(&t.name));
                    let jobs = Some(t.jobs(&roots.0, &roots.1));
                    let limits = apeiron_server::Limits {
                        max_bytes: t.max_bytes,
                        max_jobs: t.max_jobs,
                    };
                    options.push(apeiron_server::TenantOptions {
                        repo: unlock(&t.repo, t.password_file.as_deref(), &asking)?,
                        name: t.name,
                        host: t.host,
                        users: files.users,
                        folders: t.folders,
                        audit,
                        shares: files.shares,
                        replicas: t.replicas,
                        standby,
                        limits,
                        operator: t.operator,
                        jobs,
                    });
                }
                let (config, state) = web_roots()?;
                let manage = apeiron_server::Manage {
                    file: tenants_file()?,
                    config,
                    state,
                    standby,
                    syslog: audit_syslog.clone(),
                };
                apeiron_server::Web::start_tenants(listen, tls, options, Some(manage))?
            } else {
                let files = files()?;
                apeiron_server::Users::load(&files.users)?.need_admin()?;
                let repo = open(cli.repo, password_file)?;
                let folders = if folders.is_empty() {
                    vec![
                        dirs::home_dir()
                            .ok_or_else(|| Failure::Usage("no home folder".to_owned()))?,
                    ]
                } else {
                    folders
                };
                apeiron_server::Web::start(apeiron_server::WebOptions {
                    repo,
                    listen,
                    tls,
                    users: files.users,
                    folders,
                    audit: copy_audit(files.audit, &audit_syslog, None),
                    shares: files.shares,
                    replicas,
                    standby,
                })?
            };
            let scheme = if plain { "http" } else { "https" };
            say!(
                "Serving on {scheme}://{}; press Ctrl+C to stop.",
                server.address()
            );
            if let Some(fingerprint) = fingerprint {
                say!("Certificate fingerprint (SHA-256): {fingerprint}");
            }
            if plain {
                say!("Plain HTTP: use this only behind a proxy that adds TLS.");
            }
            if standby {
                say!("Standby: backups are refused until the server is started without --standby.");
            }
            server.wait();
            Ok(0)
        }
        Command::Plugin { command } => {
            match command {
                PluginCommand::Add { file, from_repo } => {
                    let added = match (file, from_repo) {
                        (_, Some(name)) => {
                            open(cli.repo, password_file)?.install_shared_plugin(&name, None)?
                        }
                        (Some(file), None) => apeiron::add_plugin(&file, None)?,
                        (None, None) => {
                            return Err(Failure::Usage("give a file or --from-repo".to_owned()));
                        }
                    };
                    out.result(&added, |a| {
                        let i = &a.info;
                        let verb = if a.replaced { "Replaced" } else { "Installed" };
                        say!("{verb} {} {}: {}", i.name, i.version, i.description);
                        say!("It merges files named {}", i.patterns.join(", "));
                    });
                }
                PluginCommand::List { shared } => {
                    let list = if shared {
                        open(cli.repo, password_file)?.shared_plugins()?
                    } else {
                        apeiron::plugins(None)?
                    };
                    out.result(&list, |list| {
                        for i in list {
                            say!(
                                "{} {}  {}  {}",
                                i.name,
                                i.version,
                                i.patterns.join(" "),
                                i.description
                            );
                        }
                        if list.is_empty() {
                            say!("No plugins; see docs/PLUGIN.md");
                        }
                    });
                }
                PluginCommand::Remove { name } => {
                    let removed = apeiron::remove_plugin(&name, None)?;
                    out.result(&removed, |i| say!("Removed {} {}", i.name, i.version));
                }
                PluginCommand::Share { name } => {
                    let shared = open(cli.repo, password_file)?.share_plugin(&name, None)?;
                    out.result(&shared, |i| {
                        say!(
                            "The repository now carries {} {}; on another copy, `apeiron plugin add --from-repo {}` installs it",
                            i.name,
                            i.version,
                            i.name
                        )
                    });
                }
                PluginCommand::Unshare { name } => {
                    open(cli.repo, password_file)?.unshare_plugin(&name)?;
                    out.result(&serde_json::json!({ "name": name }), |_| {
                        say!("The repository no longer carries {name}")
                    });
                }
            }
            Ok(0)
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
            priority,
            tenant,
        } => run_daemon(out, command, jobs, state_dir, priority, tenant),
        Command::Clone { url, path, rate } => {
            let mut request = CloneRequest {
                url,
                path,
                secret: password_file.map(read_password_file).transpose()?,
                rate,
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
        Command::Switch {
            snapshot,
            folder,
            dry_run,
        } => {
            let repo = open(cli.repo, password_file)?;
            let request = apeiron::SwitchRequest {
                snapshot: snapshot.clone(),
                folder,
                dry_run,
            };
            let report = watch(out, repo.switch(request)?)?;
            out.result(&report, |r| print_switch(r, &snapshot, dry_run));
            let attention = !report.conflicts.is_empty() || !report.skipped.is_empty();
            Ok(match warnings_exit(out, &report.warnings) {
                0 if attention => EXIT_WARNINGS,
                code => code,
            })
        }
        Command::Sync {
            folder,
            remote,
            ref_name,
            mode,
            dry_run,
            rate,
        } => {
            let repo = open(cli.repo, password_file)?;
            let mut request = SyncRequest::new(folder, remote.clone());
            request.ref_name = ref_name;
            request.rate = rate;
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
        Command::Push {
            remote,
            all: true,
            rate,
            ..
        } => {
            let repo = open(cli.repo, password_file)?;
            let request = apeiron::RemoteRequest {
                remote: remote.clone(),
                rate,
            };
            let report = watch(out, repo.push_all(request)?)?;
            Ok(mirror_result(out, &report, &remote))
        }
        Command::Pull {
            remote,
            all: true,
            rate,
            ..
        } => {
            let repo = open(cli.repo, password_file)?;
            let request = apeiron::RemoteRequest {
                remote: remote.clone(),
                rate,
            };
            let report = watch(out, repo.pull_all(request)?)?;
            Ok(mirror_result(out, &report, &remote))
        }
        Command::Push {
            remote,
            ref_name,
            rate,
            ..
        } => {
            let repo = open(cli.repo, password_file)?;
            let request = TransferRequest {
                rate,
                ..TransferRequest::new(remote.clone(), ref_name.clone())
            };
            let report = watch(out, repo.push(request)?)?;
            out.result(&report, |r| say!("{}", transferred(r, &ref_name, &remote)));
            Ok(0)
        }
        Command::Pull {
            remote,
            ref_name,
            rate,
            ..
        } => {
            let repo = open(cli.repo, password_file)?;
            let request = TransferRequest {
                rate,
                ..TransferRequest::new(remote.clone(), ref_name.clone())
            };
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
    priority: PriorityArg,
    tenant: Option<String>,
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
    if let Some(name) = tenant {
        let tenants = apeiron_server::Tenants::load(&tenants_file()?)?;
        let (config, state) = web_roots()?;
        paths = tenants.get(&name)?.jobs(&config, &state);
    }
    if let Some(jobs) = jobs {
        paths.jobs = jobs;
    }
    if let Some(state_dir) = state_dir {
        paths.state_dir = state_dir;
    }
    let failed = |result: RunResult| result == RunResult::Failed;
    let runs_jobs = matches!(command, DaemonCommand::Run { .. } | DaemonCommand::Once);
    if runs_jobs && priority == PriorityArg::Low {
        apeiron_daemon::lower_priority().map_err(daemon_error)?;
    }
    match command {
        DaemonCommand::Run { no_notify } => {
            let daemon = Daemon::start(paths)
                .map_err(daemon_error)?
                .notify_failures(!no_notify);
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let _ = ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed));
            daemon.run(&stop).map_err(daemon_error)?;
            Ok(0)
        }
        DaemonCommand::Once => {
            let jobs = apeiron_daemon::load_scoped(&paths.jobs, paths.scope.as_ref())
                .map_err(daemon_error)?;
            let daemon = Daemon::start(paths).map_err(daemon_error)?;
            let runs = daemon
                .run_due(&jobs, &AtomicBool::new(false))
                .map_err(daemon_error)?;
            out.result(&runs, |runs| {
                for r in runs {
                    say!(
                        "{}: {}: {}",
                        r.job,
                        apeiron_daemon::RunResult::name(r.run.result),
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
                                apeiron_daemon::RunResult::name(last.result),
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

/// Opens the interface in the default browser through a page only this
/// user can read, so the token never appears in a process list.
fn open_browser(url: &str) -> std::io::Result<()> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let page = std::env::temp_dir().join(format!("apeiron-ui-{}-{nanos}.html", std::process::id()));
    let mut file = std::fs::OpenOptions::new();
    file.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut file, 0o600);
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=\"0;url={url}\"><title>Apeiron</title>"
    );
    file.open(&page)?.write_all(html.as_bytes())?;
    let launched = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(&page)
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&page).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&page).spawn()
    };
    // The browser reads the page within seconds; then it goes.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(30));
        let _ = std::fs::remove_file(page);
    });
    launched.map(|_| ())
}

/// A size in bytes, or with `K`, `M` or `G` (powers of 1024).
fn parse_size(s: &str) -> std::result::Result<u64, String> {
    let (digits, shift) = match s.char_indices().last() {
        Some((i, 'K' | 'k')) => (&s[..i], 10),
        Some((i, 'M' | 'm')) => (&s[..i], 20),
        Some((i, 'G' | 'g')) => (&s[..i], 30),
        _ => (s, 0),
    };
    digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(1 << shift))
        .ok_or_else(|| format!("{s:?} is not a size such as 4096, 10K, 5M or 2G"))
}

/// The start of a day `YYYY-MM-DD` in local time.
/// Which way `merge_command` joins two lines of history.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Joining {
    Merge,
    CherryPick,
    Rebase,
}

/// Runs `merge`, `cherry-pick` or `rebase`, and prints the result.
#[allow(clippy::too_many_arguments)]
fn merge_command(
    out: &Output,
    repo_path: Option<PathBuf>,
    password_file: Option<&Path>,
    kind: Joining,
    from: String,
    into: String,
    message: Option<String>,
    dry_run: bool,
) -> Result<u8, Failure> {
    let repo = open(repo_path, password_file)?;
    let mut request = MergeRequest::new(from.clone(), into.clone());
    request.message = message;
    request.dry_run = dry_run;
    let report = match kind {
        Joining::Merge => repo.merge(&request)?,
        Joining::CherryPick => repo.cherry_pick(&request)?,
        Joining::Rebase => repo.rebase(&request)?,
    };
    // The conflict copy holds the version of the side brought in.
    let brought = if kind == Joining::Rebase {
        &into
    } else {
        &from
    };
    out.result(&report, |r| {
        for change in &r.changes {
            let mark = match change.kind {
                ChangeKind::Added => '+',
                ChangeKind::Removed => '-',
                ChangeKind::Modified => '~',
            };
            say!("{mark} {}", change.path);
        }
        for m in &r.plugin_merges {
            say!("merged by plugin {}: {}", m.plugin, m.path);
        }
        for c in &r.conflicts {
            match &c.copy {
                Some(copy) => {
                    say!("conflict: {}; the version from {brought} is {copy}", c.path)
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
            MergeAction::Rebased => say!(
                "{into} {would}rebased onto {from} ({}, {})",
                format::count(r.changes.len() as u64, "change"),
                format::count(r.conflicts.len() as u64, "conflict")
            ),
            MergeAction::Picked => say!(
                "the changes of {from} {would}copied into {into} ({}, {})",
                format::count(r.changes.len() as u64, "change"),
                format::count(r.conflicts.len() as u64, "conflict")
            ),
            MergeAction::Merged => say!(
                "{from} {would}merged into {into} ({}, {})",
                format::count(r.changes.len() as u64, "change"),
                format::count(r.conflicts.len() as u64, "conflict")
            ),
        }
    });
    let warned = warnings_exit(out, &report.warnings);
    Ok(if report.conflicts.is_empty() {
        warned
    } else {
        EXIT_WARNINGS
    })
}

/// A speed such as 500K or 2M, in bytes per second; zero is refused.
fn parse_rate(s: &str) -> std::result::Result<u64, String> {
    match parse_size(s)? {
        0 => Err("a speed limit must be more than zero".to_owned()),
        n => Ok(n),
    }
}

fn parse_date(s: &str) -> std::result::Result<Time, String> {
    let start = s
        .parse::<jiff::civil::Date>()
        .and_then(|d| d.to_zoned(jiff::tz::TimeZone::system()))
        .map_err(|_| format!("{s:?} is not a date such as 2026-10-29"))?;
    Ok(Time {
        seconds: start.timestamp().as_second(),
        nanos: 0,
    })
}

fn local_time(t: jiff::Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}

/// One line about a push or pull.
fn print_switch(r: &apeiron::SwitchReport, snapshot: &str, dry_run: bool) {
    let would = if dry_run { "would be " } else { "" };
    if let Some(saved) = &r.saved {
        say!(
            "the folder was recorded first as {} in {}",
            format::short_id(saved),
            r.ref_name
        );
    }
    for change in &r.changes {
        let mark = match change.kind {
            ChangeKind::Added => '+',
            ChangeKind::Removed => '-',
            ChangeKind::Modified => '~',
        };
        say!("{mark} {}", change.path);
    }
    for c in &r.conflicts {
        if let Some(copy) = &c.copy {
            say!(
                "conflict: {} changed during the switch; the version of {snapshot} is {copy}",
                c.path
            );
        }
    }
    for path in &r.skipped {
        say!("left alone, changed during the switch: {path}");
    }
    say!(
        "the folder {would}switched to {snapshot} ({}, {})",
        format::short_id(&r.snapshot),
        format::count(r.changes.len() as u64, "change")
    );
}

fn print_sync(r: &SyncReport, remote: &str, dry_run: bool) {
    for change in &r.changes {
        let mark = match change.kind {
            ChangeKind::Added => '+',
            ChangeKind::Removed => '-',
            ChangeKind::Modified => '~',
        };
        say!("{mark} {}", change.path);
    }
    for m in &r.plugin_merges {
        say!("merged by plugin {}: {}", m.plugin, m.path);
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

/// Prints what pushing or pulling every ref did; exit code 3 when a ref
/// was left because of a conflict.
fn mirror_result(out: &Output, report: &apeiron::MirrorReport, remote: &str) -> u8 {
    out.result(report, |r| {
        for m in &r.refs {
            match (&m.report, &m.conflict) {
                (Some(t), _) => say!("{}", transferred(t, &m.name, remote)),
                (None, Some(e)) => say!("{}: not copied: {}", m.name, e.message),
                (None, None) => {}
            }
        }
        if r.refs.is_empty() {
            say!("No refs to copy");
        }
    });
    if report.conflicts().next().is_some() {
        EXIT_WARNINGS
    } else {
        0
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

thread_local! {
    /// Output caught for a file instead of printed (`report --out`).
    static CAUGHT: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

fn write_line(args: std::fmt::Arguments<'_>) {
    let caught = CAUGHT.with(|c| {
        c.borrow_mut().as_mut().map(|buf| {
            let _ = buf.write_fmt(args);
            buf.push(b'\n');
        })
    });
    if caught.is_some() {
        return;
    }
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
