> **Türkçe:** [CONSISTENT.tr.md](CONSISTENT.tr.md)

# Application-Consistent Backups

> **Status: Done.** A backup of a running database or a busy disk can catch files halfway through a change. `apeiron backup --source` backs up a database dump, a read-only snapshot of a file system or the content of a container volume instead, each made at one moment by the system's own program. The decision behind it is ADR-059 ([DECISIONS.md](DECISIONS.md)).

## 1. Command

```bash
apeiron backup --source postgres:shop                 # into postgres/shop
apeiron backup --source btrfs:/srv/photos --ref photos
apeiron backup --source docker:pgdata --pause
```

`apeiron backup --source <kind>:<target> [--ref <name>] [--pause]` takes the place of a folder. Without `--ref` the snapshot goes to `<kind>/<name>`, such as `postgres/shop`. A job of the background service does the same with `source = "<kind>:<target>"` instead of `folder`, and `pause = true` ([DAEMON.md](DAEMON.md) §3); a tenant's jobs cannot, since they would run programs on the server. Whatever was made for the backup is removed after it, also when it fails.

## 2. Sources

| Source | What is backed up | Made by | Needs | Tried |
| --- | --- | --- | --- | --- |
| `postgres:<database or connection>` | A plain SQL dump, `<database>.sql` | `pg_dump`, which reads one snapshot of the database | The password from `PGPASSWORD` or `~/.pgpass`; `pg_dump` at least as new as the server | Yes, PostgreSQL 17 |
| `mysql:<database>` | A plain SQL dump, `<database>.sql` | `mysqldump` or `mariadb-dump` with `--single-transaction` | The password from `~/.my.cnf` or `MYSQL_PWD` | No |
| `btrfs:<subvolume>` | A read-only snapshot, made beside the subvolume | `btrfs subvolume snapshot -r` | A subvolume; deleting the snapshot needs root, or else it is made writable and removed by its owner | Yes |
| `zfs:<dataset>` | The snapshot, read through `.zfs/snapshot` | `zfs snapshot`, `zfs destroy` | Rights on the dataset (`zfs allow`) or root | No |
| `lvm:<group>/<volume>` | A snapshot volume of a tenth of the origin, mounted read-only | `lvcreate --snapshot`, `mount`, `lvremove` | Root and room in the volume group | No |
| `docker:<volume>` | The volume as a tar archive, imported (CLI.md §4.2) | A `busybox` container that mounts it read-only | Docker; `--pause` pauses the containers using the volume while it is read | Yes |
| `kubernetes:<namespace>/<claim>` | The persistent volume as a tar archive, imported | A helper pod with the claim mounted read-only, removed afterwards | `kubectl` with rights to run pods; a claim of one node can be read only where that node can run the pod | No |
| `vss:<folder>` | The folder in a Volume Shadow Copy of its drive | Windows' shadow copy service through PowerShell | Administrator rights | No |

A dump of plain SQL changes little from day to day, so its chunks deduplicate (FORMAT.md §7); a compressed dump would be new each time. Dumps and archives are written into the repository's `tmp/` folder, which needs room for one of them.

## 3. Not part of this work item

| Item | Where |
| --- | --- |
| Restoring into a database | Restore the dump and load it with `psql` or `mysql`, as for any dump |
| Other databases, such as MongoDB | Later; a job can back up the folder a dump program writes |
| Trying the sources marked "No" on real systems | With the first users of each |

## 4. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-daemon` | `sources`: preparing and removing each source | Done |
| `apeiron-cli` | `apeiron backup --source` | Done |
