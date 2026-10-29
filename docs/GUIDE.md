> **Türkçe:** [GUIDE.tr.md](GUIDE.tr.md)

# Apeiron User Guide

> **Status:** For the Stage 1 test ([ROADMAP.md](ROADMAP.md) §3). Apeiron has no release yet; it is built from source. All commands below work today. The full command reference is [CLI.md](CLI.md).

This guide takes you from installation to a daily backup on an external disk, restoring files, and the routine for the one-month test.

## 1. Before you start

| Point | Advice |
| --- | --- |
| Keep your old backup | Apeiron is in early development. During the test, leave your existing backup untouched as a safety net; do not delete it |
| Where the repository lives | On a different disk from the data it protects, for example an external disk. A repository is an ordinary folder |
| Platforms | Linux and macOS are the main targets of this guide; Windows works too, but the automatic backup in section 8 is Linux-only |

## 2. Installation

### 2.1 Tools

Apeiron is written in Rust. You need the Rust tools, Git and a C compiler.

| System | Command |
| --- | --- |
| Arch Linux, CachyOS, Manjaro | `sudo pacman -S rustup git gcc` then `rustup default stable` |
| Debian, Ubuntu | `sudo apt install git build-essential` then install Rust from [rustup.rs](https://rustup.rs) |
| macOS | `xcode-select --install` then install Rust from [rustup.rs](https://rustup.rs) |
| Windows | Install Git, the Visual Studio C++ build tools and Rust from [rustup.rs](https://rustup.rs) |

### 2.2 Building and installing

```bash
git clone https://github.com/efe-clk/apeiron.git
cd apeiron
cargo install --path crates/apeiron-cli
apeiron --version
```

`cargo install` builds the program and copies it to `~/.cargo/bin/apeiron`. If `apeiron --version` says "command not found", add `~/.cargo/bin` to your `PATH` (rustup usually does this; open a new terminal first).

### 2.3 Updating

```bash
cd apeiron
git pull
cargo install --path crates/apeiron-cli
```

## 3. Creating the repository

Plug in the external disk and find where it is mounted (on most Linux desktops `/run/media/<user>/<disk>`).

```bash
apeiron init /run/media/<user>/<disk>/apeiron
```

So that you do not have to give the path every time, add this line to `~/.zshrc` (or `~/.bashrc`) and open a new terminal:

```bash
export APEIRON_REPO=/run/media/<user>/<disk>/apeiron
```

Every command also accepts `--repo <path>`, which takes precedence.

### 3.1 Encrypted repository

If the disk may be lost or stolen, or will later be copied to a server, create the repository encrypted instead:

```bash
apeiron init --encrypt /run/media/<user>/<disk>/apeiron
```

The command asks for a password twice and prints a **recovery key** once (13 groups of four characters). Write the recovery key on paper and keep it apart from the computer and the disk. Either the password or the recovery key opens the repository; if both are lost, nobody can recover the data. Encryption can only be chosen when the repository is created.

Every later command asks `Password or recovery key:`. For the automatic backup (section 8), put the password in a file that only you can read and tell Apeiron where it is:

```bash
mkdir -p ~/.config/apeiron
printf '%s\n' 'your password' > ~/.config/apeiron/password
chmod 600 ~/.config/apeiron/password
export APEIRON_PASSWORD_FILE=~/.config/apeiron/password
```

Without a key, someone holding the disk sees only the number and size of the data files and the ref names (`backup/documents`), not file names or content ([FORMAT.md](FORMAT.md) §14.5).

### 3.2 Changing the password

Add the new password first, then remove the old one:

```bash
apeiron key add                 # asks the current password, then the new one twice
apeiron key list                # the old password is marked (in use)
apeiron key remove <id>         # the first 8 characters of the old password's id
```

If a password file is used, write the new password into it after `key add`. `apeiron key add --recovery` makes a new recovery key, for example when the old paper is lost; remove the old recovery key the same way. The last key cannot be removed, and `key remove` works only while no backup is running.

## 4. Backing up

Give every folder its own ref, so each has its own history:

```bash
apeiron backup ~/Documents --ref backup/documents -m "first backup"
apeiron backup ~/Pictures  --ref backup/pictures
```

| What you see | Meaning |
| --- | --- |
| `Snapshot 53373ba88b2e (backup/documents): 120 files, 14 folders, 1.2 GiB read, 3.4 MiB new` | The backup is complete. "new" is the data that was not in the repository yet; unchanged files are never stored twice |
| `warning: <path>: …` and exit code 3 | The backup is complete, but some entries were skipped (unreadable files, sockets) or changed while being read. Read the warnings |
| Progress line during the backup | Files and bytes done and the current file. Ctrl+C stops safely: the ref is not moved and nothing half-written becomes visible |

Ref names use only lowercase letters, digits, `.`, `_`, `-` and `/` (`backup/documents`, not `Belgelerim`).

## 5. Looking at backups

| Task | Command |
| --- | --- |
| List all refs | `apeiron refs` |
| History of one folder, newest first | `apeiron log backup/documents` |
| Files inside a backup | `apeiron ls backup/documents` or `apeiron ls backup/documents some/folder` |
| What changed between the last two backups | `apeiron diff backup/documents~1 backup/documents` |

A backup can be named by its ref (`backup/documents`), by the first 8 or more characters of its id (`53373ba8`), and `~N` goes N backups back (`backup/documents~3`). In `diff`, `A` means added, `D` deleted, `M` modified.

## 6. Restoring

Apeiron never overwrites existing files. The target folder must not exist or be empty.

| Task | Command |
| --- | --- |
| Everything from the latest backup | `apeiron restore backup/documents ~/restored` |
| From an older backup | `apeiron restore backup/documents~2 ~/restored` |
| One folder | `apeiron restore backup/documents ~/restored --path projects/thesis` |
| One file | `apeiron restore backup/documents ~/restored --path notes/todo.txt` |

Then copy what you need from `~/restored` to its place. Permissions and times are restored; owners only when you have the right to set them.

To hand a backup to someone without Apeiron, write it into an archive: `apeiron export backup/documents ~/documents.zip` (`.tar` and `.7z` work as well).

### 6.1 A second copy on an external disk

A backup on the same computer is lost with the computer. Keep a second copy on an external disk:

```bash
apeiron remote add disk /run/media/$USER/DISK/apeiron   # once; use the disk's folder
apeiron push disk backup/documents                      # after each backup
```

The first push copies everything; later ones copy only what is new. The folder on the disk is a complete repository with the same password: if the computer is lost, restore straight from it with `apeiron --repo /run/media/$USER/DISK/apeiron restore backup/documents ~/restored`. An interrupted push is simply run again.

### 6.2 A copy on a home server (SFTP)

A server at home or elsewhere that you reach with `ssh` can hold a copy too. The repository must be encrypted (section 3.1): only encrypted data leaves the computer.

1. Make sure `ssh user@nas.local` works without typing a password, with a key in the SSH agent or `~/.ssh/id_ed25519` without a passphrase. The first `ssh` asks you to check the server's key; Apeiron trusts only servers you have checked that way.
2. `apeiron remote add nas sftp://user@nas.local/srv/apeiron` (add `:2222` after the host for another port).
3. `apeiron push nas backup/documents`, as with a disk.

### 6.3 A copy in cloud storage (S3)

Amazon S3 and many other services (Backblaze B2, Cloudflare R2, Wasabi and others) offer S3-compatible storage. The repository must be encrypted (section 3.1).

1. Create a bucket and an access key at the service. Put the key into the environment, for example in `~/.config/apeiron/env`, readable only by you:

   ```bash
   export AWS_ACCESS_KEY_ID=…
   export AWS_SECRET_ACCESS_KEY=…
   ```

2. `apeiron remote add cloud 's3://BUCKET/apeiron?endpoint=https://SERVICE-ADDRESS&region=REGION'` (for Amazon itself, leave out `endpoint`).
3. `source ~/.config/apeiron/env && apeiron push cloud backup/documents`.

The service must support conditional writes; Apeiron refuses to move a ref otherwise.

## 7. Routine

| How often | Command | Why |
| --- | --- | --- |
| Daily (or automatic, section 8) | `apeiron backup …` for each folder | New snapshot of what changed |
| After each backup, or weekly | `apeiron push disk …` for each ref (section 6.1) | A second copy that survives the computer |
| Weekly | `apeiron verify` | Reads every stored byte and checks it. `OK: …` means no damage; exit code 4 lists damaged files |
| Monthly | `apeiron gc` | Deletes data that no ref reaches and that is older than 90 days. Try `apeiron gc --dry-run` first |
| Monthly | Restore a few files to a new folder and open them | A backup is only proven by restoring it |
| When a warning says so | `apeiron repair` | Removes stale locks, completes reflogs and rebuilds damaged index files |

## 8. Automatic daily backup (Linux)

A systemd user timer runs the backup every day, and also after the computer was off at the scheduled time. Create `~/.config/systemd/user/apeiron-backup.service`:

```ini
[Unit]
Description=Apeiron backup of Documents

[Service]
Type=oneshot
Environment=APEIRON_REPO=/run/media/<user>/<disk>/apeiron
# Only for an encrypted repository (section 3.1):
Environment=APEIRON_PASSWORD_FILE=%h/.config/apeiron/password
ExecStart=%h/.cargo/bin/apeiron backup %h/Documents --ref backup/documents
# Exit code 3 means "done with warnings".
SuccessExitStatus=3
```

and `~/.config/systemd/user/apeiron-backup.timer`:

```ini
[Unit]
Description=Daily Apeiron backup

[Timer]
OnCalendar=daily
Persistent=true

[Install]
WantedBy=timers.target
```

Then enable it and check the results:

```bash
systemctl --user daemon-reload
systemctl --user enable --now apeiron-backup.timer
systemctl --user list-timers apeiron-backup.timer
journalctl --user -u apeiron-backup.service
```

If the external disk is not plugged in, that day's run fails with "no config file" and the next run tries again. A built-in scheduler comes with the background service in Stage 2.

## 9. The one-month test

The exit criterion of Stage 1 is to back up and restore your own data with nothing but Apeiron for one month. Keep short notes; they decide what Stage 2 fixes first.

| Week | Do | Note down |
| --- | --- | --- |
| 1 | Install, create the repository, first backups, set up the timer | Anything unclear in this guide; how long the first backup took |
| 2 | Daily backups; `apeiron verify`; restore one folder and compare it | Warnings you saw; whether the restored files open |
| 3 | Restore one deleted file from an older backup (`~N`); `apeiron gc --dry-run` | How easy it was to find the right backup |
| 4 | Restore everything to a new folder and compare with the original; `apeiron gc` | Missing or wrong files, error messages, repository size |

## 10. When something goes wrong

| Message | Meaning and what to do |
| --- | --- |
| `no repository given` | Set `APEIRON_REPO` (section 3) or pass `--repo` |
| `invalid repository … no config file` | The path is wrong or the external disk is not mounted |
| `the repository is locked by another program` | `gc` runs only when nothing else uses the repository, and other commands cannot start during `gc`. Wait and try again |
| `warning: damaged file …` | Run `apeiron verify` for details and `apeiron repair` to rebuild damaged index files. Damaged packs cannot be repaired yet (Stage 2 brings copies on other targets) |
| `wrong password or recovery key` | Type the password again; the recovery key works too, with or without dashes. With a password file, check that its first line is the password |
| `already exists and is not an empty folder` | `restore` and `export` never overwrite; choose a new target |
| Exit code 130 | The command was cancelled with Ctrl+C; nothing half-done is visible |

Exit codes are listed in [CLI.md](CLI.md) §5. Please report problems as GitHub issues, and security problems as described in [SECURITY.md](../SECURITY.md).
