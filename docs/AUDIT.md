> **Türkçe:** [AUDIT.tr.md](AUDIT.tr.md)

# Audit Log

> **Status: Done.** This is the design of Stage 4 work item 4 ([ROADMAP.md](ROADMAP.md) §6): a record of who did what in the web interface ([WEB.md](WEB.md)) and to its users ([USERS.md](USERS.md)), which nobody can change unnoticed. The decision behind it is ADR-029 ([DECISIONS.md](DECISIONS.md)).

When several people share a backup server, the admin must be able to answer "who restored this folder, and when?" and "did someone try to sign in as me?".

## 1. What is recorded

| Event | When | Detail |
| --- | --- | --- |
| `sign_in` | Every sign-in, right or wrong | The name given, or for the identity provider `"via": "oidc"` with the identity ([IDENTITY.md](IDENTITY.md) §5); never the password or the code. A right password that still needs the authenticator code has the outcome `code_needed`. Git signs in at every request ([GIT.md](GIT.md) §7): only its wrong passwords are recorded here, with `"via": "git"` |
| `git_fetch` | Every pack sent to `git clone` or `git fetch` | The ref prefix and the refs asked for |
| `git_push` | Every `git push` | The ref prefix and each ref with its new Git id and error; the outcome is `conflict` when no ref moved |
| `sign_out` | Signing out | |
| `call:<operation>` | Every short operation, such as `list` or `log` | The request after the checks of USERS.md §4, with snapshots as ids and paths resolved; a `password` in it is written as `null` |
| `start:<operation>` | Every long operation started: `backup`, `restore`, `verify`, `search` | The same |
| `done:<operation>` | When the result of a long operation is collected | The job number |
| `start:backup`, `done:backup`, `start:restore`, `done:restore` with `"via": "cli"` | `apeiron backup` and `apeiron restore`, when the web server of this computer has an audit log: that of the tenant whose repository it is, or else the server's own | The repository, the folder or snapshot and target, and at the end the new snapshot or the files and bytes restored; the outcome is the error kind if it failed. A start that cannot be recorded stops the command |
| `cancel` | A request to stop a job | The job number |
| `share_add`, `share_remove` | Making or removing a share link | The link's id, version, path and end ([SHARE.md](SHARE.md) §6) |
| `shared:list`, `shared:read`, `shared:upload` | Every use of a share link, without signing in; `shared:upload` names the file sent ([SHARE.md](SHARE.md) §7) | The link's id and the path; never the link's secret |
| `shared:unlock` | Every password given for a share link | The link's id; never the password or the secret |
| `user_add`, `user_change`, `user_password`, `user_remove` | `apeiron web user`, or an admin on the Users page | The name, role, backups, folders and identity; never the password |
| `user_second_factor_on`, `user_second_factor_off` | `apeiron web user second-factor` | The name; never the secret |
| `oidc_set`, `oidc_remove` | `apeiron web oidc` | The provider's settings; never the client secret |
| `replicate` | Each push to a replica by the web server | The remote, refs and objects, the refs left by a conflict, or the error ([REPLICATION.md](REPLICATION.md) §7) |
| `tenant_add`, `tenant_limit`, `tenant_remove` | `apeiron web tenant`, in that tenant's own log | The tenant's settings ([TENANTS.md](TENANTS.md) §5) |

An operation refused by the checks is recorded with its error, so attempts are visible too. Polling a job, the session state and the folder browser are not recorded. The desktop interface records nothing: it serves only the person who runs it. The CLI's other commands are not recorded; the repository's reflog records every ref change.

## 2. The file

The log is `audit.log` in the state folder (`~/.local/state/apeiron` on Linux), one JSON object per line:

```json
{"seq":12,"time":"2026-10-30T14:03:11Z","user":"ayse","from":"192.168.1.20:51544","event":"start:restore","detail":{"snapshot":"5c1e…","target":"/home/ayse/out"},"outcome":"ok","prev":"9f86…"}
```

| Field | Meaning |
| --- | --- |
| `seq` | 1 for the first line, one more for each next line |
| `time` | UTC time of the event |
| `user` | The signed-in user, or for the CLI the system user who ran it; absent for `apeiron web user`, for sign-ins that failed and for uses of a share link |
| `from` | The address and port of the browser's connection; behind a proxy, the proxy's; `cli` for the CLI |
| `event`, `detail` | §1; strings in `detail` longer than 256 characters are cut |
| `outcome` | `ok`, or the error kind (`not_found`, `key`, `unsupported` …); a long operation is recorded as `ok` when it is allowed and starts, and its result comes with `done:` |
| `prev` | SHA-256 of the previous line's bytes, hex; 64 zeros on the first line |

A short operation is recorded after it runs, so no result leaves unrecorded; a long one before it starts, so nothing runs unrecorded. Lines are only ever appended. Each is written under a lock (`audit.log.lock` beside the log), so the server and `apeiron web user` can write at the same time and readers never see half a line, and flushed to disk before the request goes on. If a line cannot be written, the request fails: nothing happens that is not recorded.

## 3. Checking it

Because each line holds the hash of the one before it, changing, removing or reordering a line breaks the chain from there on. `apeiron web audit verify` reads the whole log, reports the first broken line, and prints the hash of the last line. Writing that hash down from time to time, or sending it elsewhere, also reveals a log rewritten from start to end, which the chain alone cannot.

When `audit.log` has grown past 16 MiB, about 50 000 events, the next event first renames it to `audit.log.<seq>`, where `<seq>` is the number of its last event written with 12 digits, and starts a new `audit.log`. The chain goes on across the files: the first line of the new file holds the hash of the last line of the old one, and its `seq` follows on. `verify` and `show` read the parts in order as one log. Old parts may be moved to an archive or deleted when they no longer have to be kept; `verify` then checks the chain from the oldest part left and says from which event on.

## 4. Reading it

| Where | What |
| --- | --- |
| `apeiron web audit show [--user <name>] [--limit <n>]` | The newest events, 50 by default, oldest first; `--json` for JSON |
| `apeiron web audit verify` | §3 |
| The Activity page | For an `admin` in the web interface: the newest 200 events, newest first |
| `GET /api/audit?limit=<n>` | The same for an `admin`; others get `unsupported` |

## 5. A copy in syslog

`apeiron web --audit-syslog <target>` also sends each event, after it is written to the file, to a syslog server (`udp://host:port`) or a local syslog socket (`/dev/log` on Linux), so that a central log service holds a copy the server cannot take back. Each event is one RFC 5424 message with the facility `authpriv` and the severity `info`; the app name is `apeiron`, or `apeiron-<tenant>` with `--tenants`, the message id is the event's name, and the message is the event's line exactly as in the file, with its `seq` and `prev`, so the copy can be checked against the chain.

The file stays the record: an event is sent only after it is on disk, and an event that cannot be sent does not stop the request; the server writes a warning with its `seq` to its error output, and the gap in the `seq` numbers the central service received shows it too. UDP may lose messages without a warning; a local socket to a syslog daemon that forwards over TCP or TLS does not.

## 6. Not part of this work item

| Item | Where |
| --- | --- |
| Archiving old parts elsewhere | Left to the admin: the parts are plain files that never change once renamed |

## 7. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-server` | The log, recording in web mode, `/api/audit` and the Activity page | Done |
| `apeiron-cli` | `apeiron web audit`, recording user changes | Done |
