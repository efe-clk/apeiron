> **Türkçe:** [ARCHIVE.tr.md](ARCHIVE.tr.md)

# Immutable Archive

> **Status: Done.** This is the design of Stage 5 work item 4 ([ROADMAP.md](ROADMAP.md) §7): backups kept as an archive that nobody can change or delete before the time an organization must keep them, not by mistake, not an admin, and not ransomware that has taken over an admin's account. The decision behind it is ADR-034 ([DECISIONS.md](DECISIONS.md)).

Laws and contracts often say how long records must be kept: invoices for ten years, health records longer. An auditor then asks for proof that what is shown is what was stored. Ransomware, in turn, looks for the backups first and deletes them.

## 1. Two layers

| Layer | Protects against | How |
| --- | --- | --- |
| Retention rules (§2) | Mistakes, users and admins of Apeiron, programs using its API | Apeiron itself refuses to move a kept ref back or to delete it before its time |
| Storage that cannot be changed (§4) | Someone with access to the files or to the storage account | An S3 bucket with Object Lock refuses to delete or overwrite stored data, whoever asks |

The first layer works everywhere and costs nothing; the second needs storage that supports it. A copy kept under both, for example a replica ([REPLICATION.md](REPLICATION.md)) in a bucket with Object Lock, can be changed by no one before its time.

## 2. Retention rules

A rule names refs, as a name or a prefix followed by `*`, and a number of days:

```text
apeiron retention add "archive/*" --days 3650
apeiron backup /srv/invoices --ref archive/invoices
```

For every ref a rule matches ([FORMAT.md](FORMAT.md) §16):

| Change | Allowed |
| --- | --- |
| A new backup on the ref, or a merge into it | Yes: the history grows |
| Moving the ref back to an older snapshot, or to another history | Never |
| Deleting the ref | Only when the days have passed since its last change |
| Creating a new ref that matches | Yes |

Since a kept ref only grows, every snapshot it ever pointed to stays reachable, and garbage collection keeps all of it. A rule cannot be removed or shortened, through the CLI, the API or the web interface; a longer rule can be added beside it, and the longest one counts. When several rules match a ref, the longest one applies. A refused change fails with `conflict` and says how many days are left.

Rules travel with the repository: a push writes them to the remote and a pull or clone brings them back, so a replica keeps the same refs (FORMAT.md §16.3).

## 3. Proving what was stored

Nothing more is needed for the proof than what every repository has: each object is named by its hash and each snapshot names its tree and its parents ([FORMAT.md](FORMAT.md) §4), so the id of a snapshot fixes every byte of every file under it and all the history before it. `apeiron verify` reads every stored byte and checks it against these hashes. An auditor given a snapshot id from the time it was made can check that the archive still holds exactly that.

## 4. Storage that cannot be changed

Apeiron's rules bind Apeiron. Someone who can write to the repository's folder, or to the storage account, can still delete the files. An S3 bucket with Object Lock in compliance mode closes that gap: S3 itself refuses to delete or overwrite any stored version before its retention ends, even for the account's owner.

1. Create the bucket with Object Lock and a default retention in compliance mode, at least as long as the rules of §2.
2. Add it as a remote of an encrypted repository: `apeiron remote add vault "s3://bucket/apeiron?endpoint=…&region=…"`.
3. Keep it current with `apeiron push --all vault`, a scheduled job or `apeiron web --replicate vault` ([REPLICATION.md](REPLICATION.md)).

Every object Apeiron writes to S3 carries a SHA-256 checksum, which buckets with Object Lock require and which guards the data on its way. Packs and index files are only ever written, never changed, so each is kept as it was first stored. A ref that moves, or a file deleted by a push, becomes a new version or a delete marker: the earlier versions stay locked, and the storage's admin can bring them back.

## 5. In the JSON API and the CLI

| Where | What |
| --- | --- |
| `apeiron retention add <refs> --days <n>` | Adds a rule; it cannot be undone |
| `apeiron retention list` | Lists the rules |
| `retention`, `add_retention` (API.md §8) | The same for programs |
| Deleting or moving a kept ref back, by any program | Refused with `conflict` and the days left |

## 6. Not part of this work item

| Item | Where |
| --- | --- |
| Legal holds that keep a ref beyond its rule until lifted | Later, if a pilot needs one; a lifted hold is exactly what compliance rules forbid, so it needs its own design |
| Adding rules in the web interface | Later; the CLI until then |
| Object Lock for SFTP or folder remotes | Not possible in Apeiron; such storage needs its own immutable snapshots, such as those of ZFS or a backup appliance |
| Deleting single old snapshots from the middle of a kept history | Not planned: a history that can lose its middle is not an archive |

## 7. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-core` | Rule files, the checks of every ref change, copying rules with push and pull | Done |
| `apeiron` | `retention`, `add_retention` | Done |
| `apeiron-remote` | SHA-256 checksums on every S3 write | Done |
| `apeiron-cli` | `apeiron retention` | Done |
