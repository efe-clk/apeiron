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
| `--all-tenants` | One report for each tenant, each on the tenant's own repository (opened with its password file) and, with `--web`, its own files; the reports follow one another, separated by `---`, or form a JSON array with a `tenant` field each. The exit code is the worst of them | Off |
| `--since <YYYY-MM-DD>` | With `--web`: count audit events from that day | 30 days ago |
| `--standard iso27001`, `--standard soc2` | Add the facts mapped to the controls of a standard (§3.1) | None |

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
4. **Retention rules**, with when each was added, and **legal holds**, with their reasons and, once lifted, the lift (ARCHIVE.md §2.1).
5. **Remotes**, and any that did not answer.
6. **Web server**, with `--web`: every user with role, how they sign in (and whether with an authenticator code), backups and folders; the identity provider; whether the audit log's chain of hashes is whole, its number of events and the hash of the last one; and from `--since` on the number of sign-ins, failed sign-ins, backups and restores started, share links made and failed replications.

The snapshot ids are the proof: an id fixes every byte under it and all the history before it, so a later report, or `apeiron verify` on a replica, shows that the same data is still there. The hash of the last audit event does the same for the audit log: kept in the report, it shows later that no earlier event was changed (AUDIT.md §3).

### 3.1 Mapped to a standard

With `--standard iso27001` the report ends with the controls of ISO/IEC 27001:2022, Annex A, that a backup system bears on, each with what the report shows: `evidence` when its facts support the control, `gap` when they show something the control asks for is missing, and `not covered` when the report holds no facts on it, with the option that adds them. In JSON the section is `standard`, with `id`, `title`, `status` and `facts` per control. The mapping points an auditor to evidence; it is not a certification, and a control may need more than a backup system shows.

| Control | Evidence | Gap |
| --- | --- | --- |
| A.5.15 Access control | With `--web`: the users and their roles | — |
| A.5.17 Authentication information | With `--web`: password hashing, identity provider and authenticator codes | An admin who signs in with a password alone |
| A.5.30 ICT readiness for business continuity | With `--remotes`: every backup current on every remote | No remote, a copy behind or missing, a remote that did not answer |
| A.5.33 Protection of records | Retention rules or legal holds | Neither |
| A.8.13 Information backup | Backups, none stale, and with `--verify` the data checked | No backups, a stale one, damaged data |
| A.8.15 Logging | With `--web`: an audit log with a whole chain of hashes | A broken chain |
| A.8.24 Use of cryptography | An encrypted repository | Not encrypted |

With `--standard soc2` the same facts appear under the 2017 Trust Services Criteria of SOC 2, so the two reports cannot disagree. The titles are short ones of our own. The table was made from secondary descriptions of the criteria, not checked against the AICPA's text; CC6.7 and C1.1 are the least direct, and an auditor should confirm them.

| Criterion | Same facts as |
| --- | --- |
| CC6.1 Logical access security | A.5.17 |
| CC6.3 Access based on roles | A.5.15 |
| CC6.7 Protecting data that moves | A.8.24 |
| CC7.2 Monitoring | A.8.15 |
| CC9.1 Mitigating business disruption | A.5.30 |
| A1.2 Data backup and recovery | A.8.13 |
| C1.1 Keeping confidential information | A.5.33 |

## 4. Findings

| Level | When |
| --- | --- |
| Problem | `--verify` found damaged data; damaged files were found when the repository was opened; a remote did not answer; a ref is behind, missing or different on a remote; the audit log is not whole |
| Warning | A ref is stale; the repository is not encrypted; it is encrypted without a recovery key; it has no remote, so every backup exists in one place only; the audit log's oldest parts were removed (AUDIT.md §3) |

In JSON each finding has `level`, an English `text`, and a `code` with its facts, so a program or a page can say it in another language: `damaged` (`files`, `objects`), `damaged_on_open` (`files`), `remote_down` (`remote`, `message`), `copy` (`name`, `remote`, `state`), `stale` (`name`, `days`), `not_encrypted`, `no_recovery_key`, `no_remote`, `audit_trimmed` (`first`), `audit_broken` (`message`).

## 5. In the pages, the JSON API and the CLI

| Where | What |
| --- | --- |
| `apeiron report` | §2 |
| `report` (API.md §8) | Long operation with `verify`, `remotes` and `stale_days`; returns a `ComplianceReport` with everything of §3 but the web server's part |
| The Report page | For an `admin` in the web interface, and in the desktop interface: findings, repository and backups of §3, with the choice to verify and to compare with the remotes; the web server's part comes from `apeiron report --web` |

### 5.1 Signing a report

A signed report shows the auditor that it is the one the admin made, unchanged since:

```text
apeiron report key ~/.config/apeiron/report.key
apeiron report --verify --remotes --web --out report-2026-10.md --sign ~/.config/apeiron/report.key
apeiron report check report-2026-10.md --public-key 3b6a…
```

| Command | What it does |
| --- | --- |
| `apeiron report key <file>` | Makes an Ed25519 key in a new file readable by its owner only, and prints its public key to give the auditor |
| `apeiron report … --out <file> [--sign <key>]` | Writes the report, Markdown or with `--json` JSON, to the file instead of printing it; with `--sign` also writes `<file>.sig`: `{"algorithm": "ed25519", "public_key", "signature"}`, the signature of the file's bytes, both hex |
| `apeiron report check <file> [--public-key <hex>]` | Checks the signature in `<file>.sig`, and that it is by the given key; exit code 4 when the report was changed or signed by another key |

The signature covers the report's bytes exactly, so even a changed space breaks it; a report is checked as written, not after it was turned into a PDF. Anyone with the key file can sign, so it is kept like a password; the public key in the `.sig` file only says which key signed, and the auditor checks it against the key the admin gave them.

## 6. Not part of this work item

| Item | Where |
| --- | --- |
| Mappings to other standards, such as the GDPR | Later, with a pilot that names its standard; §3.1 shows the form |

## 7. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron` | `report`, `ComplianceReport` | Done |
| `apeiron-cli` | `apeiron report`, the web server's part, the Markdown form | Done |
