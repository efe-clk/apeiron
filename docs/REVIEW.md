> **Türkçe:** [REVIEW.tr.md](REVIEW.tr.md)

# Reviews

> **Status: Done.** A work item of [ROADMAP.md](ROADMAP.md) §8: someone asks for the changes on a branch to be merged, others comment and approve, and the approved version is merged. It works for every kind of file, not only code, as [VISION.md](VISION.md) asks. The decision behind it is ADR-047 ([DECISIONS.md](DECISIONS.md)).

Zeynep writes an article on the branch `draft/spring` ([BRANCH.md](BRANCH.md)). Before it goes into `main`, which her site is built from, her editor should read it and say yes.

## 1. What a review is

| Part | Detail |
| --- | --- |
| From | A ref with the changes, usually a branch |
| Into | The ref they would be merged into; `main` by default |
| Title and text | One line of 1 to 200 characters, and an optional text of up to 10000 |
| Steps | Opening, comments (on the whole or on one file), approvals, requests for changes, merging or closing; each with its author and time |
| Id | 8 random hex characters; any unique start of it works too |

A review does not copy the branch: it names it, so a new backup on the branch is part of the review at once.

## 2. Rules

| Rule | Detail |
| --- | --- |
| An approval is of one version | It names the snapshot the branch pointed to when it was given; after a new snapshot on the branch it no longer counts, and someone approves again |
| A request for changes stays | Until the same person approves; the latest verdict of each person counts |
| The author cannot approve | The name that opened the review cannot approve it |
| Ready to merge | Open, the branch still exists, at least one approval of what it holds now and no request for changes left |
| Merging | Merges exactly the approved snapshot as `apeiron merge` does ([BRANCH.md](BRANCH.md) §2). If the merge would make conflict copies, nothing changes and the review stays open: merge `into` into the branch first, then ask for an approval again |
| After merging or closing | Only comments are taken; a closed review cannot be opened again, open a new one |

Names are the ones given with `--author`, or the user of the operating system. In the web interface every step is the signed-in user's, a member or reader sees only reviews whose two refs are both theirs, and a reader cannot open or merge one ([USERS.md](USERS.md) §4). On one computer anyone who can write to the repository can write any name, as with the author of a backup; a review is a record of agreement between people who trust each other, not access control. Who may change a ref at all is decided by permissions ([USERS.md](USERS.md)) and retention rules ([ARCHIVE.md](ARCHIVE.md)).

## 3. Commands

| Command | What it does |
| --- | --- |
| `apeiron review open <from> [--into <ref>] --title <text> [--text <text>] [--author <name>]` | Opens a review and prints its id |
| `apeiron review list [--all]` | The open reviews, newest first; `--all` also merged and closed ones |
| `apeiron review show <id>` | Every step, who approved, who asked for changes and the files a merge would change in `into` |
| `apeiron review comment <id> <text> [--path <file>] [--author <name>]` | Adds a comment, on one file with `--path` |
| `apeiron review approve <id> [--text <text>] [--author <name>]` | Approves what the branch holds now |
| `apeiron review request-changes <id> --text <text> [--author <name>]` | Asks for changes |
| `apeiron review close <id> [--text <text>] [--author <name>]` | Closes it without merging |
| `apeiron review merge <id> [--message <text>] [--author <name>]` | Merges a review that is ready; exit code 1 with a message saying what is missing otherwise |

```text
$ apeiron review open draft/spring --title "Spring article" --author zeynep
Opened review 3f9a01c2: merge draft/spring into main
$ apeiron review request-changes 3f9a --text "The second part is long" --author editor
$ apeiron backup ~/articles --ref draft/spring
$ apeiron review approve 3f9a --author editor
$ apeiron review merge 3f9a --author zeynep
Merged review 3f9a01c2 into main: 7c2b04e1
```

The Reviews page of the desktop and web interfaces does the same with buttons ([DESKTOP.md](DESKTOP.md)). The same steps are the operations `reviews`, `review`, `open_review`, `note_review` and `merge_review` of the API ([API.md](API.md) §8.3) and the SDKs ([SDK.md](SDK.md)). The text plugin shows what changed in a text file ([TEXT.md](TEXT.md)); `apeiron diff --text main draft/spring` does that from the command line.

## 4. Where reviews are kept

Every step is one JSON file in the ref `apeiron/reviews`, named `<id>-<nanoseconds>-<random>.json`, with the fields `action`, `author`, `time` (Unix seconds) and, as the step needs, `text`, `path`, `snapshot`, `from`, `into` and `title`. A new step adds a file and never changes one, so two people writing at once both keep their steps: the ref moves with compare-and-swap and the second writer tries again on top of the first ([BRANCH.md](BRANCH.md) §2). The steps of a review are read with the opening first and then by file name, so every copy shows them in the same order.

Because they are in a ref, reviews are encrypted with the repository, copied by push and pull, and protected by retention rules and legal holds like any other ref. The merge itself also writes the operation log ([FORMAT.md](FORMAT.md) §9.5).

## 5. Not part of this work item

| Item | Where |
| --- | --- |
| Requiring an approval before a ref may change | Later; today a review is an agreement, and anyone who may change the ref can still merge without one |
| Comments on one line of a file | Later; a comment names a file |

## 6. Crates

| Crate | Part |
| --- | --- |
| `apeiron` | `review.rs`: steps, rules and merging |
| `apeiron-cli` | `apeiron review` |
| `apeiron-server` | The Reviews page and the checks of the web interface |
