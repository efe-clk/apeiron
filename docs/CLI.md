> **Türkçe:** [CLI.tr.md](CLI.tr.md)

# Apeiron Command-Line Interface

> **Status: In progress.** This is the Stage 1 design of the `apeiron` command ([ROADMAP.md](ROADMAP.md) §3). The parts marked Done work today.

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

### 2.1 Encrypted repositories

`apeiron init --encrypt <path>` asks for a new password twice and prints the recovery key once ([FORMAT.md](FORMAT.md) §14). Every other command on an encrypted repository needs the password or the recovery key:

| Source | Example | Priority |
| --- | --- | --- |
| `--password-file` option | `apeiron --password-file ~/.config/apeiron/password log` | 1 |
| `APEIRON_PASSWORD_FILE` environment variable | `export APEIRON_PASSWORD_FILE=~/.config/apeiron/password` | 2 |
| Asked on the terminal, without showing it | `Password or recovery key:` | 3 |

The first line of the file is the password; `init --encrypt` also reads the new password from it. A wrong password or recovery key ends the command with exit code 1 and the error kind `key`. With `--json`, `init` adds the field `recovery_key`.

To change a password, add the new one with `apeiron key add`, find the old one in `apeiron key list` (it is marked `(in use)` when it opened the repository) and remove it with `apeiron key remove <id>`. The first 8 characters of an id are enough. The last key cannot be removed, and `key remove`, like `gc`, needs the repository to itself.

### 2.2 Remotes

A remote is a complete copy of the repository in another place ([FORMAT.md](FORMAT.md) §15). `apeiron remote add disk /run/media/user/disk/apeiron` names a folder; `apeiron push disk` copies only what the folder does not have yet, and the folder can then be opened like any repository, with the same password. `apeiron pull disk` brings snapshots back. A ref only moves forward: if both copies have snapshots the other lacks, the command stops with a conflict and changes nothing.

A home server is reached with SFTP: `apeiron remote add nas sftp://user@nas.local/srv/apeiron`. Apeiron logs in like `ssh` does, with the keys of the SSH agent or `~/.ssh/id_ed25519`, `id_ecdsa` or `id_rsa` without a passphrase, and checks the server against `~/.ssh/known_hosts`; a server that is not there is refused, so connect once with `ssh` first. Only an encrypted repository can use an SFTP remote.

Cloud storage is reached with S3: `apeiron remote add cloud 's3://bucket/apeiron?endpoint=https://s3.example.com&region=eu-1'`. The `endpoint` is needed for S3-compatible services other than Amazon's. The keys come from `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`; they are never stored. The service must support conditional writes, and only an encrypted repository can use an S3 remote.

## 3. Commands

