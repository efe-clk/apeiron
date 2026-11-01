> **Türkçe:** [SHARE.tr.md](SHARE.tr.md)

# Sharing

> **Status: Done.** This is the design of Stage 4 work item 5 ([ROADMAP.md](ROADMAP.md) §6): a user of the web interface ([WEB.md](WEB.md)) gives a link to someone without an account, who can then see and download one version of a backup, or a folder or file in it, until the link ends. The decision behind it is ADR-030 ([DECISIONS.md](DECISIONS.md)).

Ayşe wants to send her accountant last month's invoices folder, or a relative the photos of one trip, without making them a user of the server and without copying the files somewhere else first.

## 1. What can be shared

| Rule | Detail |
| --- | --- |
| What | One version of a backup: all of it, one folder or one file in it |
| Who shares | An `admin` any version; a `member` only versions it can see (USERS.md §1); a `reader` nothing |
| What the link gives | Reading only: listing folders and downloading files below the shared path, without signing in |
| Which version | The one named when the link was made, by its id; later backups do not change what the link shows |

## 2. Links

A link looks like `https://<server>/s/<secret>`. The secret is 32 random bytes as 64 hex characters, so links cannot be guessed. The server keeps only its SHA-256 hash; the link is shown once, when it is made, and cannot be shown again.

A link works until the first of these:

| End | Detail |
| --- | --- |
| Its time is up | 1, 7 or 30 days, chosen when it is made; 7 by default, at most 30 |
| It is removed | By its owner or an `admin`, in the pages or with `apeiron web share remove` |
| Its owner may no longer share it | The owner was removed, became a `reader`, or no longer sees the version |
| Its downloads are used up | When a limit of 1 to 1000 downloads was chosen at making: each file or zip file sent through the link counts once it starts, even if it is cut short; looking at folders does not count |

A link can also ask for a password, of at least 8 characters, chosen when it is made; the owner sends it another way than the link. The server keeps only its Argon2id hash. Until the password is given, the link shows nothing but a password field. A right password sets the cookie `apeiron_share` for the link's own path (`HttpOnly`, `Secure`, `SameSite=Strict`, until the browser closes): a SHA-256 of the secret and the stored hash, which no one can make without the password, and which opens no other link. Wrong passwords pause like wrong sign-ins (WEB.md): after 5 in a row, every link's password waits a minute.

```mermaid
sequenceDiagram
    participant A as Ayşe (member)
    participant S as Server
    participant G as Guest
    A->>S: POST /api/shares {snapshot, path, days}
    S->>S: check the version is Ayşe's, keep the hash
    S-->>A: link with the secret, shown once
    A-->>G: sends the link
    G->>S: GET /api/shared/<secret>?path=…
    S->>S: hash matches, not ended, Ayşe still sees it
    S-->>G: folder listing or file
```

## 3. The shares file

Links are kept in `shares.json` in the state folder (`~/.local/state/apeiron` on Linux), readable only by its owner:

```json
{
  "shares": [
    {"id": "4661079506c34678", "hash": "4661079506c34678…", "owner": "ayse",
     "snapshot": "e63639f3…", "path": "invoices/2026-09",
     "made": "2026-10-30T10:30:55Z", "expires": "2026-11-06T10:30:55Z"}
  ]
}
```

| Field | Meaning |
| --- | --- |
| `id` | The first 16 characters of `hash`; names the link in lists and for removing it |
| `hash` | SHA-256 of the secret, hex |
| `owner` | The user who made the link |
| `snapshot` | The id of the shared version |
| `path` | The shared path inside it; empty for all of it |
| `made`, `expires` | UTC times |
| `max_downloads`, `downloads` | The limit, left out for a link without one, and the downloads counted so far; counted under the same lock, so two downloads cannot both take the last one |
| `protected`, `password` | Whether the link asks for a password, and its Argon2id hash as a PHC string; `password` is left out for a link without one |

The server and `apeiron web share` change the file under a lock (`shares.lock`) and replace it whole with a new file written and flushed beside it, so a crash leaves the old file or the new one. Ended links are dropped whenever the file changes. A damaged file is reported, never replaced.

## 4. What the link holder sees

Opening the link shows a page with the shared folder, its subfolders and a Download button for each file, and a button to download the folder shown with everything in it as one zip file; a shared file is shown alone. Files and zip files are sent as they are read, without a copy on the server's disk, so a large file does not fill the server's memory. Links inside the backup are listed but not followed.

