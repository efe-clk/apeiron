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
| Both changed the same lines, or lines next to each other, differently | A conflict: the conflict copy, as without the plugin; unless all three versions are JSON, YAML or Markdown, as below |
| No common version (the file was added on both sides) | A conflict, unless both are equal |
| A file is not text (not UTF-8 or holds a zero byte) | A conflict, as without the plugin |

A conflict never writes markers such as `<<<<<<<` into a file: the user's file stays a valid file, and the other version is beside it. Line endings are kept as they are; a line differs when its ending differs.

When the lines collide and the common version and both sides are JSON documents, the plugin merges them by structure instead: objects member by member, at any depth; a member added on one side is kept, one removed on one side is removed if the other side left it unchanged; any other value, such as a number, a text or an array, must be changed on one side only or the same way on both, else the file is still a conflict. The merged document keeps the members in the order of this side, then the new members of the other side, and is written with this side's indentation (two spaces when there is none), so its formatting can differ from both sides.

When the lines collide and the files are not JSON but YAML documents whose left edge holds only top-level keys (`name:` or `name: value`), comments and a `---` at the start, the plugin merges them key by key: a key and the indented lines below it form one block, the lines before the first key one more, and blocks are joined as Markdown sections are below, matched by the key's name. A key that comes twice, or any other line at the left edge, such as a list item, leaves the file to the next way.

When the lines collide and the files are neither JSON nor such YAML but both sides are Markdown with headings (lines of one to six `#` and a space, outside fenced code), the plugin merges them section by section, a section being a heading and the lines up to the next one, and the text before the first heading being one more. Sections are matched by their heading line: a section changed on one side is taken from that side, one changed on both is merged line by line and must not collide, one removed on one side is removed if the other side left it unchanged, and a new section follows the section it follows on its side; two new sections in the same place keep the other side's first. When one side has two equal headings the file is a conflict, as sections cannot be told apart. The plugin does not see file names, so any text with such lines, a script with `# ` comments too, can be merged this way.

## 3. Showing differences

`apeiron diff <snapshot> <snapshot> --text` lists the changed paths as `apeiron diff` does and, under each file the plugin handles, the changed lines with three lines around them, as `diff -u` writes them. An added or removed file is compared with an empty one. Files no plugin handles, and files that are not text, are listed without lines.

The Compare page of the desktop and web interfaces shows the same lines and also marks the words that changed: where a run of removed lines is followed by as many added lines, each removed line is paired with the added line in the same place and the words outside their longest common sequence are highlighted. A word is a run of letters, digits and `_`; spaces and other characters count one by one. Very long line pairs are shown without word marks.

## 4. Files it handles

The plugin handles files by name, ignoring case: plain text and documents (`*.txt`, `*.md`, `*.rst`, `*.tex`, `*.csv`, …), configuration (`*.json`, `*.toml`, `*.yaml`, `*.ini`, `*.xml`, …), web files (`*.html`, `*.css`, `*.js`, `*.ts`, `*.svg`), source code in common languages (`*.rs`, `*.py`, `*.c`, `*.go`, `*.java`, …), subtitles and logs. `apeiron plugin list` shows the whole list.

## 5. Not part of this work item

| Item | Where |
| --- | --- |
| Merging by other structures, such as XML elements | Later plugins |
| A ready-built plugin file to download | With the first release |

## 6. Parts

| Part | Role | Status |
| --- | --- | --- |
| `plugins/text` | The plugin; the line merge and the unified diff come from the `diffy` library | Done |
| `apeiron` | `diff_text` (API.md §8.3) | Done |
| `apeiron-cli` | `apeiron diff --text` | Done |
