> **Türkçe:** [TEXT.tr.md](TEXT.tr.md)

# The Text Plugin

> **Status: Done.** This is the design of Stage 3 work item 6 ([ROADMAP.md](ROADMAP.md) §5): the first real type driver ([PLUGIN.md](PLUGIN.md)). It merges text files line by line and shows their differences.

Without this plugin, two changes to different lines of one text file are a conflict: the merge keeps both versions, one as a conflict copy (BRANCH.md §3). With it, both changes go into one file, as in Git, and `apeiron diff --text` shows which lines changed.

## 1. Installing

The plugin is built from `plugins/text` for WebAssembly and installed like any plugin:

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2 --manifest-path plugins/text/Cargo.toml
apeiron plugin add plugins/text/target/wasm32-wasip2/release/apeiron_text.wasm
```

From then on every merge and sync on this computer uses it; `apeiron plugin remove text` stops that.

## 2. Merging

The plugin is asked only when both sides changed a file differently (PLUGIN.md §4). It compares each side with the common version line by line:

| Situation | Result |
| --- | --- |
| The sides changed different lines | One file with both changes |
| Both made the same change | That change, once |
| Both changed the same lines, or lines next to each other, differently | A conflict: the conflict copy, as without the plugin |
| No common version (the file was added on both sides) | A conflict, unless both are equal |
| A file is not text (not UTF-8 or holds a zero byte) | A conflict, as without the plugin |

A conflict never writes markers such as `<<<<<<<` into a file: the user's file stays a valid file, and the other version is beside it. Line endings are kept as they are; a line differs when its ending differs.

## 3. Showing differences

`apeiron diff <snapshot> <snapshot> --text` lists the changed paths as `apeiron diff` does and, under each file the plugin handles, the changed lines with three lines around them, as `diff -u` writes them. An added or removed file is compared with an empty one. Files no plugin handles, and files that are not text, are listed without lines.

## 4. Files it handles

The plugin handles files by name, ignoring case: plain text and documents (`*.txt`, `*.md`, `*.rst`, `*.tex`, `*.csv`, …), configuration (`*.json`, `*.toml`, `*.yaml`, `*.ini`, `*.xml`, …), web files (`*.html`, `*.css`, `*.js`, `*.ts`, `*.svg`), source code in common languages (`*.rs`, `*.py`, `*.c`, `*.go`, `*.java`, …), subtitles and logs. `apeiron plugin list` shows the whole list.

## 5. Not part of this work item

| Item | Where |
| --- | --- |
| Merging by structure, such as JSON keys or Markdown sections | Later plugins |
| Word-by-word differences inside a line | Later |
| A ready-built plugin file to download | With the first release |

## 6. Parts

| Part | Role | Status |
| --- | --- | --- |
| `plugins/text` | The plugin; the line merge and the unified diff come from the `diffy` library | Done |
| `apeiron` | `diff_text` (API.md §8.3) | Done |
| `apeiron-cli` | `apeiron diff --text` | Done |
