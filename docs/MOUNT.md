> **Türkçe:** [MOUNT.tr.md](MOUNT.tr.md)

# Mounting a Repository

> **Status: Done on Linux and macOS.** `apeiron mount` shows every snapshot of a repository as a read-only folder, so that its files can be opened, searched with `grep` or compared with `diff` without restoring them first. The decision behind it is ADR-056 ([DECISIONS.md](DECISIONS.md)).

## 1. Command

```text
$ mkdir ~/backups
$ apeiron --repo /run/media/user/disk/apeiron mount ~/backups
Mounted at /home/user/backups; press Ctrl+C or unmount it to stop
```

`apeiron mount <folder>` mounts the repository at an empty folder and serves it until Ctrl+C, or until the folder is unmounted from outside (`fusermount3 -u <folder>` on Linux, `umount <folder>` on macOS). An encrypted repository asks for its password first, as every command does.

## 2. What it shows

```text
~/backups/backup/documents/latest/…
~/backups/backup/documents/2026-11-20T103000Z_3f9ae1c2/…
~/backups/sync/photos/latest/…
```

| Path | Content |
| --- | --- |
| The root and its folders | One folder per part of each ref name: the ref `backup/documents` is the folder `backup/documents` |
| A ref's folder | `latest`, the snapshot the ref points to now, and one folder for each snapshot of its history (by first parent), named by its time in UTC and the start of its id |
| A snapshot's folder | Its files, folders and symbolic links, with their recorded sizes, times and permission bits |

Refs under `apeiron/` and `folder/` belong to Apeiron itself and are not shown. A ref made or moved while the repository is mounted appears within a second. Everything is read-only: writing, renaming and deleting fail with "read-only file system", and permission bits are shown without their write bits. Files and folders belong to whoever looks at them, since the owners recorded at backup time may not exist on this computer.

## 3. How it works

The mount uses only the public API ([API.md](API.md)): the refs, the history of each ref and the listing of each folder come from `refs`, `log` and `list`, and a file is read from any offset with `FileReader::read_at`, so a program that reads a few bytes from the middle of a large file fetches only the chunks it needs. Every chunk is checked against its id as it is read, as in a restore. A snapshot never changes, so its listings are kept for as long as the repository is mounted and the system may keep its pages for an hour; the folders of refs are read again after a second.

## 4. Systems

| System | Needs | Status |
| --- | --- | --- |
| Linux | FUSE 3: the `fuse3` package, which gives `fusermount3` | Done |
| macOS | [macFUSE](https://macfuse.github.io) | Built and tested on Linux only; macOS needs macFUSE installed by the user |
| Windows | — | Not written: Windows has no FUSE, and Projected File System needs a separate layer (§5) |

## 5. Not part of this work item

| Item | Where |
| --- | --- |
| Windows, through Projected File System or WinFsp | Later; `apeiron restore` and the desktop interface's Browse page meanwhile |
| Writing into a mounted snapshot | Not planned: snapshots never change; back up a folder instead |
| Mounting a remote without a local repository | Later; clone or pull it first |

## 6. Crates and libraries

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-mount` | The folders of the mount and the FUSE file system | Done |
| `apeiron-cli` | `apeiron mount` | Done |

| Need | Library | Reason |
| --- | --- | --- |
| FUSE | `fuser` without libfuse | Pure Rust; it speaks the FUSE protocol itself and needs only `fusermount3` to mount, so nothing has to be built against libfuse |