Nothing above or beside the shared path can be reached: a path with `.` or `..` is refused. An unknown, ended or removed link, and one whose owner may no longer share it, all give the same "not found" answer. Pages answer with `Referrer-Policy: no-referrer`, so the link does not leave with a request to another site.

## 5. In the pages, the JSON API and the CLI

| Where | What |
| --- | --- |
| Share buttons while looking inside a version | Make a link to a file, a folder or the whole version; for an `admin` and a `member` |
| The Links page | The user's live links with their end; an `admin` sees everyone's; Remove |
| `POST /api/shares` `{"snapshot", "path", "days", "password", "downloads"}` | Makes a link, with a password and a limit of downloads when they are given; answers `{"share", "secret"}` |
| `GET /api/shares` | The user's live links, without hashes; all of them for an `admin` |
| `POST /api/shares/<id>/remove` | Removes a link of the user; any link for an `admin` |
| `GET /api/shared/<secret>?path=<path>` | Without signing in: the entry at the path below the link, a folder's entries, the version's time, the link's end and the downloads left, if limited |
| `GET /api/shared/<secret>/file?path=<path>` | Without signing in: the file at the path, as a download |
| `GET /api/shared/<secret>/zip?path=<path>` | Without signing in: the folder at the path with everything in it, as one zip file; recorded as `shared:read` |
| `POST /api/shared/<secret>/unlock` `{"password"}` | Without signing in: checks the link's password and sets its cookie; the two requests above answer `key` (401) for a link with a password until then |
| `apeiron web share list` | The live links of all users |
| `apeiron web share remove <id>` | Removes a link |

The desktop interface has no sharing: it serves only the person who runs it, on their own computer.

## 6. In the audit log

| Event | Detail |
| --- | --- |
| `share_add` | The link's id, version, path and end; on failure the request |
| `share_remove` | The link's id |
| `shared:list`, `shared:read` | The link's id and the path asked for; no user, the address of the connection |
| `shared:unlock` | The link's id and whether the password was right (`ok`, `key`, `locked`); never the password |
| `shared:upload` | The link's id and the file name given; no user, the address of the connection |

The secret is never recorded ([AUDIT.md](AUDIT.md)).

## 7. Upload links

The other way round: Ayşe asks her accountant for this year's papers, and the accountant sends them into one of her backups without an account.

| Rule | Detail |
| --- | --- |
| What | Files sent into a folder (default `uploads`) of one backup, the ref named when the link is made; the backup must exist |
| Who makes one | An `admin` for any backup, a `member` for its own (USERS.md §1); the Ask for files button beside a backup on the Overview page, or `POST /api/shares` `{"upload": "<ref>", "path", "days", "password", "downloads"}` |
| What the link gives | Sending files only: the link holder sees nothing of the backup, not even what is in the folder; the read requests of §5 answer "not found" |
| Each file | Becomes a new version of the backup whose parent is the newest one, with the message `upload <name> through link <id>`: everything the backup held stays, and a name already in the folder gets ` (2)`, ` (3)` … before its extension. Nothing is ever replaced or removed |
| Size | At most 1 GiB per file; files are read as they arrive and stored in chunks, never kept whole in memory or on the server's disk |
| Names | One file name: not empty, without `/`, `\` or control characters, not `.` or `..`, at most 255 bytes |

The link ends as a read link does (§2): time, removal, an owner that may no longer write the backup, or, when a limit was chosen, the number of files it took; a file counts once it starts. It can ask for a password the same way. In the shares file it has `upload` with the ref, `path` with the folder, and `snapshot` with the version the backup had when the link was made; `downloads` counts the files taken. A file taken is copied to the replicas, as a backup is (REPLICATION.md §4).

| Request | What |
| --- | --- |
| `GET /api/shared/<secret>` | `{"upload": true, "expires", "uploads_left", "max_size"}` |
| `POST /api/shared/<secret>/upload?name=<file name>` | The file's bytes as the body; answers the path it got and its size |

## 8. Not part of this work item

| Item | Where |
| --- | --- |
| Sharing with named users of the server | Not needed: give the user the backup in the users file (USERS.md §2) |

## 9. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-services` | `add_file`: a file added to a backup as a new version (section 7) | Done |
| `apeiron` | `Repository::read_file`: the content of a file in a version, as a stream ([API.md](API.md) §8.3); `Repository::add_file` | Done |
| `apeiron-server` | The shares file, the checks, `/api/shares`, `/api/shared`, the Links page, the link holder's page and the upload page | Done |
| `apeiron-cli` | `apeiron web share` | Done |
