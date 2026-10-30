> **Türkçe:** [REPORTS.tr.md](REPORTS.tr.md)

# Compliance Reports

> **Status: Done.** This is the design of Stage 5 work item 5 ([ROADMAP.md](ROADMAP.md) §7): one document that answers what an auditor asks about backups, made from the repository and the web server as they are, not from what someone remembers. The decision behind it is ADR-035 ([DECISIONS.md](DECISIONS.md)).

Standards and laws such as ISO 27001, SOC 2 and the GDPR ask an organization to show that its data is backed up, that backups are whole and kept apart, that they are protected and kept as long as required, and who had access. Collecting this by hand for each audit is slow and easy to get wrong.

## 1. What it answers

| Question | Where in the report |
| --- | --- |
| What is backed up, and when last? | Backups: every ref, its newest snapshot and its time; refs with no new snapshot for longer than `--stale-days` are marked stale |
| Is the stored data whole? | Repository: with `--verify`, every stored byte read and checked |
| Is there a second copy, and is it current? | Backups and Remotes: with `--remotes`, each ref compared with each remote |
| Is it protected? | Repository: encryption, passwords and recovery keys |
| Is it kept as long as required? | Backups and Retention rules ([ARCHIVE.md](ARCHIVE.md)) |
| Who can reach it, and what did they do? | Web server, with `--web`: users and their rights, the identity provider, the audit log's state and counts ([AUDIT.md](AUDIT.md)) |

## 2. Making one

```text
apeiron report --verify --remotes --web > report-2026-10.md
apeiron --json report --verify --remotes --web > report-2026-10.json
```

| Option | Meaning | Default |
| --- | --- | --- |
| `--verify` | Read and check every stored byte, as `apeiron verify`; takes as long | Off |
| `--remotes` | Compare every ref with each remote; the remotes must answer | Off |
| `--stale-days <n>` | Days without a new snapshot after which a ref is stale; 0 checks nothing | 7 |
| `--web` | Add the web server's part | Off |
| `--tenant <name>` | With `--web`: the tenant whose files to report on ([TENANTS.md](TENANTS.md)) | None |
| `--since <YYYY-MM-DD>` | With `--web`: count audit events from that day | 30 days ago |

The report reads only; it changes nothing. Its text form is Markdown, ready to keep or turn into a PDF; `--json` gives the same content for other programs. A scheduled job can make one every month ([DAEMON.md](DAEMON.md) runs commands only through its own jobs, so use the system's scheduler for this).

| Exit code | Meaning |
| --- | --- |
| 0 | No problem; there may be warnings |
| 3 | At least one problem |
| 4 | `--verify` found damaged data |

## 3. Contents

1. **Findings**, most serious first (§4).
2. **Repository**: format version, encryption, how many passwords and recovery keys, the result of `--verify`.
3. **Backups**: per ref the snapshot id, the time of the newest snapshot, the number of snapshots in its history, how many days a retention rule keeps it, and its state on each remote: `current`, `behind`, `missing`, or `other` when the remote has snapshots this copy does not.
4. **Retention rules**, with when each was added.
5. **Remotes**, and any that did not answer.
6. **Web server**, with `--web`: every user with role, how they sign in, backups and folders; the identity provider; whether the audit log's chain of hashes is whole, its number of events and the hash of the last one; and from `--since` on the number of sign-ins, failed sign-ins, backups and restores started, share links made and failed replications.

The snapshot ids are the proof: an id fixes every byte under it and all the history before it, so a later report, or `apeiron verify` on a replica, shows that the same data is still there. The hash of the last audit event does the same for the audit log: kept in the report, it shows later that no earlier event was changed (AUDIT.md §3).

## 4. Findings

| Level | When |
| --- | --- |
| Problem | `--verify` found damaged data; damaged files were found when the repository was opened; a remote did not answer; a ref is behind, missing or different on a remote; the audit log is not whole |
| Warning | A ref is stale; the repository is not encrypted; it is encrypted without a recovery key; it has no remote, so every backup exists in one place only; the audit log's oldest parts were removed (AUDIT.md §3) |

## 5. In the pages, the JSON API and the CLI

| Where | What |
| --- | --- |
| `apeiron report` | §2 |
| `report` (API.md §8) | Long operation with `verify`, `remotes` and `stale_days`; returns a `ComplianceReport` with everything of §3 but the web server's part |
| The Report page | For an `admin` in the web interface, and in the desktop interface: findings, repository and backups of §3, with the choice to verify and to compare with the remotes; the web server's part comes from `apeiron report --web` |

## 6. Not part of this work item

| Item | Where |
| --- | --- |
| Reports mapped to the clauses of a given standard | Later, with a pilot that names its standard; the facts of §3 are what such a mapping points to |
| Signing the report | Later; the snapshot ids and the audit hash it holds can already be checked |
| Reports across every tenant at once | Later; one report per tenant with `--tenant` until then |

## 7. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron` | `report`, `ComplianceReport` | Done |
| `apeiron-cli` | `apeiron report`, the web server's part, the Markdown form | Done |
