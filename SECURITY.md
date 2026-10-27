> **Türkçe:** [SECURITY.tr.md](SECURITY.tr.md)

# Security Policy

## 1. Supported versions

Apeiron is in early development (Stage 0) and has no releases yet. Only the latest state of the `master` branch receives security fixes.

| Version | Supported |
| --- | --- |
| `master` | Yes |
| Releases | None published yet |

## 2. Reporting a vulnerability

Please **do not** report security problems in public issues, pull requests or discussions.

1. Open the repository on GitHub: <https://github.com/efe-clk/apeiron>.
2. Go to the **Security** tab and choose **Report a vulnerability** (GitHub private vulnerability reporting).
3. Describe the problem, the affected component and, if possible, steps to reproduce it.

Your report is visible only to the maintainers. You will get an answer as soon as possible; because the project is maintained by a small team, no fixed response time is promised. Once a fix is available, the problem is disclosed in a security advisory, crediting you if you wish.

**In scope** are especially problems that could cause silent data loss or corruption, bypass integrity verification, leak information about stored content, or (once they exist) bypass encryption, permissions or the plugin sandbox.

## 3. Security principles

| Principle | Rule | Status |
| --- | --- | --- |
| Integrity | Every object is verified with a cryptographic hash (BLAKE3); corruption is never silent | Done |
| Crash safety | Write → `fsync` → atomic rename; a power failure never leaves a half-finished state visible | Done (Windows write-through Planned) |
| Content privacy | Object identifiers are keyed hashes, so stored content cannot be guessed from them | Done |
| Memory safety | Written in Rust; `unsafe` only in the FFI crate | In effect |
| Encryption | Client-side; keys belong to the user; the remote target never sees plaintext | Planned (Stage 2) |
| Permissions | Separate read, write, delete, share and admin permissions | Planned (Stage 4) |
| Audit log | Sensitive operations are logged; sensitive data never is | Planned (Stage 4) |
| Plugin isolation | Plugins run in a WebAssembly sandbox with declared, user-approved permissions | Planned (Stage 3) |
| Secure defaults | Remote access is off by default; telemetry is off by default | In effect (there is no remote access or telemetry yet) |
| No forced third parties | Data is never sent to a third party as a requirement | In effect |

The full security model is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
