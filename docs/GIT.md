> **Türkçe:** [GIT.tr.md](GIT.tr.md)

# Apeiron and Git

> **Status: Done.** This is the design of Stage 3 work item 1 ([ROADMAP.md](ROADMAP.md) §5): importing a Git repository with its history and exporting it back. The decision behind it is ADR-021 ([DECISIONS.md](DECISIONS.md)).

Stage 3 ends when a real Git repository can be moved into Apeiron and back. "Back" is measured strictly: every commit and annotated tag exported again gets the same Git object id it had, so the history in Git is unchanged, signatures stay valid and `git fsck` finds nothing.

## 1. Commands

| Command | What it does | Status |
| --- | --- | --- |
| `apeiron git import <git-repo> [--prefix <name>]` | Stores every branch, tag and remote-tracking branch of a Git repository, with its whole history, as snapshots; refs become `<prefix>/heads/…`, `<prefix>/tags/…` and `<prefix>/remotes/…` (default prefix `git`). Run again, it stores only new commits | Done |
| `apeiron git export <git-repo> [--prefix <name>]` | Writes the snapshots of those refs as Git objects into a Git repository, created bare if it does not exist, and points its branches, tags and remote-tracking branches at them; exit code 3 if a commit got another id | Done |

Snapshots made from Git are ordinary snapshots: `apeiron log git/heads/main`, `apeiron diff`, `apeiron restore git/heads/main ~/checkout` and push to a remote all work on them. A snapshot made in Apeiron, such as a backup, becomes a new commit when its ref is under the prefix.

## 2. From Git objects to snapshots

| Git | Apeiron | What keeps it exact |
| --- | --- | --- |
| Blob | Blob, chunked as any file | The content |
| Tree | Tree, without times or owners | Entry names and kinds; the file mode in `mode` (section 2.1) |
| Commit | Snapshot with the commit's tree and its parents' snapshots | The raw headers and message in `meta` (section 2.2) |
| Annotated tag | Snapshot whose only parent is the tagged commit's snapshot, with that commit's tree | The raw headers and message in `meta` |
| Branch, lightweight tag | Ref | The name, escaped (section 3) |
| Submodule entry (`160000`) | Not in the tree; listed in the snapshot's `meta` | Path and commit id |

### 2.1 Tree entries

| Git mode | Apeiron entry |
| --- | --- |
| `100644`, `100755`, and the old `100664` | File; `mode` is the last three octal digits |
| `120000` | Symbolic link; the blob content is the target |
| `40000` | Folder |
| `160000` | Left out of the tree and listed in `git.gitlinks` (section 2.2); a folder that held only submodules stays as an empty folder |

Export writes a file mode back as `100` followed by `mode` in octal, so the old `100664` is kept. Git sorts a folder as if its name ended in `/`; export sorts that way, whatever order the Apeiron tree has.

### 2.2 Snapshot fields and `meta` keys

| Field or key | Content |
| --- | --- |
| `time` | The committer time (the tagger time for a tag) |
| `author` | `Name <email>` of the author (of the tagger for a tag) |
| `message` | The message, when it is valid UTF-8 |
| `git.id` | The Git object id, 40 hex digits; export checks it was reproduced |
| `git.kind` | `commit` or `tag` |
| `git.headers` | Every header line after `tree` and `parent` (after `object` for a tag) exactly as stored: author, committer, encoding, signatures and any other. When they are not valid UTF-8 the key is `git.headers-hex` with the bytes in hex |
| `git.message-hex` | The message bytes in hex, when they are not valid UTF-8 |
| `git.no-message` | `1` when the object has no blank line and no message at all |
| `git.gitlinks` | One line per submodule entry: the path in hex, a space, the commit id |

`meta` is free for services to use (FORMAT.md §6.4); these keys need no change to the format.

## 3. Ref names

Git allows names that Apeiron ref names do not (FORMAT.md §9.3), such as capitals in `feature/ABC-123`. Each part of a Git name is escaped: `a-z`, `0-9`, `.` and `-` stay, a `.` at the start of a part and every other byte become `_` and two lowercase hex digits. `feature/ABC-123` becomes `git/heads/feature/_41_42_43-123`; export reverses it. A name longer than 200 bytes after escaping is skipped with a warning.

Only `refs/heads/`, `refs/tags/` and `refs/remotes/` are imported; in a clone most branches are remote-tracking ones. `HEAD`, symbolic refs such as `origin/HEAD`, notes and stashes are left out and listed. A branch deleted in Git keeps its Apeiron ref; the import reports it as stale.

## 4. Import

1. Read every branch, tag and remote-tracking branch. A tag that points to a tree or a blob, or a chain of tags, is skipped with a warning.
2. Find what is already stored: walk the snapshots of the existing refs under the prefix and read their `git.id`.
3. Store the commits that are missing, parents before children. Blobs and trees met before are not read again within one run.
4. Move each ref to its snapshot with compare-and-swap. A Git branch that moved backwards (force-pushed) moves the ref too; the reflog keeps the old snapshot.

Objects are written first and refs last (FORMAT.md §9.2), so an interrupted import leaves no ref pointing to anything incomplete, and the next run finds the commits it already stored.

## 5. Export

1. Open the Git repository, or create a bare one; a new repository's `HEAD` points to `main`, else `master`, else the first branch.
2. For each ref under the prefix, write its snapshots as commits or tags, parents first. A snapshot whose `git.id` is already in the Git repository is not written again.
3. Rebuild every object from its fields; a commit with `git.id` must get that id. One that does not is still written, and reported as inexact.
4. Point each branch and tag at its object.

A snapshot without Git fields becomes a commit with the snapshot's author (or `Apeiron <apeiron@localhost>`), its time as author and committer time in UTC, and its message. Export writes loose objects; `git gc` packs them.

## 6. Limits

| Case | What happens |
| --- | --- |
| SHA-256 repositories | Refused; Apeiron reads and writes SHA-1 repositories |
| Shallow clones | Refused: the missing parents cannot be stored |
| Trees with entries in a wrong order, as old broken tools wrote | Imported; the export is reported as inexact |
| Submodule content | Not imported; the submodule's own repository can be imported separately |
| Git LFS | The pointer files are imported, not the large files |

## 7. Not part of this work item

| Item | Where |
| --- | --- |
| The Git network protocol (clone and push over SSH or HTTP) | Later in Stage 3; a goal, not a requirement (VISION.md) |
| Checking out into a Git working tree | `apeiron restore` writes a snapshot into a folder |
| Branching and merging inside Apeiron | Stage 3 work item 2 |
| Moving the code behind the plugin interface | With the plugin system, Stage 3 |

## 8. Crates and libraries

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-git` | Import and export; only it uses `gix` | Done |
| `apeiron` | `git_import` and `git_export` jobs (API.md §8.3) | Done |
| `apeiron-cli` | `apeiron git import` and `apeiron git export` | Done |

`gix` (gitoxide) 0.85 reads and writes the objects and refs (VISION.md); it is the newest release that builds with Rust 1.85. Objects are parsed and written as raw bytes, so nothing gix normalizes can change an id.

The tests build a repository with `git` that has a merge, a branch with capitals, an executable, a symbolic link, a submodule entry, both kinds of tag, a signature header and a Latin-1 message, and check that every id comes back. Real repositories came back the same way: one of 3,568 commits; one of 4,265 commits and 13 tags; and one of 27,156 commits and 70 tags, imported in 22 seconds and exported in 19.