| Command | What it does | Status |
| --- | --- | --- |
| `apeiron init <path> [--encrypt]` | Creates an empty repository; `--encrypt` encrypts it with a password (section 2.1) | Done |
| `apeiron backup <folder> [--ref <name>] [--message <text>]` | Stores the folder as a new snapshot and moves the ref to it (default ref: `main`) | Done |
| `apeiron log [<ref>]` | Lists the snapshots of a ref, newest first | Done |
| `apeiron ls <snapshot> [<path>]` | Lists the files and folders inside a snapshot | Done |
| `apeiron diff <snapshot> <snapshot> [--text]` | Lists files added, removed and changed between two snapshots; `--text` also shows the changed lines of files a plugin handles ([TEXT.md](TEXT.md)) | Done |
| `apeiron restore <snapshot> <target> [--path <path>]` | Writes a snapshot (or one path in it) into a new or empty folder | Done |
| `apeiron export <snapshot> <file> [--format <format>] [--path <path>]` | Writes a snapshot (or one path in it) into a new `.tar`, `.zip` or `.7z` file; the extension chooses the format unless `--format` is given | Done |
| `apeiron restore-test [<snapshot>] [--sample-mb <n>]` | Restores a random sample (default 100 MiB) of a snapshot into a temporary folder, checks every file and removes the folder; exit code 4 if a file is damaged | Done |
| `apeiron daemon run\|once\|status` | Runs scheduled jobs ([DAEMON.md](DAEMON.md)) | Done |
| `apeiron refs` | Lists all refs and the snapshots they point to | Done |
| `apeiron verify` | Reads and checks every pack, index and object | Done |
| `apeiron repair` | Removes stale ref locks, adds lost reflog lines and replaces damaged index files | Done |
| `apeiron gc [--keep-days <days>] [--dry-run]` | Deletes data no ref and no reflog line of the last 90 days (or `--keep-days`) can reach; `--dry-run` only reports | Done |
| `apeiron key list` | Lists the passwords and recovery keys of an encrypted repository | Done |
| `apeiron key add [--recovery] [--new-password-file <file>]` | Adds a password (asked twice, or read from the file), or with `--recovery` a new recovery key, printed once | Done |
| `apeiron remote add <name> <url>` | Adds a remote: a folder, such as one on an external disk, `sftp://[user@]host[:port]/path` or `s3://bucket/path?endpoint=…&region=…` (section 2.2) | Done |
| `apeiron remote list` | Lists the remotes | Done |
| `apeiron remote remove <name>` | Forgets a remote; the copy itself is not touched | Done |
| `apeiron clone <url> <path>` | Makes a new repository from a remote copy, with the same password, pulls every ref and names the remote `origin`; the way to set up a second device | Done |
| `apeiron sync <folder> <remote> [--ref <name>] [--mode two-way\|send\|receive] [--dry-run]` | Keeps a folder equal on several devices through a remote: backs it up, pulls, merges, changes the folder and pushes ([SYNC.md](SYNC.md)); prints `+`, `-` and `~` for added, removed and changed files and one line per conflict | Done |
| `apeiron ref create <name> [<snapshot>]` | Creates a ref, such as a branch, at a snapshot (default `main`) ([BRANCH.md](BRANCH.md)) | Done |
| `apeiron ref delete <name>` | Deletes a ref; `apeiron reflog <name>` still shows where it pointed | Done |
| `apeiron merge <from> [--into <ref>] [--message <text>] [--dry-run]` | Merges a snapshot, usually a branch, into a ref (default `main`); exit code 3 when there are conflicts | Done |
| `apeiron search [<pattern>] [--text <text>] [--larger <size>] [--smaller <size>] [--after <date>] [--before <date>] [--in <snapshot>] [--history] [--limit <n>]` | Finds files, folders and links by name, size, modification day and text in a snapshot (default `main`) or, with `--history`, in every earlier snapshot too; prints `path:line: text` for text matches and the snapshot and its time before each result with `--history` ([SEARCH.md](SEARCH.md)) | Done |
| `apeiron ui [--no-browser]` | Opens the desktop interface in the default browser, with the repository of `--repo` open if given; `--no-browser` prints the address instead ([DESKTOP.md](DESKTOP.md)) | Done |
| `apeiron plugin add <file.wasm>` | Checks a plugin and installs it for the user; a merge or sync then lets it merge the files it handles instead of making conflict copies ([PLUGIN.md](PLUGIN.md)) | Done |
| `apeiron plugin list` | Lists the installed plugins | Done |
| `apeiron plugin remove <name>` | Removes a plugin | Done |
| `apeiron git import <git-repo> [--prefix <name>]` | Stores every branch, tag and remote-tracking branch of a Git repository, with its history, under `git/` ([GIT.md](GIT.md)) | Done |
| `apeiron git export <git-repo> [--prefix <name>]` | Writes them back into a Git repository with the same commit ids; exit code 3 if one could not be reproduced | Done |
| `apeiron push <remote> [<ref>]` | Copies the snapshots of a ref (default `main`) to a remote and moves the ref there | Done |
| `apeiron pull <remote> [<ref>]` | Copies the snapshots of a ref from a remote and moves the local ref forward | Done |
| `apeiron key remove <id>` | Removes a password or recovery key; the last one cannot be removed | Done |

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
| 3 | Finished with warnings (for example skipped files), or `sync` or `merge` found conflicts, or `sync` left files for the next round |
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
| `apeiron-remote` | SFTP and S3 remotes (ADR-018) | In progress |

| Need | Proposed library | Reason |
| --- | --- | --- |
| Command-line parsing | `clap` | The most widely used Rust parser; generates help and checks options |
| JSON output | `serde_json` | Standard JSON library for `serde`, already used for configuration |
| Ctrl+C | `ctrlc` | Small, widely used; the same code on Linux, macOS and Windows |
| Hidden password prompt | `rpassword` | Small, widely used; reads from the terminal without echo on Linux, macOS and Windows |
| SFTP | `russh`, `russh-sftp` | Pure Rust SSH and SFTP, also on Windows (ADR-018) |
| S3 | `object_store` | Maintained by Apache Arrow; conditional writes on S3 (ADR-018) |
| tar, zip and 7z export | `tar`, `zip`, `sevenz-rust2` | Pure Rust; the archives open with the standard `tar`, `unzip` and `7z` tools, keeping permissions and symbolic links |

## 8. Open questions

| Question | Proposal |
| --- | --- |
| Default ref for `backup` | `main`; several folders use `--ref backup/<name>` |
| May `restore` write into a non-empty folder? | No in Stage 1; a later `--overwrite` flag may allow it |
| Can `export` write RAR files? | No. Only RARLAB's own software may create RAR archives; there is no open implementation, and the free unRAR code may not be used to create them. Reading RAR files for an import may be added later |
| Concurrent commands on one repository | Decided: commands may run side by side; `gc` runs only alone and otherwise stops with a "locked" error ([FORMAT.md](FORMAT.md) §12) |
