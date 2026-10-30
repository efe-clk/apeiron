> **Türkçe:** [USERS.tr.md](USERS.tr.md)

# Users and Permissions

> **Status: Done.** This is the design of Stage 4 work item 3 ([ROADMAP.md](ROADMAP.md) §6): several people sign in to the web interface ([WEB.md](WEB.md)), each with a name, a password, a role, the backups they may see and the folders of the server they may use. The decision behind it is ADR-028 ([DECISIONS.md](DECISIONS.md)).

A family or a small office shares one backup server: each person backs up and restores their own folders and sees only their own backups, and one of them looks after the whole server.

## 1. Roles

| Right | `admin` | `member` | `reader` |
| --- | --- | --- | --- |
| See backups and their versions, look inside, compare, search | All | Those of its backups | Those of its backups |
| Back up a folder | Into any backup | Into its backups | No |
| Restore | Yes | Yes | No |
| Check the repository | Yes | No | No |
| Share with a link ([SHARE.md](SHARE.md)) | Any version | Versions it sees | No |
| Folders of the server | Its folders, or all `--folder` folders when it has none | Its folders only | None |

A user's **backups** are a list of ref names; a name ending in `*` stands for every ref that starts with what comes before it, so `backup/ayse-*` covers `backup/ayse-photos` and `backup/ayse-documents`. A user's **folders** are folders of the server; only those inside a `--folder` of `apeiron web` count.

A version is visible to a user when it can be reached from the newest version of one of the user's backups through its parents. A backup made by merging another one therefore also shows the versions it took in.

## 2. The users file

Users are kept in `web.json` in the configuration folder, readable only by its owner:

```json
{
  "users": [
    {"name": "efe", "password": "$argon2id$v=19$...", "role": "admin"},
    {"name": "ayse", "password": "$argon2id$v=19$...", "role": "member",
     "backups": ["backup/ayse-*"], "folders": ["/home/ayse"]}
  ]
}
```

| Field | Meaning |
| --- | --- |
| `name` | 1 to 32 lowercase letters, digits, `-` and `_` |
| `password` | Argon2id hash of the password, at least 8 characters long; may be left out for a user with `oidc` |
| `oidc` | The user's identity at the organization's identity provider ([IDENTITY.md](IDENTITY.md)) |
| `role` | `admin`, `member` or `reader` |
| `backups` | Ref names, with `*` at the end for a prefix; ignored for `admin` |
| `folders` | Folders of the server |

The server reads the file again when it changes, so a user added, changed or removed while it runs counts from the next request. A session ends when its user is removed or its password changes.

## 3. Commands

| Command | What it does |
| --- | --- |
| `apeiron web user add <name> --role <role> [--backup <ref>]... [--folder <path>]...` | Adds a user; asks for the password twice |
| `apeiron web user change <name> [--role <role>] [--backup <ref>]... [--folder <path>]...` | Replaces the role, backups or folders given |
| `apeiron web user password <name>` | Sets a new password; ends the user's sessions |
| `apeiron web user remove <name>` | Removes a user; the last `admin` cannot be removed |
| `apeiron web user list` | Lists users with their role, backups and folders |

`apeiron web` does not start without at least one `admin`.

## 4. In the pages and the JSON API

| Request | Change |
| --- | --- |
| `POST /api/login` `{"name", "password"}` | Signs in as the user; a wrong name and a wrong password give the same error, after the same work |
| `GET /api/session` | Adds `"user": {"name", "role"}` when signed in |
| `POST /api/call/refs` | Only the refs of the user's backups |
| Requests naming a snapshot (`spec`, `start`, `snapshot`, `from`, `to`) | The snapshot must be visible to the user (§1); it is replaced by its id, so what was checked is what is used |
| `POST /api/call/reflog` | Only for refs of the user's backups |
| `POST /api/start/search` | A `member` or `reader` must name a snapshot |
| `GET /api/folders` | Only inside the user's folders |
| Anything else beyond the role | Refused with `unsupported`; a backup into another ref or a path outside the folders with `invalid_input` |
| `GET /api/users` | For an `admin`: every user without the password's hash, whether it has a password, and the folders users may be given |
| `POST /api/users` `{"name", "role", "password", "oidc", "backups", "folders"}` | For an `admin`: adds a user, with the checks of §3; folders must be inside a `--folder` |
| `POST /api/users/<name>` | For an `admin`: changes the fields sent; an empty `password` keeps it, `"no_password": true` takes it away. An admin cannot take its own admin role away, and there is always an admin |
| `POST /api/users/<name>/remove` | For an `admin`: removes a user other than itself |

The pages show the user's name next to Sign out and hide what the role cannot do: Back up for a `reader`, Restore for a `reader`, Check for everyone but an `admin`. An `admin` also has a Users page, which adds, changes and removes users with the requests above; every change is recorded in the audit log under that admin's name, and a changed password or identity ends the user's sessions. Changes of the pages and of `apeiron web user` are written whole, one at a time (`web.json.lock`).

## 5. Not part of this work item

| Item | Where |
| --- | --- |
| A record of who did what | Work item 4 ([AUDIT.md](AUDIT.md)) |
| Separate encryption keys per user | Users share the repository's key; the server holds it |
| Users in the desktop interface | Not needed: it serves only the user who runs it |
| Central identity | Stage 5 work item 1 ([IDENTITY.md](IDENTITY.md)) |
| Several organizations on one server, kept apart | Stage 5 work item 2 ([TENANTS.md](TENANTS.md)) |

## 6. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-server` | Users, sessions per user and the checks of §4 | Done |
| `apeiron-cli` | `apeiron web user` | Done |
