> **Türkçe:** [AGENTS.tr.md](AGENTS.tr.md)

# AI Agents

> **Status: Done.** An AI agent that edits files, such as Claude Code or Cursor, gets a checkpoint of its folder after each step, and its work can be undone by turn or by session. `apeiron mcp` gives it the history of a repository through the Model Context Protocol. The decision behind it is ADR-058 ([DECISIONS.md](DECISIONS.md)).

## 1. Commands

| Command | What it does |
| --- | --- |
| `apeiron checkpoint save [<folder>] [--session <id>] [--message <text>] [--hook]` | Records the folder (default: the current one) as a checkpoint, if it changed since the last one |
| `apeiron checkpoint list [<folder>]` | Lists the folder's checkpoints, newest first, with their session |
| `apeiron checkpoint undo [<folder>] [--session] [--to <checkpoint>] [--dry-run]` | Makes the folder hold what it held before the agent's last turn; `--session` before its whole last session; `--to` a given checkpoint |
| `apeiron mcp` | Serves the repository to an agent through the Model Context Protocol on standard input and output (§4) |

Without `--repo` (or `APEIRON_REPO`), checkpoints go into a repository of the user's own, made when first needed in the data folder (`~/.local/share/apeiron/checkpoints` on Linux). `apeiron mcp` uses the same one when no repository is given.

## 2. Checkpoints

A checkpoint is a backup of the folder into the ref `checkpoint/<folder name>`, made the way agents need:

| Rule | Why |
| --- | --- |
| A folder that has not changed since the last checkpoint makes none | Hooks run after every step, many of which change nothing |
| `.git`, the ready sets of `@dev` and what `.gitignore` names are left out (CLI.md §4.1) | Git keeps its own history, and build output can be made again |
| The ransomware alarm is off (CLI.md §4.3) | An agent rewriting many files is what checkpoints are for |
| `checkpoint.session` and `checkpoint.event` are kept in the snapshot's `meta` | Undo finds turns and sessions by them |

`--hook` reads what an agent's hook sends on standard input, as JSON: `session_id`, `cwd` (the folder), `hook_event_name` and `tool_name`. It prints nothing on standard output and always exits with 0, so a checkpoint never stops the agent; a problem is written to standard error.

For Claude Code, in `.claude/settings.json` of a project or of the user:

```json
{
  "hooks": {
    "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "apeiron checkpoint save --hook"}]}],
    "PostToolUse": [{"matcher": "Write|Edit|MultiEdit|Bash",
                     "hooks": [{"type": "command", "command": "apeiron checkpoint save --hook"}]}]
  }
}
```

The checkpoint made when the user asks something (`UserPromptSubmit`) starts a turn. Other agents can run `apeiron checkpoint save --session <id>` before and after their steps.

## 3. Undo

`apeiron checkpoint undo` takes the newest session among the folder's checkpoints and goes back to the checkpoint that started its last turn; `--session` goes back to the session's first checkpoint, and `--to` to any checkpoint `list` shows.

1. Record what the folder holds now, as a checkpoint of the session `undo`, so the undo itself can be undone with `--to`.
2. Make the folder hold the checkpoint as `apeiron switch` does (BRANCH.md §6): only what differs is written or removed, and what checkpoints leave out (`.git`, build output) is left alone.
3. Record the result as a checkpoint `checkpoint: undone`, from which the next checkpoints go on.

## 4. Model Context Protocol

`apeiron mcp` is a server of the [Model Context Protocol](https://modelcontextprotocol.io) on standard input and output, so an agent can look into the history of a repository: what a file held before, what changed between two backups, where a text was. It changes nothing.

| Tool | Gives |
| --- | --- |
| `refs` | The refs with the snapshot each points to |
| `log` | The snapshots of a ref, newest first |
| `list` | The files and folders of a snapshot at a path |
| `read_file` | A file of a snapshot as text, at most 256 KiB |
| `diff`, `diff_text` | The changed paths, and the changed lines of text files, between two snapshots |
| `search` | Files by name and text in a snapshot or its whole history (SEARCH.md) |

For Claude Code: `claude mcp add apeiron -- apeiron mcp`, or with a repository: `claude mcp add apeiron -- apeiron --repo /path/to/repo mcp`. The tools go through `apeiron::json`, the same operations as the C ABI and the pages.

## 5. Not part of this work item

| Item | Where |
| --- | --- |
| Tools that change the repository through MCP | Later; an agent changes files, and checkpoints record them |
| Hooks for other agents than Claude Code | Any agent that can run a command uses `checkpoint save`; ready settings for others come with their users |

## 6. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron` | `skip_same` of a backup request, `exclude` of a switch request | Done |
| `apeiron-cli` | `apeiron checkpoint`, `apeiron mcp` | Done |
