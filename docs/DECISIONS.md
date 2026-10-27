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

ADR 001–008 were made before Stage 0 began ([VISION.md](VISION.md)). ADR 009–013 were approved together with the storage format on 2026-10-27 ([FORMAT.md](FORMAT.md)).

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

**Consequences.** Contributors need Rust. Some C libraries may still be used through bindings where no mature Rust alternative exists (see ADR-011). Platform calls that need `unsafe` must live in the FFI crate or wait for a safe wrapper.

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

**Consequences.** The writer must enforce canonical rules; readers re-encode and compare to detect non-canonical objects. The CBOR library is not chosen yet.

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
