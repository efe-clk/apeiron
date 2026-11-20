> **Türkçe:** [SECURITY.tr.md](SECURITY.tr.md)

# Security Policy

## 1. Supported versions

Apeiron is in early development; the first release, 0.1.0, was published on 2026-11-18, and [docs/ROADMAP.md](docs/ROADMAP.md) shows the current stage. Only the latest state of the `master` branch receives security fixes; they reach users with the next release.

| Version | Supported |
| --- | --- |
| `master` | Yes |
| Published releases | No; fixes come with the next release |

## 2. Reporting a vulnerability

Please **do not** report security problems in public issues, pull requests or discussions.

1. Open the repository on GitHub: <https://github.com/efe-clk/apeiron>.
2. Go to the **Security** tab and choose **Report a vulnerability** (GitHub private vulnerability reporting).
3. Describe the problem, the affected component and, if possible, steps to reproduce it.

Your report is visible only to the maintainers. You will get an answer as soon as possible; because the project is maintained by a small team, no fixed response time is promised. Once a fix is available, the problem is disclosed in a security advisory, crediting you if you wish.

**In scope** are especially problems that could cause silent data loss or corruption, bypass integrity verification, leak information about stored content, or bypass encryption, permissions or the plugin sandbox.

## 3. Security principles

| Principle | Rule | Status |
| --- | --- | --- |
| Integrity | Every object is verified with a cryptographic hash (BLAKE3); corruption is never silent | Done |
| Crash safety | Write → `fsync` → atomic rename; a power failure never leaves a half-finished state visible | Done |
| Content privacy | Object identifiers are keyed hashes, so stored content cannot be guessed from them | Done |
| Memory safety | Written in Rust; `unsafe` only in the FFI crate and the small `apeiron-sys` crate | In effect |
| Encryption | Client-side; keys belong to the user; the remote target never sees plaintext ([FORMAT.md](docs/FORMAT.md) §14) | Done |
| Permissions | Separate read, write, delete, share and admin permissions | Done for the web interface ([USERS.md](docs/USERS.md), [SHARE.md](docs/SHARE.md)) |
| Audit log | Sensitive operations are logged; sensitive data never is | Done for the web interface ([AUDIT.md](docs/AUDIT.md)) |
| Plugin isolation | Plugins run in a WebAssembly sandbox with declared, user-approved permissions | In place; the only permission is reaching network hosts the user granted at install, for hooks and storage backends ([PLUGIN.md](docs/PLUGIN.md) §3.1) |
| Secure defaults | Remote access is off by default; telemetry is off by default | In effect (the web server starts only when asked, `apeiron ui` listens only on this computer, and there is no telemetry) |
| No forced third parties | Data is never sent to a third party as a requirement | In effect |

The full security model is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## 4. Verifying a release

Every file of a release from 0.6.0 on can be checked in three ways:

| Check | How | What it proves |
| --- | --- | --- |
| Checksum | `sha256sum -c SHA256SUMS --ignore-missing` beside the downloaded files | The file is the one the release page lists |
| Sigstore | `gh attestation verify <file> --repo efe-clk/apeiron` | The file was built by the `Release` workflow of this repository, from the commit of its tag, and has not changed since; the signature is made through Sigstore with a short-lived certificate, so there is no long-lived key to steal |
| Build record | `BUILDINFO-linux-<architecture>.txt` | The commit, the Rust version, the build flags and the checksums of the programs inside the Linux packages, for building them again |

The macOS and Windows packages are not signed with an Apple or Microsoft certificate, so those systems still warn about them (§1); the Sigstore attestation covers them too.

Builds are made to be repeatable: the time is the commit's (`SOURCE_DATE_EPOCH`), paths of the build machine are mapped to `/build`, `/cargo` and `/rustup`, and the archives are written with sorted names and no times of their own. `ape` comes out byte for byte the same from two builds. `apeiron` does not yet: the WebAssembly runtime it contains, Wasmtime 50, generates code that differs from one build to the next even in the same folder, so two builds of the same commit differ there. The build record lists what to compare once that is solved.

The decoders of the storage format (objects, index files, pack entries, ref, reflog and operation log lines, `config` and remote addresses) are fuzzed: `fuzz/` holds a target for each, which CI runs for a minute on every push. To run one longer: `cd fuzz && cargo +nightly fuzz run objects` (needs `cargo install cargo-fuzz`). An external security audit has not been made; it needs a third party the owner chooses.
