> **Türkçe:** [TUI.tr.md](TUI.tr.md)

# Terminal Interface

> **Status: Done.** `apeiron tui` shows the screens of the desktop interface ([DESKTOP.md](DESKTOP.md) §2) inside a terminal, for servers without a screen and for connections over SSH. The decision behind it is ADR-053 ([DECISIONS.md](DECISIONS.md)).

## 1. How it is built

| Part | What it is | Status |
| --- | --- | --- |
| `apeiron-tui` | A library crate that draws the screens with ratatui and reads keys with crossterm | Done |
| `apeiron tui` | Starts it in the terminal, with the repository of `--repo` open if given | Done |
| Operations | The operations by name of the public API (SDK.md §2.1), the same ones the pages call, so every screen does what its page does | Done |
| Texts | The dictionary of the pages (`crates/apeiron-server/web/i18n.js`), read when the program starts; English or Turkish by `LC_ALL`, `LC_MESSAGES` or `LANG` | Done |

There is no server and no port: the program calls the API directly, and long operations run as jobs whose progress is drawn several times a second.

## 2. What the user can do

Every screen of the desktop interface is here, with the same words.

| Screen | What it does | Status |
| --- | --- | --- |
| Start | Opens a repository (asking the password if encrypted) or creates one; shows a new recovery key once | Done |
| Overview | The backups, each with its folder and the time of its newest version | Done |
| Back up | A folder into `backup/<folder name>` or a name the user gives, with a note, progress and cancel | Done |
| History | The versions of a backup, newest first | Done |
| Browse | The folders and files inside a version | Done |
| Restore | A version, or one file or folder in it, into a new folder | Done |
| Compare | What changed against the version before, with the changed lines of text files | Done |
| Verify | Checks the whole repository | Done |
| Sync | Remotes: list, add and remove; a sync round in either direction or a dry run; conflict copies and keeping either version | Done |
| Passwords | Lists keys, adds a password or a recovery key, removes a key but never the last | Done |
| Plugins | Lists, installs from a `.wasm` file and removes plugins | Done |
| Schedule | The jobs of the background service with their last and next run; the jobs file opens in the user's editor and is saved only when valid | Done |
| Reviews | Lists reviews, asks for one, and shows one with its steps and changes; comment, approve, ask for changes, close and merge | Done |
| Retention | Lists retention rules and adds one after a warning | Done |
| Report | A compliance report, optionally with verify and the remotes | Done |

Folders are chosen with a folder browser inside the terminal, or typed.

## 3. Keys

| Key | What it does |
| --- | --- |
| `Tab`, `Shift+Tab` | The next or previous screen in the menu; inside a form, the next or previous field |
| `↑`, `↓`, `PgUp`, `PgDn` | Moves in a list or scrolls |
| `Enter` | Opens the chosen row or presses the chosen button |
| The letter in brackets | The action of a button, such as `[r]` for restore |
| `Esc` | Back, or stops a running operation |
| `Ctrl+C` | Leaves the program; a running operation is stopped first |

Questions that cannot be undone, such as removing a key or adding a retention rule, wait for `y`.

## 4. Not part of this work item

| Item | Why not now |
| --- | --- |
| The web interface's screens (sign-in, users, tenants, links, activity) | They belong to a server with many users; `apeiron web` and its commands manage them |
| A mouse | Every action has a key; terminals over SSH often pass no mouse |
| Colors chosen by the user | The terminal's own colors are used |
