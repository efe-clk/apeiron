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
| Find files by name in a backup | `apeiron search '*.pdf' --in backup/documents` |
| Find a file that was deleted or changed earlier | `apeiron search 'report*' --in backup/documents --history` |
| Find text files holding some words | `apeiron search --text 'invoice 2026' --in backup/documents` |

A backup can be named by its ref (`backup/documents`), by the first 8 or more characters of its id (`53373ba8`), and `~N` goes N backups back (`backup/documents~3`). In `diff`, `A` means added, `D` deleted, `M` modified. A search pattern needs quotes so the shell does not expand `*`; [SEARCH.md](SEARCH.md) lists every criterion.

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

### 6.4 A second device

A second computer starts from any of these copies:

```bash
apeiron clone /run/media/$USER/DISK/apeiron ~/apeiron   # or an sftp:// or s3:// address
```

It asks for the password of the copy and creates a repository at `~/apeiron` with every backup in it; the copy is then known as `origin`, so `apeiron --repo ~/apeiron pull origin backup/documents` brings later backups. If `clone` is stopped before it finishes, delete the new folder and run it again.

### 6.5 The same folder on two computers

Sync keeps a folder, such as `~/Documents`, equal on two or more computers through any of these copies ([SYNC.md](SYNC.md)). On the first computer:

```bash
apeiron sync ~/Documents disk              # backs up the folder and sends it
```

On the second, after `clone` (section 6.4):

```bash
apeiron --repo ~/apeiron sync --dry-run ~/Documents origin   # shows what would change
apeiron --repo ~/apeiron sync ~/Documents origin
```

From then on, run `apeiron sync` on either computer, or let the background service do it (section 8, `do = "sync"`, `every = "15m"`). Each run backs up the folder first, so every state stays in the history and can be restored. When both computers changed the same file differently, both versions are kept: the other one appears beside it as `name (conflict COMPUTER DATE TIME).ext`. Look at both, keep what you want and delete the copy; the next sync spreads that too.

### 6.6 A Git repository

A Git project can be kept in Apeiron with its whole history ([GIT.md](GIT.md)):

```bash
apeiron git import ~/code/project           # branches and tags become git/heads/…, git/tags/…
apeiron git export ~/restored/project.git   # the same history, with the same commit ids
```

Run `git import` again to store new commits; after `git export`, `git clone ~/restored/project.git` gives a working copy.

### 6.7 Trying something on a branch

A branch keeps an experiment apart from `main` ([BRANCH.md](BRANCH.md)):

```bash
apeiron ref create trial                      # a branch starting where main is
apeiron restore trial ~/trial                 # work on it in its own folder
apeiron backup ~/trial --ref trial            # record the changes
apeiron merge trial --dry-run                 # see what joining it would change
apeiron merge trial                           # join it into main
```

When both `main` and the branch changed the same file, `main` keeps its version and the branch's version is added as `name (conflict trial).ext`.

### 6.8 Merging text files line by line

Without a plugin, two changes to the same text file always leave a conflict copy. The text plugin joins changes to different lines ([TEXT.md](TEXT.md)). It is built once and installed for your user:

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2 --manifest-path plugins/text/Cargo.toml
apeiron plugin add plugins/text/target/wasm32-wasip2/release/apeiron_text.wasm
apeiron diff main~1 main --text               # see which lines changed
```

Install it on every computer that syncs the folder; a computer without it makes conflict copies instead, which loses nothing.

### 6.9 Without the command line

The desktop app does everything in sections 3 to 6 that most people need, in a window: opening or creating the backup folder, backing up, looking at versions, restoring and checking ([DESKTOP.md](DESKTOP.md)). On Linux it needs the WebKitGTK files first (`sudo apt install libwebkit2gtk-4.1-dev` on Debian and Ubuntu, `sudo pacman -S webkit2gtk-4.1` on Arch):

```bash
cargo install --locked --path crates/apeiron-desktop
apeiron-desktop
```

Where the desktop app cannot be built, `apeiron ui` shows the same pages in your browser; close the terminal or press Ctrl+C to stop it.

### 6.10 From another computer

On a home server, `apeiron web` shows the same pages to a browser on another computer of your network ([WEB.md](WEB.md)). Add yourself as its first user once, then start it with the repository and the folders it may use:

```bash
apeiron web user add efe --role admin
apeiron web --repo /srv/backup --folder /home/efe
```

Open `https://<server address>:8443` on the other computer. The browser warns about the certificate the first time; accept it only if the fingerprint it shows is the one the server printed. Run it as an ordinary user, never as root.

