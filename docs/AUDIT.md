> **Türkçe:** [AUDIT.tr.md](AUDIT.tr.md)

# Audit Log

> **Status: Done.** This is the design of Stage 4 work item 4 ([ROADMAP.md](ROADMAP.md) §6): a record of who did what in the web interface ([WEB.md](WEB.md)) and to its users ([USERS.md](USERS.md)), which nobody can change unnoticed. The decision behind it is ADR-029 ([DECISIONS.md](DECISIONS.md)).

When several people share a backup server, the admin must be able to answer "who restored this folder, and when?" and "did someone try to sign in as me?".

## 1. What is recorded

| Event | When | Detail |
| --- | --- | --- |
| `sign_in` | Every sign-in, right or wrong | The name given, or for the identity provider `"via": "oidc"` with the identity ([IDENTITY.md](IDENTITY.md) §5); never the password |
| `sign_out` | Signing out | |
| `call:<operation>` | Every short operation, such as `list` or `log` | The request after the checks of USERS.md §4, with snapshots as ids and paths resolved |
| `start:<operation>` | Every long operation started: `backup`, `restore`, `verify`, `search` | The same |
| `done:<operation>` | When the result of a long operation is collected | The job number |
| `cancel` | A request to stop a job | The job number |
| `share_add`, `share_remove` | Making or removing a share link | The link's id, version, path and end ([SHARE.md](SHARE.md) §6) |
| `shared:list`, `shared:read` | Every use of a share link, without signing in | The link's id and the path; never the link's secret |
| `user_add`, `user_change`, `user_password`, `user_remove` | `apeiron web user` | The name, role, backups, folders and identity; never the password |
| `oidc_set`, `oidc_remove` | `apeiron web oidc` | The provider's settings; never the client secret |

An operation refused by the checks is recorded with its error, so attempts are visible too. Polling a job, the session state and the folder browser are not recorded. The desktop interface records nothing: it serves only the person who runs it.

## 2. The file

The log is `audit.log` in the state folder (`~/.local/state/apeiron` on Linux), one JSON object per line:

```json
{"seq":12,"time":"2026-10-30T14:03:11Z","user":"ayse","from":"192.168.1.20:51544","event":"start:restore","detail":{"snapshot":"5c1e…","target":"/home/ayse/out"},"outcome":"ok","prev":"9f86…"}
```

| Field | Meaning |
| --- | --- |
| `seq` | 1 for the first line, one more for each next line |
| `time` | UTC time of the event |
| `user` | The signed-in user; absent for `apeiron web user`, for sign-ins that failed and for uses of a share link |
| `from` | The address and port of the browser's connection; behind a proxy, the proxy's |
| `event`, `detail` | §1; strings in `detail` longer than 256 characters are cut |
| `outcome` | `ok`, or the error kind (`not_found`, `key`, `unsupported` …); a long operation is recorded as `ok` when it is allowed and starts, and its result comes with `done:` |
| `prev` | SHA-256 of the previous line's bytes, hex; 64 zeros on the first line |

A short operation is recorded after it runs, so no result leaves unrecorded; a long one before it starts, so nothing runs unrecorded. Lines are only ever appended. Each is written under a lock on the file, so the server and `apeiron web user` can write at the same time, and flushed to disk before the request goes on. If a line cannot be written, the request fails: nothing happens that is not recorded.

## 3. Checking it

Because each line holds the hash of the one before it, changing, removing or reordering a line breaks the chain from there on. `apeiron web audit verify` reads the whole log, reports the first broken line, and prints the hash of the last line. Writing that hash down from time to time, or sending it elsewhere, also reveals a log rewritten from start to end, which the chain alone cannot.

The log never rotates; one event takes about 300 bytes, so a million events take about 300 MB.

## 4. Reading it

| Where | What |
| --- | --- |
| `apeiron web audit show [--user <name>] [--limit <n>]` | The newest events, 50 by default, oldest first; `--json` for JSON |
| `apeiron web audit verify` | §3 |
| The Activity page | For an `admin` in the web interface: the newest 200 events, newest first |
| `GET /api/audit?limit=<n>` | The same for an `admin`; others get `unsupported` |

## 5. Not part of this work item

| Item | Where |
| --- | --- |
| Recording the CLI's own backups and restores | Later; the repository's reflog already records every ref change |
| Sending events to syslog or a central log service | Stage 5 |
| Rotating and archiving the log | Later |

## 6. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-server` | The log, recording in web mode, `/api/audit` and the Activity page | Done |
| `apeiron-cli` | `apeiron web audit`, recording user changes | Done |
