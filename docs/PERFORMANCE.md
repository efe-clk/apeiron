> **Türkçe:** [PERFORMANCE.tr.md](PERFORMANCE.tr.md)

# Performance Measurements

> **Status: In progress.** The targets of [VISION.md](VISION.md) are written for a Raspberry Pi 4. These numbers come from a much stronger computer, so they show what the program does, not that the targets are met on a Raspberry Pi 4. That measurement is still to be made ([ROADMAP.md](ROADMAP.md) §8).

## 1. How to measure

```bash
cargo install --locked --path crates/apeiron-cli
scripts/measure.py ~/.cargo/bin/apeiron /an/empty/folder
```

The script makes its own test data, runs each scenario with the installed program and prints a table. It uses its own home folder inside the given folder, needs about three times `--big-gb` (default 10) of free disk and does not remove the folder. `--files` changes the number of small files (default 100000).

## 2. Results

Measured on 2026-11-06 on Linux with 20 logical processors, 16 GB of memory and an NVMe disk, with random data, which cannot be compressed or deduplicated.

| Scenario | Target on a Raspberry Pi 4 | Before | Now | Peak memory now |
| --- | --- | --- | --- | --- |
| First backup of 100000 files of 16 KiB | — | 23.7 s | 20.9 s | 91 MiB |
| Backup of the same files, unchanged | Under 10 s | 20.4 s | 0.5 s | 134 MiB |
| Reading the history of that backup | Under 2 s | 0.1 s | 0.0 s | 86 MiB |
| Finding a file by name among them | Under 2 s | 0.1 s | 0.1 s | 86 MiB |
| Adding one file of 10 GiB | Under 256 MiB of memory | 21.0 s | 20.6 s | 134 MiB |
| Backup of that file, unchanged | — | 11.1 s | 0.1 s | 121 MiB |
| Restoring that file | — | 10.4 s | 10.5 s | 82 MiB |
| The background service, idle | Under 50 MiB of memory | — | — | 9 MiB |
| Sent again after a push of 10 GiB stopped half way | At most one pack, about 32 MiB | 10240 MiB | What was left: 5101 MiB, with 5139 MiB already on the copy | 95 MiB |

Apeiron has no `status` command; the status query of the vision is met by reading the history and by `sync --dry-run`, so the table measures those reads.

## 3. What the measurements changed

| Finding | Change |
| --- | --- |
| An unchanged backup read every file | A file whose size and time equal the last snapshot, and older than it, is not read ([CLI.md](CLI.md) §4); `--reread` reads all |
| A stopped push started again from nothing, as the remote's index was written only at the end | An index follows each pack, and objects are sent after what they refer to ([FORMAT.md](FORMAT.md) §15.5) |
| A stopped pull could stop for good on the next run, as a pack without an index was trusted | A pull relies only on what indexes list ([FORMAT.md](FORMAT.md) §15.5) |
| A transfer cannot resume inside a pack, so the vision's "at most 1 chunk" cannot hold | The target is now one pack ([DECISIONS.md](DECISIONS.md) ADR-041) |
