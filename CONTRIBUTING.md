> **Türkçe:** [CONTRIBUTING.tr.md](CONTRIBUTING.tr.md)

# Contributing to Apeiron

This guide explains how to set up a development environment, which checks every change must pass and how changes are accepted. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first to understand how the system is built.

## 1. Repository layout

| Path | Content |
| --- | --- |
| `crates/` | Cargo workspace; one crate per layer (today only `apeiron-core`) |
| `docs/` | Documentation, in English (`X.md`) and Turkish (`X.tr.md`) |
| `.github/workflows/` | CI: formatting, lints and tests |
| `LICENSE-MIT`, `LICENSE-APACHE` | The two licences |

## 2. Development environment

- **Rust:** install with [rustup](https://rustup.rs) (stable toolchain with `rustfmt` and `clippy`). The minimum supported version is 1.85 (edition 2024).
- **C compiler:** required to build the `zstd` and `blake3` dependencies (GCC or Clang on Linux/macOS, MSVC on Windows).
- **Platforms:** Linux (x86_64, aarch64), macOS and Windows are all tested in CI.

```bash
git clone https://github.com/efe-clk/apeiron.git
cd apeiron
cargo build
cargo test
```

## 3. Checks before every change

Every change must pass these three commands cleanly. CI runs them on every push and pull request.

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

| Check | Rule |
| --- | --- |
| Formatting | `cargo fmt` with default settings |
| Lints | No Clippy warnings; warnings are errors |
| Tests | All tests pass on Linux x86_64, Linux aarch64, macOS and Windows |

## 4. Code rules

| Rule | Details |
| --- | --- |
| Rust wherever possible | C only in the outward-facing ABI layer ([ADR-001](docs/DECISIONS.md#adr-001-rust-as-the-main-language)) |
| Strict layering | Each layer is a separate crate and uses only the layer directly below it |
| No `unsafe` | `unsafe` is allowed only in the FFI crate; every other crate has `#![forbid(unsafe_code)]` |
| Typed errors | Library crates use `thiserror`; no `unwrap`/`expect` in library code (tests may use them) |
| Fault-injection tests | Data-integrity code (writing, ref updates, recovery) is verified with fault-injection tests |
| English in code | Code, identifiers, comments and commit messages are in English |
| Match the surroundings | New code follows the naming, comment density and idioms of the code around it |

## 5. Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org/):

```text
<type>(<optional scope>): <short summary in the imperative>

<optional body: what and why>
```

| Type | When |
| --- | --- |
| `feat` | A new capability |
| `fix` | A bug fix |
| `docs` | Documentation only |
| `test` | Tests only |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `build` | Build system or dependencies |
| `ci` | CI configuration |
| `chore` | Anything else |

Example: `feat(core): add storage engine with packs, index and atomic refs`.

## 6. Storage format changes

The storage format is a public contract ([docs/FORMAT.md](docs/FORMAT.md)): data written today must stay readable.

1. Every change that affects the format is documented in `docs/FORMAT.md` and `docs/FORMAT.tr.md` **first**.
2. Only after that change is agreed is it implemented.
3. A change that existing readers cannot understand raises the format version; old versions must remain readable.
4. If the change is a decision, a new ADR is added to `docs/DECISIONS.md` and `docs/DECISIONS.tr.md`.

## 7. Documentation

Documentation is bilingual: `X.md` is English, `X.tr.md` is Turkish.

| Rule | Details |
| --- | --- |
| Same commit | When a document changes, both language versions are updated in the same commit |
| Same structure | Both versions have the same headings, tables and diagrams |
| Language link | Every file starts with a link to its other-language version |
| Honest status | Unwritten features are never described as working; status is always stated (Planned / In progress / Done) |
| Diagrams | Diagrams are written in Mermaid so GitHub renders them |
| Links | Internal links use relative paths |

## 8. Licence of contributions

Unless you explicitly state otherwise, any contribution you submit is dual-licensed under MIT OR Apache-2.0, without additional terms ([ADR-008](docs/DECISIONS.md#adr-008-dual-licence-mit-or-apache-20)).
