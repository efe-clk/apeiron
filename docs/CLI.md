> **Türkçe:** [CLI.tr.md](CLI.tr.md)

# Apeiron Command-Line Interface

> **Status: In progress.** This is the Stage 1 design of the `apeiron` command ([ROADMAP.md](ROADMAP.md) §3). The parts marked Done work today; `export` and `gc` are still planned.

The CLI is the first interface of Apeiron. Like every other interface it uses only the public API ([API.md](API.md)); it has no access to the core that a third-party program would not have.

## 1. Principles

| Principle | Rule | Status |
| --- | --- | --- |
| API only | The CLI calls only the public `apeiron` crate | Done |
| Never destroy data silently | No command overwrites or deletes existing files unless an explicit flag asks for it | Done |
| Machine-readable output | Every command accepts `--json` and then prints one JSON document to standard output | Done |
| Progress | Long commands show progress on standard error when it is a terminal | Done |
| Safe cancellation | Ctrl+C stops a command; the repository stays in its previous state (old or new, never half) | Done |
| Continue past unreadable files | A file that cannot be read is reported and skipped; the backup continues | Done |
| Clear exit codes | Each outcome has its own exit code (section 5) | Done |

## 2. Choosing the repository

| Source | Example | Priority |
| --- | --- | --- |
| `--repo` option | `apeiron --repo /mnt/backup/apeiron log` | 1 |
| `APEIRON_REPO` environment variable | `export APEIRON_REPO=/mnt/backup/apeiron` | 2 |

If neither is given, the command stops with a usage error. A repository on an external disk is just a repository at a path on that disk.

## 3. Commands

| Command | What it does | Status |
| --- | --- | --- |
| `apeiron init <path>` | Creates an empty repository | Done |
| `apeiron backup <folder> [--ref <name>] [--message <text>]` | Stores the folder as a new snapshot and moves the ref to it (default ref: `main`) | Done |
| `apeiron log [<ref>]` | Lists the snapshots of a ref, newest first | Done |
| `apeiron ls <snapshot> [<path>]` | Lists the files and folders inside a snapshot | Done |
| `apeiron diff <snapshot> <snapshot>` | Lists files added, removed and changed between two snapshots | Done |
| `apeiron restore <snapshot> <target> [--path <path>]` | Writes a snapshot (or one path in it) into a new or empty folder | Done |
| `apeiron export <snapshot> <file>` | Writes a snapshot into a `.tar` or `.zip` file | Planned |
| `apeiron refs` | Lists all refs and the snapshots they point to | Done |
| `apeiron verify` | Reads and checks every pack, index and object | Done |
| `apeiron repair` | Removes stale ref locks and adds lost reflog lines | Done |
| `apeiron gc` | Deletes objects no ref or reflog can reach | Planned (after garbage collection exists) |

A `<snapshot>` can be written as a ref name (`main`), a full snapshot id, or the first 8 or more characters of an id if they match exactly one snapshot. A `~N` suffix goes N steps back through first parents: `main~1` is the snapshot before `main`.

## 4. What a backup stores

| Item | Stored | Status |
| --- | --- | --- |
| Files, folders, symbolic links | Yes | Done |
| Permission bits and modification time | Yes | Done |
| Owner (uid/gid) on Linux and macOS | Yes | Done |
| Extended attributes | Not in Stage 1 | Planned |
| Hard links | As separate files; the link between them is not kept ([FORMAT.md](FORMAT.md) §6.3) | Done |
| Device files, sockets, pipes | No; skipped and reported | Done |

A file that changes while it is being read is stored as read and reported as a warning. Restore recreates permissions and times; owners only when run with permission to set them.

## 5. Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Error; nothing was changed in the repository |
| 2 | Usage error (wrong command or option) |
| 3 | Finished with warnings (for example skipped files) |
| 4 | Damage found by `verify` |
| 130 | Cancelled with Ctrl+C |

## 6. Example session

```bash
apeiron init /mnt/backup/apeiron
export APEIRON_REPO=/mnt/backup/apeiron
apeiron backup ~/Documents --message "first backup"
apeiron backup ~/Documents
apeiron log
apeiron diff main~1 main
apeiron restore main /tmp/documents-restored
apeiron verify
```

## 7. Crates and libraries

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron` | Public Rust API ([API.md](API.md) §8) | In progress |
| `apeiron-services` | Folder ↔ snapshot, diff and export logic used by the API | In progress |
| `apeiron-cli` | The `apeiron` program | In progress |

| Need | Proposed library | Reason |
| --- | --- | --- |
| Command-line parsing | `clap` | The most widely used Rust parser; generates help and checks options |
| JSON output | `serde_json` | Standard JSON library for `serde`, already used for configuration |
| Ctrl+C | `ctrlc` | Small, widely used; the same code on Linux, macOS and Windows |
| tar and zip export | `tar`, `zip` | Widely used, pure Rust |

## 8. Open questions

| Question | Proposal |
| --- | --- |
| Default ref for `backup` | `main`; several folders use `--ref backup/<name>` |
| May `restore` write into a non-empty folder? | No in Stage 1; a later `--overwrite` flag may allow it |
| Concurrent commands on one repository | A repository lock (Stage 1, [FORMAT.md](FORMAT.md) §12) makes a second writer wait or stop |
