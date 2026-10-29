> **Türkçe:** [DECISIONS.tr.md](DECISIONS.tr.md)

# Apeiron Architecture Decision Records

This document records every technical decision made so far, in ADR (architecture decision record) form: context, decision, rationale, alternatives considered and consequences. A decision is changed only by adding a new ADR that supersedes it; old records are not rewritten.

## Index

| ADR | Topic | Decision | Status |
| --- | --- | --- | --- |
| [001](#adr-001-rust-as-the-main-language) | Language | Rust wherever possible; C only for the ABI | Accepted |
| [002](#adr-002-gitoxide-for-git-compatibility) | Git compatibility | gitoxide (`gix`) | Accepted |
| [003](#adr-003-blake3-as-the-hash-algorithm) | Hash algorithm | BLAKE3 | Accepted |
| [004](#adr-004-fastcdc-for-chunking) | Chunking | FastCDC, average 1 MiB | Accepted |
| [005](#adr-005-wasmtime-and-the-webassembly-component-model-for-plugins) | Plugin runtime | WebAssembly component model, Wasmtime | Accepted |
| [006](#adr-006-tokio-outside-the-core-only) | Async runtime | Tokio, outside the core only | Accepted |
| [007](#adr-007-personal-backup-and-home-server-as-the-first-target-user) | First target user | Personal backup and home server | Accepted |
| [008](#adr-008-dual-licence-mit-or-apache-20) | Licence | MIT OR Apache-2.0 | Accepted |
| [009](#adr-009-keyed-blake3-for-object-identifiers) | Object identifiers | Keyed BLAKE3 with a per-repository key | Accepted |
| [010](#adr-010-pack-files-instead-of-one-file-per-object) | Object storage | Pack files with separate index files | Accepted |
| [011](#adr-011-zstd-for-compression) | Compression | zstd | Accepted |
| [012](#adr-012-canonical-cbor-for-structured-objects) | Structured objects | Canonical CBOR | Accepted |
| [013](#adr-013-lowercase-ascii-ref-names) | Ref names | Lowercase ASCII only | Accepted |
| [014](#adr-014-a-small-system-crate-for-unsafe-os-calls) | `unsafe` code | Only in the FFI crate and the small `apeiron-sys` crate | Accepted |
| [015](#adr-015-json-values-at-the-c-abi-boundary) | C ABI | A small fixed set of functions; requests and results as JSON | Proposed |
| [016](#adr-016-synchronous-public-api-with-jobs-on-threads) | Long operations | Synchronous API; jobs on threads with polled progress | Proposed |
| [017](#adr-017-xchacha20-poly1305-and-argon2id-for-encryption) | Encryption | XChaCha20-Poly1305 for data, Argon2id for passwords | Accepted |
| [018](#adr-018-whole-repository-copies-as-remotes-russh-for-sftp-object_store-for-s3) | Remote targets | Remotes are whole repository copies; `russh` for SFTP, `object_store` for S3 | Accepted |
| [019](#adr-019-the-background-service-is-a-user-process-with-a-jobs-file) | Background service | A user process with a TOML jobs file, started by the operating system; missed jobs catch up | Accepted |
| [020](#adr-020-sync-through-a-remote-with-a-three-way-merge-and-conflict-copies) | Sync | Devices meet at a remote; a three-way merge per path; conflicting versions are kept as copies | Accepted |
| [021](#adr-021-git-import-and-export-with-exact-object-ids-in-a-crate-of-its-own) | Git | `apeiron-git` with `gix`; raw headers in `meta` so exported commits keep their ids | Accepted |

ADR 001–008 were made before Stage 0 began ([VISION.md](VISION.md)). ADR 009–013 were approved together with the storage format on 2026-10-27 ([FORMAT.md](FORMAT.md)). ADR-014 was approved on 2026-10-28.

## ADR-001: Rust as the main language

**Status:** Accepted

**Context.** Apeiron handles other people's data, runs on devices as small as a Raspberry Pi 4 and must be embeddable in applications written in other languages. Memory-safety bugs in storage code can silently corrupt data.

**Decision.** Apeiron is written in Rust wherever possible. C is used only in the outward-facing ABI layer. `unsafe` is allowed only in the FFI crate.

**Rationale.** Rust is compiled and fast, prevents whole classes of memory errors at compile time, has no garbage-collector pauses, and can expose a C ABI for SDKs in other languages.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| C or C++ | Memory-safety bugs are a direct risk to data integrity |
| Go | Garbage collector and runtime make embedding through a C ABI harder and memory use less predictable on small devices |
| Zig | Promising, but the language and ecosystem are not yet stable enough for a long-lived storage format |

**Consequences.** Contributors need Rust. Some C libraries may still be used through bindings where no mature Rust alternative exists (see ADR-011). Platform calls that need `unsafe` live in the `apeiron-sys` crate (ADR-014).

## ADR-002: gitoxide for Git compatibility

**Status:** Accepted

**Context.** Developers must be able to move Git repositories into Apeiron and back. Git import/export is mandatory; the Git network protocol (SSH, smart HTTP) is a goal but not a requirement.

**Decision.** Git compatibility uses gitoxide (`gix`). The Git protocol is not written from scratch. Git support lives in a plugin or service, never in the core.

**Rationale.** gitoxide is a pure-Rust Git implementation, which fits ADR-001 and avoids a C dependency.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| libgit2 (`git2` crate) | A C library; conflicts with ADR-001 |
| Calling the `git` command | Requires Git to be installed and parsing text output; fragile |
| Writing our own Git implementation | Large and error-prone; not the purpose of the project |

**Consequences.** Git feature coverage depends on gitoxide's progress, especially for the network protocol. Work is scheduled for Stage 3.

## ADR-003: BLAKE3 as the hash algorithm

**Status:** Accepted

**Context.** Every object is addressed and verified by a cryptographic hash. Hashing runs over every byte that is written or read, including on a Raspberry Pi.

**Decision.** BLAKE3 (`blake3` crate) is used for all hashing.

**Rationale.** BLAKE3 is a cryptographic hash, parallel and very fast, efficient on ARM as well, and has an official Rust implementation. It also has a keyed mode (used by ADR-009).

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| SHA-256 | Secure, but considerably slower without hardware acceleration |
| SHA-1 (as in Git) | Broken for collision resistance |
| xxHash and other non-cryptographic hashes | Fast, but do not protect against deliberate collisions |

**Consequences.** Git objects (SHA-1/SHA-256) are not reused as Apeiron identifiers; Git import converts them.

## ADR-004: FastCDC for chunking

**Status:** Accepted

**Context.** Files must be split so that a small change rewrites only a few chunks. This drives deduplication, incremental backups and resumable transfers.

**Decision.** FastCDC (2020 version) with an average chunk size of 1 MiB, minimum 256 KiB and maximum 4 MiB. The average is configurable between 64 KiB and 4 MiB.

**Rationale.** Content-defined chunking keeps chunk boundaries stable when data is inserted or removed. FastCDC is among the fastest such algorithms and has a Rust implementation (`fastcdc`). 1 MiB balances deduplication against the number of objects.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Fixed-size chunks | One inserted byte shifts every following chunk; deduplication collapses |
| Rabin fingerprinting | Works, but slower than FastCDC |
| Buzhash (as in Borg) | Similar results; FastCDC is faster and has normalised chunk sizes |

**Consequences.** Chunk settings are stored in `config` but are needed only when writing; readers use the chunk lists in blobs. Changing the settings keeps old data readable but reduces deduplication between old and new data.

## ADR-005: Wasmtime and the WebAssembly component model for plugins

**Status:** Accepted

**Context.** Domain-specific behaviour (diffs, previews, storage backends, importers) must be addable without touching the core, and a faulty plugin must not endanger data.

**Decision.** Plugins are WebAssembly components run by Wasmtime.

**Rationale.** WebAssembly gives a sandbox and a capability-based permission model out of the box. Wasmtime is written in Rust. The component model lets plugins be written in any language that compiles to WebAssembly.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Native shared libraries | No sandbox; a crash takes down the host |
| Embedded scripting (Lua, Python) | Ties plugin authors to one language; weaker isolation |
| Separate processes over IPC | Strong isolation but heavier on small devices and harder to distribute |
| Other WASM runtimes | Wasmtime has the most complete component model support and is written in Rust |

**Consequences.** The plugin API needs its own versioning. Plugins pay a small performance cost compared with native code. Work is scheduled for Stage 3.

## ADR-006: Tokio outside the core only

**Status:** Accepted

**Context.** Networking, sync and the background service are naturally asynchronous. The core must stay simple to embed and to test.

**Decision.** Tokio is the async runtime, used only in the network, sync and daemon layers. The core is synchronous.

**Rationale.** Tokio is the de facto standard in Rust for networking. Keeping the core synchronous means applications can use it without adopting an async runtime, and its crash-safety logic stays easy to reason about.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| async-std | No longer maintained |
| smol | Smaller ecosystem for the networking libraries needed |
| An async core | Forces every embedder into an async runtime; complicates fault-injection tests |

**Consequences.** The service layer must bridge blocking core calls into async code (for example with `spawn_blocking`).

## ADR-007: Personal backup and home server as the first target user

**Status:** Accepted

**Context.** Apeiron targets six user profiles. Building for all at once would delay a usable product.

**Decision.** The first target user is personal backup and home servers. Developer features come in Stage 3.

**Rationale.** It is the narrowest real use case, and it is measured directly by the Stage 1 exit criterion: backing up and restoring your own data with nothing but Apeiron for one month.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Developers first | Competes head-on with Git; needs branching, merging and Git compatibility first |
| Creative professionals first | Needs type drivers (audio, image) and therefore the plugin system first |
| Enterprises first | Needs identity, permissions and audit features first |

**Consequences.** Stages 1–2 prioritise backup, restore, verification and sync. Developer-facing features wait until Stage 3.

## ADR-008: Dual licence MIT OR Apache-2.0

**Status:** Accepted

**Context.** Apeiron is meant to be embedded in other applications through its library and SDKs.

**Decision.** Apeiron is dual-licensed under MIT or Apache-2.0, at the user's option.

**Rationale.** It is the standard in the Rust ecosystem and makes embedding the SDK in other applications easy. Apache-2.0 adds an explicit patent grant.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| GPL-3.0 | Prevents embedding in non-GPL applications |
| MPL-2.0 | Workable, but less common in the Rust ecosystem |
| MIT only | No explicit patent grant |
| Apache-2.0 only | Incompatible with GPL-2.0 projects |

**Consequences.** Contributions are accepted under the same dual licence ([CONTRIBUTING.md](../CONTRIBUTING.md)). Dependencies must have compatible licences.

## ADR-009: Keyed BLAKE3 for object identifiers

**Status:** Accepted (2026-10-27)

**Context.** In Stage 2 remote targets will store encrypted data. If object identifiers were plain content hashes, a remote target could check whether a known file is stored just by computing its hash.

**Decision.** Object identifiers are `BLAKE3_keyed(id_key, type_byte ‖ content)` with a random 32-byte key per repository. Pack and index file names use plain BLAKE3 ([FORMAT.md](FORMAT.md) §4).

**Rationale.** A keyed identifier reveals nothing about content to someone without the key, while plain file names still let a server without the key detect damaged files.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Plain BLAKE3 of the content | Leaks which known files are stored |
| Keyed only when encryption is enabled | Two identifier schemes; changing encryption would require rehashing everything |

**Consequences.** Two independently created repositories do not deduplicate against each other. Copies of the same repository (sync, backup targets) share the key.

## ADR-010: Pack files instead of one file per object

**Status:** Accepted (2026-10-27)

**Context.** A 100,000-file workspace produces at least as many objects. Storing each as its own file is slow on small devices and costly on remote targets that charge per request.

**Decision.** Objects are stored in pack files (recommended 16–64 MiB), with separate index files that can be rebuilt from the packs ([FORMAT.md](FORMAT.md) §8).

**Rationale.** Fewer, larger files mean fewer file-system operations and fewer remote requests. Because indexes are only a cache, losing them never loses data.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| One file per object (loose objects) | Too many small files; slow and costly on remote targets |
| An embedded database (e.g. SQLite) | Harder to read without Apeiron; conflicts with the open-format principle |

**Consequences.** Deleting objects requires rewriting packs (garbage collection, Stage 1).

## ADR-011: zstd for compression

**Status:** Accepted (2026-10-27)

**Context.** Many objects compress well; compression saves disk space and transfer time.

**Decision.** Objects in packs are compressed with zstd (level 3) when that makes them smaller; otherwise they are stored uncompressed ([FORMAT.md](FORMAT.md) §8.1).

**Rationale.** zstd offers a very good balance of speed and ratio and is widely available, so independent readers can decompress it.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| LZ4 | Faster, but a noticeably lower compression ratio |
| A pure-Rust zstd implementation | Not yet mature enough for writing |
| No compression | Wastes disk space and bandwidth |

**Consequences.** The `zstd` crate binds the C zstd library, an accepted exception to "Rust wherever possible" (ADR-001). A C compiler is needed to build.

## ADR-012: Canonical CBOR for structured objects

**Status:** Accepted (2026-10-27)

**Context.** Blobs, trees and snapshots are hashed, so the same object must always encode to exactly the same bytes, and independent readers must be able to decode them.

**Decision.** Structured objects use CBOR (RFC 8949) with core deterministic encoding and integer map keys ([FORMAT.md](FORMAT.md) §6).

**Rationale.** CBOR is a standard with implementations in many languages; deterministic encoding guarantees stable hashes; integer keys keep objects small.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| A custom binary format | Every independent reader would need a custom parser |
| JSON | No byte strings, and canonical JSON is not widely standardised |
| Protocol Buffers | Its encoding is not canonical by design |

**Consequences.** The writer must enforce canonical rules; readers re-encode and compare to detect non-canonical objects. The library is `minicbor` (chosen on 2026-10-28): it gives full control over the bytes written and is actively maintained.

## ADR-013: Lowercase ASCII ref names

**Status:** Accepted (2026-10-27)

**Context.** Refs are stored as files. On case-insensitive file systems (Windows, macOS) `Main` and `main` would collide.

**Decision.** Ref names may contain only `a-z`, `0-9`, `.`, `_`, `-` and `/` as a separator ([FORMAT.md](FORMAT.md) §9.3).

**Rationale.** It removes collisions and encoding problems on every platform.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Any Unicode name | Case folding and Unicode normalisation differ between platforms |
| Encoding names into file names (e.g. hex) | Ref files would no longer be readable by humans |

**Consequences.** Free-form display names (including Turkish characters) are kept in the service layer and mapped to ref names.

## ADR-014: A small system crate for unsafe OS calls

**Status:** Accepted (2026-10-28)

**Context.** On Windows a rename reaches the disk before the call returns only with `MoveFileExW(MOVEFILE_WRITE_THROUGH)` ([FORMAT.md](FORMAT.md) §9.1). The standard library does not offer this, and calling the Windows API needs `unsafe` code. Until now `unsafe` was allowed only in the FFI (C ABI) crate, which sits above the core and cannot be used by it.

**Decision.** Operating system calls that need `unsafe` live in a separate, small `apeiron-sys` crate that the core depends on. It uses Microsoft's official `windows-sys` bindings. Every `unsafe` block has a comment explaining why it is sound, and every function has a test. All other crates except the FFI crate keep `#![forbid(unsafe_code)]`.

**Rationale.** The unsafe code stays in one place that is easy to review, and the layer rule (each layer uses only the layers below it) is kept.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Postpone write-through on Windows | NTFS already prevents half-finished states, but the last operation could look undone after a power failure |
| Put the call in the C ABI crate | The core would depend on a crate above it, breaking the layer rule |
| A third-party wrapper crate | No maintained, widely used crate offering this call was found |

**Consequences.** `apeiron-sys` must stay small. A new function is added there only when no safe alternative exists.

## ADR-015: JSON values at the C ABI boundary

**Status:** Proposed (2026-10-28)

**Context.** Every SDK is built on the C ABI ([SDK.md](SDK.md)). A C function per operation with C structs for every request and result would make the ABI large, and every new field would break it.

**Decision.** The C ABI has a small fixed set of functions (open, call, start, progress, cancel, wait, free). The operation is named by a string; requests, results, progress and errors are UTF-8 JSON in the form of [API.md](API.md) §8.7, described by JSON Schemas generated from the Rust types.

**Rationale.** New operations and new optional fields need no ABI change. Every language can read JSON, and the same values serve the CLI's `--json` output and the future remote mode. Converting small values to JSON costs nothing next to disk and network work; file content never passes through JSON.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| C structs and one function per operation | Large ABI; adding a field breaks binary compatibility |
| A binary encoding such as CBOR or Protocol Buffers | Faster, but harder to debug and needs an extra library in every SDK |
| Language-specific bindings without a C ABI (PyO3, …) | Each language would have its own glue code, against the "same result everywhere" goal |

**Consequences.** SDKs generate their typed classes from the schemas. Type errors in requests are found at run time by the library and reported as `InvalidInput`.

## ADR-016: Synchronous public API with jobs on threads

**Status:** Proposed (2026-10-28)

**Context.** Backups and restores take minutes or hours. Callers need progress and cancellation, and the core is synchronous (ADR-006).

**Decision.** The public API is synchronous. A long operation runs on its own thread and is returned as a job; callers read progress by polling, stop it with `cancel` and collect the result with `wait` ([API.md](API.md) §8.5).

**Rationale.** Callers need no async runtime; polling crosses the C ABI without callbacks, so no foreign code runs on Apeiron's threads; cancellation is checked between files and chunks, where stopping is always safe.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| An async API (Tokio) | Forces a runtime on every caller and every SDK |
| Progress callbacks | Calling foreign code from Apeiron's threads is fragile across the C ABI (threads, exceptions, garbage collectors) |
| Blocking calls only | No progress and no cancellation |

**Consequences.** SDKs may add callbacks or iterators on top of polling in their own language.

## ADR-017: XChaCha20-Poly1305 and Argon2id for encryption

**Status:** Accepted (2026-10-28)

**Context.** Stage 2 stores repositories on disks and servers that are not trusted ([FORMAT.md](FORMAT.md) §14). The cipher must be safe with nonces chosen at random by several independent writers, fast on a Raspberry Pi 4, and available as a maintained pure-Rust library. Passwords need a slow, memory-hard derivation.

**Decision.** Data is encrypted with XChaCha20-Poly1305 (24-byte random nonces, 16-byte tags) using the RustCrypto `chacha20poly1305` crate. Password keys are derived with Argon2id (default 64 MiB, 3 iterations, 1 lane) using the RustCrypto `argon2` crate. Recovery keys are random and use BLAKE3 key derivation.

**Rationale.** A 24-byte nonce makes random nonces safe without a counter shared between writers; ChaCha20 is fast without AES hardware, which the Raspberry Pi 4 lacks; Argon2id is the current recommendation for password hashing; both libraries are widely used, pure Rust and support Rust 1.85.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| AES-256-GCM | 12-byte nonces are unsafe to pick at random at scale, and it is slow without AES hardware |
| AES-256-GCM-SIV | Misuse-resistant, but slow without AES hardware and less widely available in Rust |
| scrypt or PBKDF2 for passwords | Weaker against GPU attacks than Argon2id |
| Encrypting whole pack files as one stream | Reading one object would require decrypting the whole pack |

**Consequences.** Encrypted packs are slightly larger (40 bytes per object and per table). Ref names stay visible (§14.5). Libraries outside RustCrypto are not used for cryptography.

## ADR-018: Whole repository copies as remotes; `russh` for SFTP, `object_store` for S3

**Status:** Accepted (2026-10-29)

**Context.** Stage 2 sends backups to an external disk, a home server and cloud storage ([FORMAT.md](FORMAT.md) §15). Remote storage is slow and may be interrupted at any moment; cloud storage has no file renames or locks. Windows, macOS and Linux are all supported, and the core stays synchronous (ADR-006).

**Decision.** A remote is a complete copy of the repository with the same keys; push and pull copy only the objects a ref is missing, as stored (still encrypted) bytes, and move refs forward only. Every remote offers the same small set of operations (read, list, write a new file atomically, replace a ref conditionally, delete, create the folders of a new copy). This `Store` interface and the folder store are in `apeiron-core`, so push and pull are written once for every kind of remote. Folders use Apeiron's own durable writes. SFTP uses `russh` and `russh-sftp`. S3 and S3-compatible storage use `object_store`. These libraries run on Tokio inside a new crate, `apeiron-remote`, behind a blocking interface.

**Rationale.** A whole copy can be restored without the original computer, and needs no new file kinds. Copying stored bytes means a remote never needs the password and data is never decrypted on the way. `russh` is pure Rust and works on Windows, unlike libraries that drive the system `ssh` program. `object_store` is maintained by the Apache Arrow project and supports conditional writes on S3, which a safe ref update needs. Both support Rust 1.85.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| One encrypted archive file per backup | Every backup would upload everything again; no deduplication |
| `ssh2` (libssh2) for SFTP | C library; ADR-001 prefers Rust |
| `openssh-sftp-client` | Drives the system `ssh` program; does not work on Windows |
| `opendal` for every remote | Its SFTP support also drives the system `ssh` program |
| Lock files on S3 instead of conditional writes | S3 has no atomic create-if-absent without conditional writes |

**Consequences.** `russh` is pinned to 0.63, the newest release that supports Rust 1.85, with the pure Rust `ring` cryptography backend; `object_store` brings `aws-lc-rs` for HTTPS. S3-compatible services without conditional writes are refused. SFTP needs the server in `~/.ssh/known_hosts`. SFTP and S3 copies have no reflog. Garbage collection of a remote copy is a later work item; a folder copy can already be collected by opening it as a repository.

## ADR-019: The background service is a user process with a jobs file

**Status:** Accepted (2026-10-29)

**Context.** Stage 2 needs scheduled backups and a regular restore test ([DAEMON.md](DAEMON.md)). The first users back up a personal computer, often a laptop that sleeps or is switched off, on Linux, macOS and Windows. Encrypted repositories need their password without a person present.

**Decision.** `apeiron daemon run` is an ordinary process of the logged-in user, started at login by the operating system's own mechanism (systemd user unit, launchd agent, Task Scheduler). Jobs are listed in a TOML file in the user's configuration folder; a job that was missed runs once at the next chance. The state of every job is kept in a JSON file and the daemon is a separate crate, `apeiron-daemon`, that uses only the public API. Passwords come from password files, as in the CLI.

**Rationale.** A user process can read the user's files and keys without administrator rights, and the same code runs on all three systems; only the start-up file differs. Catching up missed runs is what makes a schedule work on a laptop. A plain text jobs file can be read, edited and backed up by the user.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| A system-wide service running as administrator | Needs administrator rights and access to every user's files; more than a personal backup needs |
| `cron`, launchd or Task Scheduler entries that run single CLI commands | Different on each system; `cron` does not catch up on runs missed while the computer was off; no shared state for `status` |
| Keeping the jobs inside the repository | A job can span repositories and remotes; the repository should not depend on one computer's folders |
| A cron-style schedule (`30 2 * * *`) | Hard to read for the first users; `every` and `at` cover personal backups |

**Consequences.** Jobs run only while the user is logged in. Two users of one computer each run their own daemon. A server without a logged-in user runs `apeiron daemon run` as a system service by hand; the guide shows how.

## ADR-020: Sync through a remote, with a three-way merge and conflict copies

**Status:** Accepted (2026-10-29)

**Context.** Stage 2 ends with one month of trouble-free sync between two devices and a remote server ([SYNC.md](SYNC.md)). The devices are laptops and desktops that are often off; the remote is a disk, an SFTP server or S3 (ADR-018). Sync must never lose a change silently and must not become a way for a deletion to destroy the only copy (VISION.md).

**Decision.** A sync round backs up the folder, pulls the remote ref, and either pushes, applies the remote snapshot to the folder, or merges the two with their last common snapshot as base. The merge works per path; when both sides changed a file differently, the local version stays and the other is added as a conflict copy named after its device and time. The folder is changed only where it still equals the snapshot taken at the start of the round. A new device starts with `clone`, which copies the remote's config and keys.

**Rationale.** Everything is made of snapshots, so every round is history that can be restored, and push already guarantees that two devices cannot overwrite each other's ref. Meeting at a remote works when the devices are never online together. Conflict copies are what people know from Dropbox and Syncthing and need no tool to resolve; a resolution tool can come with the desktop interface.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Peer-to-peer sync between devices | Both devices must be online at once; needs discovery and NAT traversal; the remote already exists |
| Last writer wins | Silently loses the other device's change |
| Stopping the round on a conflict until the user decides | Sync stalls for every other file; the user may not look for days |
| Per-file version vectors instead of snapshots | A second model beside snapshots; loses the history that protects against spreading deletions |

**Consequences.** Conflict copies appear in the folder and must be cleaned up by hand. A file changed during a round waits for the next round. The repository history grows by one snapshot per round with changes; garbage collection with retention rules keeps it in check later.

## ADR-021: Git import and export with exact object ids, in a crate of its own

**Status:** Accepted (2026-10-29)

**Context.** Stage 3 ends when a real Git repository can be moved into Apeiron and back ([GIT.md](GIT.md)). Git concepts belong in a plugin, not in the core (VISION.md principle 1), but the plugin system comes later in the same stage. Git ref names allow characters that Apeiron ref names do not (ADR-013).

**Decision.** A new crate `apeiron-git`, beside the services and outside the core, imports and exports with `gix`. Commits and annotated tags become snapshots; whatever the snapshot fields cannot hold, such as the committer, time zones and signatures, is kept as the raw header lines in the snapshot's `meta`. Export rebuilds each object from raw bytes and checks that it gets its old id. Git ref names are escaped into Apeiron ref names. The crate moves behind the plugin interface once that exists.

**Rationale.** Keeping the raw headers makes the round trip exact for every commit Git itself writes, including signed ones, without teaching the core anything about Git. A separate crate keeps `gix` out of every other crate and is easy to wrap as a plugin later. Waiting for the plugin system would block the stage's exit criterion behind its largest item.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Map only author, time and message | Committer, time zones and signatures are lost; every exported commit gets a new id |
| Store the raw Git objects beside the snapshots | A second copy of all metadata; snapshots would not be usable on their own |
| Allow capitals and more characters in ref names | Refs are files; names differing only in case collide on Windows and macOS |
| Write the import as a WebAssembly plugin first | Needs the plugin system and a file system interface first; blocks the exit criterion |

**Consequences.** Snapshots imported from Git carry `git.*` keys in `meta`. Tree entries hold no times, so restoring a Git snapshot gives files the time of the restore. Export writes loose objects. SHA-256 repositories and shallow clones are refused for now.