Other people of the house get their own name, backups and folders, for example `apeiron web user add ayse --role member --backup "backup/ayse-*" --folder /home/ayse`; they see only their own backups ([USERS.md](USERS.md)).

## 7. Routine

| How often | Command | Why |
| --- | --- | --- |
| Daily (or automatic, section 8) | `apeiron backup …` for each folder | New snapshot of what changed |
| After each backup, or weekly | `apeiron push disk …` for each ref (section 6.1) | A second copy that survives the computer |
| Weekly | `apeiron verify` | Reads every stored byte and checks it. `OK: …` means no damage; exit code 4 lists damaged files |
| Monthly | `apeiron gc` | Deletes data that no ref reaches and that is older than 90 days. Try `apeiron gc --dry-run` first |
| Monthly | Restore a few files to a new folder and open them | A backup is only proven by restoring it |
| When a warning says so | `apeiron repair` | Removes stale locks, completes reflogs and rebuilds damaged index files |

## 8. Automatic backups (background service)

The background service runs backups, pushes and checks on a schedule ([DAEMON.md](DAEMON.md)). A run missed while the computer was off or asleep is made once at the next chance.

1. Write the jobs into `~/.config/apeiron/jobs.toml` (macOS: `~/Library/Application Support/apeiron/jobs.toml`, Windows: `%APPDATA%\apeiron\jobs.toml`):

   ```toml
   [job.documents]
   repo = "/run/media/me/disk/apeiron"
   password_file = "~/.config/apeiron/password"   # only for an encrypted repository
   do = "backup"
   folder = "~/Documents"
   ref = "backup/documents"
   every = "day"
   at = "12:30"
   push = ["nas"]                                 # optional: remotes (section 6)

   [job.check]
   repo = "/run/media/me/disk/apeiron"
   password_file = "~/.config/apeiron/password"
   do = "restore-test"
   ref = "backup/documents"
   every = "week"
   ```

2. Try it once: `apeiron daemon once` runs every job that is due and prints the results.
3. Let the system start `apeiron daemon run` when you log in:

   | System | How |
   | --- | --- |
   | Linux | Create `~/.config/systemd/user/apeiron.service` (below), then `systemctl --user enable --now apeiron` |
   | macOS | Create `~/Library/LaunchAgents/org.apeiron.daemon.plist` (below), then `launchctl load -w ~/Library/LaunchAgents/org.apeiron.daemon.plist` |
   | Windows | `schtasks /Create /SC ONLOGON /TN Apeiron /TR "%USERPROFILE%\.cargo\bin\apeiron.exe daemon run"` |

   ```ini
   [Unit]
   Description=Apeiron background service

   [Service]
   ExecStart=%h/.cargo/bin/apeiron daemon run
   Restart=on-failure

   [Install]
   WantedBy=default.target
   ```

   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
     <key>Label</key><string>org.apeiron.daemon</string>
     <key>ProgramArguments</key>
     <array><string>/Users/me/.cargo/bin/apeiron</string><string>daemon</string><string>run</string></array>
     <key>RunAtLoad</key><true/>
     <key>KeepAlive</key><true/>
   </dict>
   </plist>
   ```

4. Look at the results now and then: `apeiron daemon status` lists each job's last run and the next one; exit code 4 means a job failed. The details are in `daemon.log` next to the state (Linux: `~/.local/state/apeiron`).

On a home server where nobody logs in, `loginctl enable-linger <user>` keeps the Linux service running without a login. If the external disk is not plugged in, the job fails with "no config file" and is tried again an hour later.

## 9. The one-month test

The exit criterion of Stage 1 is to back up and restore your own data with nothing but Apeiron for one month. Keep short notes; they decide what Stage 2 fixes first.

| Week | Do | Note down |
| --- | --- | --- |
| 1 | Install, create the repository, first backups, set up the background service | Anything unclear in this guide; how long the first backup took |
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
