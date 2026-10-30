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
| [015](#adr-015-json-values-at-the-c-abi-boundary) | C ABI | A small fixed set of functions; requests and results as JSON | Accepted |
| [016](#adr-016-synchronous-public-api-with-jobs-on-threads) | Long operations | Synchronous API; jobs on threads with polled progress | Accepted |
| [017](#adr-017-xchacha20-poly1305-and-argon2id-for-encryption) | Encryption | XChaCha20-Poly1305 for data, Argon2id for passwords | Accepted |
| [018](#adr-018-whole-repository-copies-as-remotes-russh-for-sftp-object_store-for-s3) | Remote targets | Remotes are whole repository copies; `russh` for SFTP, `object_store` for S3 | Accepted |
| [019](#adr-019-the-background-service-is-a-user-process-with-a-jobs-file) | Background service | A user process with a TOML jobs file, started by the operating system; missed jobs catch up | Accepted |
| [020](#adr-020-sync-through-a-remote-with-a-three-way-merge-and-conflict-copies) | Sync | Devices meet at a remote; a three-way merge per path; conflicting versions are kept as copies | Accepted |
| [021](#adr-021-git-import-and-export-with-exact-object-ids-in-a-crate-of-its-own) | Git | `apeiron-git` with `gix`; raw headers in `meta` so exported commits keep their ids | Accepted |
| [022](#adr-022-branches-are-refs-one-three-way-merge-for-sync-and-branches) | Branches | Branches are refs; one per-path three-way merge with conflict copies for sync and `merge` | Accepted |
| [023](#adr-023-search-reads-the-snapshots-without-a-stored-index) | Search | Snapshots are read directly, each folder and content once per search; no stored index | Accepted |
| [024](#adr-024-the-python-sdk-calls-the-c-abi-through-ctypes) | Python SDK | Pure Python over the C ABI with `ctypes`; one conformance suite for every language | Accepted |
| [025](#adr-025-plugins-run-with-nothing-granted-type-drivers-come-first) | Plugins | Per-user components with no permissions, time and memory limits; type drivers merge before a conflict copy; Rust 1.97 | Accepted |
| [026](#adr-026-one-set-of-web-pages-served-locally-shown-in-a-native-window) | Desktop interface | HTML pages without a build step, a JSON API on `127.0.0.1` with a token, a `wry` window | Accepted |
| [027](#adr-027-the-web-interface-serves-one-repository-over-tls-to-a-password-sign-in) | Web interface | One repository fixed at start; password sign-in with a cookie session; `rustls` with a self-signed certificate by default; allowed operations and folders | Accepted |
| [028](#adr-028-users-with-three-roles-backups-by-ref-name-and-visibility-through-parents) | Users and permissions | `admin`, `member`, `reader`; backups as ref names with a prefix; a snapshot is visible when reachable from the user's refs | Accepted |
| [029](#adr-029-the-audit-log-is-a-hash-chained-json-lines-file-written-before-anything-happens) | Audit log | JSON lines with a SHA-256 chain, appended under a lock and flushed before the request goes on; a request that cannot be recorded fails | Accepted |
| [030](#adr-030-sharing-is-by-expiring-links-to-a-fixed-version-kept-only-as-hashes) | Sharing | A random secret in the link, only its SHA-256 kept; a fixed version and path; 1 to 30 days; ends when the owner may no longer share it | Accepted |

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

**Status:** Accepted (2026-10-29)

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

**Status:** Accepted (2026-10-29)

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

## ADR-022: Branches are refs; one three-way merge for sync and branches

**Status:** Accepted (2026-10-29)

**Context.** Stage 3 adds branching and merging ([BRANCH.md](BRANCH.md)). Refs already name snapshots of any kind (ADR-013), and sync already merges two lines of history per path with conflict copies (ADR-020).

**Decision.** A branch is an ordinary ref; `ref create` and `ref delete` manage them. `merge` joins a snapshot into a ref: nothing to do, a fast-forward, or a new snapshot with both as parents, made by the same per-path three-way merge as sync, now in its own module of the services. Two different versions of a file are both kept, the incoming one as `name (conflict <from>).ext`. A merge works on snapshots only; folders are changed by `restore`, `backup` and sync.

**Rationale.** One merge means one set of rules to test and explain, for devices and for branches alike. Conflict copies never stop a merge and lose nothing; resolving them needs no special tool. Keeping folders out of the merge avoids a second way of changing a user's files beside sync, whose safety rules were made for that.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| A separate branch type beside refs | Two concepts for the same thing; ADR-013 made refs serve every purpose |
| Stopping a merge on a conflict until it is resolved, as Git does | Needs a state for a half-done merge and a tool to finish it; conflict copies need neither |
| Merging text line by line now | Belongs to type drivers (VISION.md); the text diff plugin brings it |
| A working folder that follows a branch (`checkout`) | A second way of changing user files; `restore`, `backup` and sync cover the need for now |

**Consequences.** Two edits to different lines of one text file are a conflict until the text diff plugin exists. Conflict copies appear in the merged snapshot and are cleaned up by the next change. Rebase and cherry-pick are not available.

## ADR-023: Search reads the snapshots, without a stored index

**Status:** Accepted (2026-10-29)

**Context.** Stage 3 adds search by name, metadata and text, in one snapshot or in the whole history ([SEARCH.md](SEARCH.md)). VISION.md asks for an index that works offline and is rebuilt when needed. Repositories may be encrypted, and refs move through backups, pulls, merges and garbage collection.

**Decision.** A search reads the snapshots themselves, newest first. A folder seen at the same path in a newer snapshot is not walked again, and a file's text is read once per content. Each version of a path is reported once, with the newest snapshot holding it. Text is searched in UTF-8 files up to 16 MiB.

**Rationale.** Content addressing already gives what an index would: unchanged folders and contents share ids, so a history of 3568 Git commits is searched by name in a fraction of a second and by text in a few seconds. Without an index there is no second copy of names and text to encrypt, and nothing to keep in step with refs.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| A stored index, such as a full-text index file | A second store to encrypt, update after every change and repair; not needed at today's speed |
| Searching only the newest snapshot | Finding where a file was in the past is a stated goal (VISION.md) |
| Reading every format for text now | Needs type drivers, which come with the plugin system |

**Consequences.** Text search reads every distinct text file content it meets, so it grows with the amount of text in the history. A stored index can be added later behind the same command if measurements call for it.

## ADR-024: The Python SDK calls the C ABI through ctypes

**Status:** Accepted (2026-10-29)

**Context.** Stage 3 adds the C ABI and the first SDK, for Python ([SDK.md](SDK.md)). Every SDK must give the same result as the others (ADR-015), and the owner of the project does not maintain code in several languages by hand.

**Decision.** The Python SDK is one pure Python module that loads `libapeiron` with `ctypes` and calls the functions of the C ABI; it adds only conversions, an exception class per error kind, context managers and a progress iterator. The same JSON scenarios in `sdk/conformance` run through the C functions from Rust and through the SDK from Python on every operating system in CI. The C header is generated by `cbindgen`, and a test fails when the checked-in header differs.

**Rationale.** `ctypes` is part of Python, so the SDK needs no compiler and the same file works with any Python version and operating system; the native library is the only binary. Going through the C ABI, and not around it, tests the ABI every other language will use. Shared scenarios turn "the same result in every language" into a check instead of a promise.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| A compiled extension with PyO3 | Glue code of its own beside the C ABI, and a build per Python version |
| `cffi` | An extra dependency for what `ctypes` already does for a dozen functions |
| Typed classes written by hand | Would drift from the Rust types; they come generated from the JSON schemas |

**Consequences.** Results are records read by attribute or key until the schemas exist. The library is found through `APEIRON_LIB` or next to the module until wheels carry it.

## ADR-025: Plugins run with nothing granted; type drivers come first

**Status:** Accepted (2026-10-29)

**Context.** Stage 3 adds the plugin system ([PLUGIN.md](PLUGIN.md)) on Wasmtime and the component model (ADR-005). Its first user is the text diff plugin, which merges and shows text. Plugins run code written by others, and a merge must never lose data. The newest Wasmtime that builds with Rust 1.85 was released in mid-2025.

**Decision.** The first plugin interface, `apeiron:plugin/type-driver@0.1.0`, has `describe`, `merge` and `diff`. A merge asks the first installed plugin whose pattern matches the file name before it makes a conflict copy; anything but merged content leaves the conflict copy. Plugins are installed per user. Each call runs in a new instance with WASI but nothing granted: no files, network, environment or arguments, at most 30 seconds and 512 MiB; a plugin that asks for permissions is refused. The minimum Rust version rises from 1.85 to 1.97 to use the current Wasmtime.

**Rationale.** A type driver is useful at once and needs no permission, so the sandbox can start closed and open only for plugin types that need it, with the user's approval. Falling back to the conflict copy keeps every failure as safe as having no plugin. Plugins written with a standard library import WASI, so an empty WASI context lets them run without giving them anything. A sandbox for others' code should carry current security fixes, which outweighs building with an older Rust.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Plugins stored in the repository and copied with it | Every device would run code it received from a remote; installing stays a decision of each user |
| A permission prompt now | No plugin type of this work item needs a permission |
| Fuel instead of a time limit | Fuel counts instructions, not time; the user waits for time |
| Wasmtime 34, the newest for Rust 1.85 | Misses more than a year of fixes to the sandbox |

**Consequences.** A device without the plugin makes a conflict copy where another device merged; both results are safe. Building Apeiron needs Rust 1.97 or newer, and testing the plugin host needs the `wasm32-wasip2` target. The diff function waits for the text diff plugin to be shown.

## ADR-026: One set of web pages, served locally, shown in a native window

**Status:** Accepted (2026-10-30)

**Context.** Stage 4 needs a desktop interface for users with little technical knowledge and then a web interface for server administrators ([DESKTOP.md](DESKTOP.md), VISION.md). Both must use only the public API. The owner of the project does not maintain code by hand, so two separate interfaces would double the work.

**Decision.** The interface is one set of HTML, CSS and JavaScript pages without a build step or outside libraries, compiled into the program. A local server, `apeiron-server` on `axum`, serves them and a JSON API that is the C ABI over HTTP, only on `127.0.0.1` and only to holders of a random token given in the address fragment; it checks `Host` and `Origin`. The desktop app shows the pages in a window of the operating system's web view through `wry` and `tao`; `apeiron ui` opens them in a browser.

**Rationale.** The web interface of work item 2 reuses the same pages and the same JSON API, served to the network with logins. Web pages are accessible to screen readers and keyboard users and look right in light and dark themes without extra work. Without a build step there is no JavaScript tool chain to keep current, and the pages are plain files anyone can read. `wry` uses the web view the system already has (WebKitGTK, WebKit, WebView2), so no browser is shipped, and it is the base of Tauri without Tauri's configuration and build tools.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| A native Rust toolkit (egui, iced, Slint) | A second interface would still be needed for the web; weaker accessibility; Slint's licence adds terms |
| Tauri | Its build tools and configuration add little to a window around pages served by the program itself |
| Electron | Ships a whole browser; large on a Raspberry Pi |
| A JavaScript framework with a build step | A tool chain to keep current for a few screens |

**Consequences.** Building the desktop app on Linux needs the WebKitGTK development files. The pages work only while the program runs. Anyone who can read the token, which is never written to disk by the desktop app, can use the repository the user opened.

## ADR-027: The web interface serves one repository over TLS to a password sign-in

**Status:** Accepted (2026-10-30)

**Context.** Stage 4 work item 2 shows the pages of the desktop interface to other computers, for a repository on a home server ([WEB.md](WEB.md)). On the network, the token of the local server is not enough: anyone who reaches the address could try, a browser must stay signed in across pages, and the server's whole file system must not be open to whoever signs in. Several users with their own rights are work item 3.

**Decision.** `apeiron web` serves the repository given at start, opened by the CLI as for any other command. A browser signs in with one password, stored as an Argon2id hash in `web.json` in the configuration folder; signing in gives a random 256-bit session in an `HttpOnly`, `Secure`, `SameSite=Strict` cookie, which ends after 12 idle hours or 7 days. Five wrong passwords in a row pause sign-ins for a minute. TLS comes from `rustls` with the given certificate, or a self-signed one made once and announced by its fingerprint; `--plain` leaves TLS to a proxy. Only the operations the pages use are allowed, and paths are allowed only inside the `--folder` folders after resolving links.

**Rationale.** The repository fixed at start keeps opening and creating repositories, and their passwords, off the network, and lets an encrypted repository be unlocked by `--password-file` without a person present. A cookie session needs no script to hold a secret and cannot be sent by other sites. A self-signed certificate makes TLS work without a domain name, which a home server often lacks, while a proxy such as Caddy can add a public certificate. An allow-list of operations and folders is smaller to check than every path of the whole C ABI.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| The local server's token on the network | A long secret in an address is copied into history and bookmarks; no sign-out |
| HTTP basic authentication | The browser sends the password with every request and cannot sign out |
| Plain HTTP only, TLS always from a proxy | A proxy is one more program for a home user to set up |
| Certificates from Let's Encrypt in the program | Needs a public domain name and an open port 80 or DNS access; a proxy does it today |
| Every operation of the C ABI | Remotes, export and plugins reach paths and programs beyond the allowed folders |
| A session store on disk | Restarting the server signing everyone out is acceptable and leaves nothing to steal |

**Consequences.** Someone who keeps guessing can keep the owner from signing in, a minute at a time. Whoever signs in has the rights of the user that runs the server inside the allowed folders. The browser warns about a self-signed certificate once per browser. Users and roles (work item 3) will replace the single password.

## ADR-028: Users with three roles, backups by ref name and visibility through parents

**Status:** Accepted (2026-10-30)

**Context.** Stage 4 work item 3 lets several people share one backup server ([USERS.md](USERS.md)). Each must see and restore only their own backups and folders, and one must look after the whole server. The repository has one encryption key, and snapshots are found by ids as well as by ref names, so hiding a ref is not enough.

**Decision.** The users file `web.json` replaces the single password of ADR-027 with users that have a name, an Argon2id password hash, one of three fixed roles (`admin`, `member`, `reader`), a list of ref names with `*` for a prefix, and folders inside the `--folder` folders. A snapshot is visible to a user when it is reachable through parents from the head of one of the user's refs; every snapshot a request names is resolved, checked and replaced by its id. The server reads the file again when it changes, and a session holds the password hash it began with. Jobs belong to the user who started them. Users are managed with `apeiron web user`.

**Rationale.** Three fixed roles cover a family or a small office without a rule language to learn. Ref names are already how backups are named, so a prefix such as `backup/ayse-*` gives a person every new backup of theirs without editing the file. Reachability is the only check that an id cannot get around, and the sets are kept per ref head, so it costs one walk of a history until the next backup. Reading the file again lets the CLI change users without restarting the server, and holding the password hash in the session ends stolen sessions when the password changes.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Permissions per operation, freely combined | More to understand and to get wrong than three roles |
| Checking only ref names in requests | Snapshot ids and specs such as `backup/x~1` would pass |
| A key per user with separate encryption | Deduplication across users would stop, and the server needs every key for checks anyway |
| A database for users | One small JSON file is enough for a home server and is read by people |
| Managing users in the pages now | The CLI covers it; the pages can follow once the audit log records changes |

**Consequences.** Everyone who can sign in relies on the server holding the repository's key, so users are protected from each other by the server, not by cryptography. A snapshot merged into a user's backup becomes visible to that user. A `member` or `reader` must name a snapshot to search. The `--set-password` option of ADR-027 is gone.

## ADR-029: The audit log is a hash-chained JSON lines file, written before anything happens

**Status:** Accepted (2026-10-30)

**Context.** Stage 4 work item 4 records who did what in the web interface ([AUDIT.md](AUDIT.md)). An admin must be able to trust the record: a user who gains access to the file, or a mistake, must not change it unnoticed, and nothing may happen that is missing from it. The server and the CLI both write to it.

**Decision.** The log is one file of JSON lines in the state folder. Each line has a sequence number and the SHA-256 hash of the line before it; `apeiron web audit verify` checks the chain and prints the hash of the last line. A line is appended under a file lock and flushed to disk before the request goes on; a short operation is recorded after it runs and before its result is sent, a long one after its checks and before it starts. If recording fails, the request fails. Admins read the log on an Activity page and with `apeiron web audit show`.

**Rationale.** A hash chain shows changed, removed and reordered lines with nothing more than a file and a hash function, and the last hash, kept elsewhere, also shows a rewritten or shortened log. JSON lines can be read with any tool and appended without rewriting. Failing closed is what makes the record complete: an operation that cannot be recorded does not happen. The lock of the standard library lets the CLI record user changes while the server runs.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Keeping the log inside the repository as snapshots | Every page view would write a snapshot; the repository's history would fill with records |
| syslog or journald | Not on every system, not tamper-evident, and not readable by the pages without more rights |
| Signing each line with a key | The server would hold the key, so anyone who can change the file can usually sign too; the chain plus a hash kept elsewhere covers the same cases |
| Recording only changes, not reading | "Who looked at my files?" is one of the questions the log must answer |
| Recording after a long operation ends | A crash or a lost result would leave a restore unrecorded |

**Consequences.** Every request in the web interface waits for a write to disk. The log grows without limit until rotation is added. The log can be rewritten whole by someone with access to the file; only a hash kept elsewhere reveals that.

## ADR-030: Sharing is by expiring links to a fixed version, kept only as hashes

**Status:** Accepted (2026-10-30)

**Context.** Stage 4 work item 5 lets a user of the web interface give a version, folder or file of a backup to someone who has no account on the server ([SHARE.md](SHARE.md)). The person receiving it must need nothing but a browser; the person sharing it must be able to end it; a leaked copy of the server's files must not open the links; and sharing must not reach past what the user may see.

**Decision.** A link carries a secret of 32 random bytes; the server keeps only its SHA-256 hash, with the owner, the version's id, the path and an end 1 to 30 days away, in `shares.json` in the state folder, changed under a lock and replaced whole. Each use checks that the link has not ended and that its owner still exists, is not a `reader` and still sees the version. The link holder can only list and download below the shared path; files are streamed with a new `Repository::read_file`. Every use is recorded in the audit log without the secret.

**Rationale.** A random secret needs no account and no password to type, and 256 bits cannot be guessed. Keeping only its hash means a copy of the file opens nothing, as with passwords. A fixed version means the link shows exactly what was checked when it was made. Checking the owner at every use makes a link follow the owner's rights instead of outliving them. A small file of its own keeps links apart from users, so removing one never touches the other.

**Alternatives considered.**

| Alternative | Why not chosen |
| --- | --- |
| Signed links with no state (HMAC of the path and end) | A single link could not be removed before it ends without keeping a list anyway |
| Guest accounts in the users file | The receiver would need a name and a password, and the admin would have to clean up afterwards |
| Exporting an archive and serving the file | Copies the data on the server for each share and does not show the folder before downloading |
| Links to a backup that follow its newest version | What is shared would change after it was checked; a user would share files they never looked at |
| Keeping the secret so the link can be shown again | Anyone who reads the file could use every link |

**Consequences.** A lost link cannot be shown again; a new one must be made. Anyone who holds the link can read what it names until it ends, so it must be sent over a channel the sharer trusts. Each use reads the shares file and the users file, which is fine for the numbers of a home or small office. Folders are downloaded one file at a time until archives are added.
