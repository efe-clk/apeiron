//! `apeiron mcp` (AGENTS.md §4): a Model Context Protocol server on
//! standard input and output, so an AI agent can read the history of a
//! repository: refs, snapshots, files, differences and search. It changes
//! nothing; the tools go through `apeiron::json`, as every interface does.

use std::io::{BufRead, Write};

use apeiron::{ListRequest, Repository};
use serde_json::{Value, json};

/// The protocol versions this server speaks, newest first.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// At most this much of a file is given to the agent.
const FILE_LIMIT: u64 = 256 * 1024;

fn tools() -> Value {
    let text = |d: &str| json!({"type": "string", "description": d});
    let snapshot =
        text("A snapshot: a ref such as backup/documents, ref~1 for the one before, or an id");
    json!([
        {"name": "refs", "description": "The refs of the repository: backups, branches, checkpoints, each with the snapshot it points to.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "log", "description": "The snapshots of a ref, newest first, with time, message and id.",
         "inputSchema": {"type": "object", "properties": {
            "start": snapshot,
            "limit": {"type": "integer", "description": "How many, 20 by default"},
            "grep": text("Only snapshots whose message, author, host or metadata hold this text")},
            "required": ["start"]}},
        {"name": "list", "description": "The files and folders inside a snapshot at a path.",
         "inputSchema": {"type": "object", "properties": {
            "snapshot": snapshot, "path": text("A path inside the snapshot; empty for its root")},
            "required": ["snapshot"]}},
        {"name": "read_file", "description": "The content of a file in a snapshot, as text; at most 256 KiB.",
         "inputSchema": {"type": "object", "properties": {
            "snapshot": snapshot, "path": text("The file's path inside the snapshot")},
            "required": ["snapshot", "path"]}},
        {"name": "diff", "description": "The paths added, removed and changed from one snapshot to another.",
         "inputSchema": {"type": "object", "properties": {"from": snapshot, "to": snapshot},
            "required": ["from", "to"]}},
        {"name": "diff_text", "description": "The changed lines of text files from one snapshot to another.",
         "inputSchema": {"type": "object", "properties": {"from": snapshot, "to": snapshot},
            "required": ["from", "to"]}},
        {"name": "search", "description": "Files by name and text in a snapshot, or in its whole history.",
         "inputSchema": {"type": "object", "properties": {
            "snapshot": snapshot,
            "pattern": text("A name with * and ?, or with / a whole path"),
            "text": text("Text a text file must hold, ignoring case"),
            "history": {"type": "boolean", "description": "Search every snapshot of its history"},
            "limit": {"type": "integer", "description": "At most this many hits, 100 by default"}},
            "required": ["snapshot"]}},
    ])
}

/// Runs a tool; the text the agent gets, or why it failed.
fn call(repo: &Repository, name: &str, args: &Value) -> Result<String, String> {
    let s = |k: &str| args[k].as_str().unwrap_or_default().to_owned();
    let value = match name {
        "refs" => apeiron::json::call(repo, "refs", json!({})),
        "log" => apeiron::json::call(
            repo,
            "log",
            json!({"start": s("start"), "limit": args["limit"].as_u64().unwrap_or(20), "grep": args.get("grep")}),
        ),
        "list" => apeiron::json::call(
            repo,
            "list",
            json!({"snapshot": s("snapshot"), "path": s("path")}),
        ),
        "diff" | "diff_text" => {
            apeiron::json::call(repo, name, json!({"from": s("from"), "to": s("to")}))
        }
        "read_file" => return read_file(repo, &s("snapshot"), &s("path")),
        "search" => {
            let request = json!({
                "snapshot": s("snapshot"),
                "history": args["history"].as_bool().unwrap_or(false),
                "pattern": args.get("pattern"),
                "text": args.get("text"),
                "limit": args["limit"].as_u64().unwrap_or(100),
            });
            apeiron::json::start(repo, "search", request).and_then(|mut job| {
                job.wait(None).unwrap_or_else(|| {
                    Err(apeiron::Error::new(
                        apeiron::ErrorKind::Internal,
                        "no result",
                    ))
                })
            })
        }
        _ => return Err(format!("unknown tool {name}")),
    };
    value
        .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
        .map_err(|e| e.to_string())
}

fn read_file(repo: &Repository, snapshot: &str, path: &str) -> Result<String, String> {
    let mut reader = repo
        .open_file(&ListRequest {
            snapshot: snapshot.to_owned(),
            path: path.into(),
        })
        .map_err(|e| e.to_string())?;
    let size = reader.size();
    let mut buf = vec![0; size.min(FILE_LIMIT) as usize];
    let n = reader.read_at(0, &mut buf).map_err(|e| e.to_string())?;
    buf.truncate(n);
    let mut text = match String::from_utf8(buf) {
        Ok(text) => text,
        Err(_) => return Ok(format!("{path} is not text ({size} bytes)")),
    };
    if size > FILE_LIMIT {
        text.push_str(&format!("\n… {} more bytes not shown", size - FILE_LIMIT));
    }
    Ok(text)
}

/// The answer to one message, or `None` for a notification.
fn answer(repo: &Repository, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let params = &message["params"];
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = VERSIONS
                .iter()
                .find(|v| **v == asked)
                .unwrap_or(&VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "apeiron", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Read-only access to an Apeiron repository: its refs, the history of each, the files of any snapshot, differences and search. Checkpoints of folders an agent works in are under checkpoint/.",
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call(repo, name, &args) {
                Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            }
        }
        method => {
            return Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unknown method {method}")}}));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Serves the protocol on standard input and output until input ends.
pub(crate) fn serve(repo: &Repository) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => answer(repo, &message),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": e.to_string()}})),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}
