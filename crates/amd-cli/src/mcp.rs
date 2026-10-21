//! `amd mcp`: the board for any AI assistant, over the Model Context
//! Protocol (ticket 186).
//!
//! An AI tool (Claude Code, Copilot in VS Code, Cursor, Codex, Gemini
//! CLI…) starts `amd mcp` and talks JSON-RPC 2.0 to it, one message per
//! line on stdin and stdout. The tools themselves are
//! `agile_md::assistant`'s; this is only the wire. Written by hand rather
//! than with an SDK: five methods, and the CLI stays synchronous.
//!
//! **stdout carries nothing but protocol.** amd-lib prints nothing, but the
//! `git` it runs inherits stdout and stdin, so the server keeps private
//! copies of both and points the process's own at stderr and `/dev/null`
//! (`NUL` on Windows) before anything else runs.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use agile_md::assistant;

/// The protocol versions this server speaks, newest first.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "agile-md is a Kanban board kept as markdown files in git. \
These tools read the project's board, tickets, documents and ideas, and change tickets \
the way the `amd` command and the amdui desktop board do: each change is committed and \
pushed. Call `screen` to find what the user is looking at in amdui (\"this ticket\"). \
Tickets move one column at a time: backlog, todo, doing, done, or the project's own columns (`board` lists them). Ask before a change the \
user didn't request.";

/// Runs the server until the client closes stdin.
pub fn run() -> Result<()> {
    // Never a prompt, and never git asking for a password on our stdin.
    // Safety: nothing else is running yet; these are the first lines.
    unsafe {
        std::env::set_var("AMD_NO_INPUT", "1");
        std::env::set_var("GIT_TERMINAL_PROMPT", "0");
    }
    let (input, mut output) = take_stdio()?;
    let input = BufReader::new(input);
    for line in input.lines() {
        let line = line.context("reading from the client")?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(&line) {
            let mut bytes = serde_json::to_vec(&reply)?;
            bytes.push(b'\n');
            output.write_all(&bytes)?;
            output.flush()?;
        }
    }
    Ok(())
}

/// Private copies of stdin and stdout for the protocol; the process's own
/// become `/dev/null` and stderr, for everything it runs.
#[cfg(unix)]
fn take_stdio() -> Result<(File, File)> {
    use std::os::fd::FromRawFd;
    // Safety: plain descriptor calls on 0, 1 and 2, checked; the new
    // descriptors are owned by the returned files from here on.
    unsafe {
        let input = libc::dup(0);
        let output = libc::dup(1);
        if input < 0 || output < 0 {
            bail!("couldn't take over stdin and stdout");
        }
        let null = File::open("/dev/null").context("opening /dev/null")?;
        if libc::dup2(std::os::fd::AsRawFd::as_raw_fd(&null), 0) < 0 || libc::dup2(2, 1) < 0 {
            bail!("couldn't redirect stdin and stdout");
        }
        Ok((File::from_raw_fd(input), File::from_raw_fd(output)))
    }
}

/// The same on Windows (ticket 279): a child process inherits the standard
/// *handles*, not the C runtime's descriptors, so those are swapped. The
/// protocol keeps the handles stdin and stdout were; the process's become
/// `NUL` and stderr, which `std::process::Command` and `println!` pick up,
/// since both ask for the standard handle each time.
#[cfg(windows)]
fn take_stdio() -> Result<(File, File)> {
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    // Safety: the standard handles are checked before use; once replaced,
    // the old input and output are owned by the returned files and nothing
    // else refers to them. `NUL` is leaked on purpose: it is stdin from here
    // on, for as long as the process runs.
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        let error = GetStdHandle(STD_ERROR_HANDLE);
        let usable = |h: HANDLE| !h.is_null() && h != INVALID_HANDLE_VALUE;
        if !usable(input) || !usable(output) {
            bail!("couldn't take over stdin and stdout");
        }
        let null = File::open("NUL").context("opening NUL")?.into_raw_handle();
        if SetStdHandle(STD_INPUT_HANDLE, null) == 0 || SetStdHandle(STD_OUTPUT_HANDLE, error) == 0
        {
            bail!("couldn't redirect stdin and stdout");
        }
        Ok((File::from_raw_handle(input), File::from_raw_handle(output)))
    }
}

/// The reply to one line from the client, if it wants one.
fn handle(line: &str) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(err) => return Some(error(Value::Null, -32700, &format!("not JSON: {err}"))),
    };
    if !message.is_object() {
        return Some(error(Value::Null, -32600, "expected one JSON-RPC message"));
    }
    let method = message["method"].as_str().unwrap_or_default();
    // A notification (no id) is answered with nothing, and so is a reply
    // to a request we never sent.
    let id = message.get("id").cloned()?;
    if method.is_empty() {
        return None;
    }
    let params = message.get("params").cloned().unwrap_or(json!({}));
    Some(match method {
        "initialize" => result(id, initialize(&params)),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({"tools": tool_list()})),
        "tools/call" => result(id, call(&params)),
        _ => error(id, -32601, &format!("amd mcp doesn't do {method}")),
    })
}

fn result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn initialize(params: &Value) -> Value {
    let asked = params["protocolVersion"].as_str().unwrap_or_default();
    let version = VERSIONS
        .iter()
        .find(|v| **v == asked)
        .unwrap_or(&VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {
            "name": "agile-md",
            "title": "agile-md board",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": INSTRUCTIONS,
    })
}

fn tool_list() -> Vec<Value> {
    assistant::tools()
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "title": tool.title,
                "description": tool.description,
                "inputSchema": tool.input,
                "annotations": {
                    "title": tool.title,
                    "readOnlyHint": tool.read_only,
                    // Changes are commits: undone by another change, not
                    // lost.
                    "destructiveHint": false,
                    "idempotentHint": tool.read_only,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

/// A tool's result. A tool that fails is still a result, marked as an
/// error, so the model reads why (a refused move, a stale edit) and can
/// say so.
fn call(params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let outcome = std::panic::catch_unwind(|| assistant::call(name, &args))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("{name} failed unexpectedly")));
    match outcome {
        Ok(reply) => {
            let mut result = json!({
                "content": [{"type": "text", "text": reply.text}],
                "isError": false,
            });
            if reply.data.is_object() {
                result["structuredContent"] = reply.data;
            }
            result
        }
        Err(err) => json!({
            "content": [{"type": "text", "text": format!("amd: {err:#}")}],
            "isError": true,
        }),
    }
}

/// The AI tools `amd mcp setup` knows how to connect.
pub const CLIENTS: [&str; 5] = ["claude", "vscode", "cursor", "codex", "gemini"];

/// How to connect `client` (or every one) to `amd mcp`.
pub fn setup(client: Option<&str>) -> Result<String> {
    let amd = "amd";
    let mut out = String::new();
    let all = client.is_none();
    let wants = |name: &str| all || client == Some(name);
    if let Some(client) = client
        && !CLIENTS.contains(&client)
    {
        bail!(
            "amd mcp setup knows {}; for another tool, run `{amd} mcp` as a stdio MCP server",
            CLIENTS.join(", ")
        );
    }
    if wants("claude") {
        out.push_str(&format!(
            "Claude Code (in a terminal, or amdui's AI tab):\n  claude mcp add --scope user agile-md -- {amd} mcp\n\n"
        ));
    }
    if wants("vscode") {
        out.push_str(&format!(
            "VS Code (Copilot agent mode): add to .vscode/mcp.json, or run \"MCP: Add Server\":\n  {{\"servers\": {{\"agile-md\": {{\"type\": \"stdio\", \"command\": \"{amd}\", \"args\": [\"mcp\"]}}}}}}\n\n"
        ));
    }
    if wants("cursor") {
        out.push_str(&format!(
            "Cursor: add to ~/.cursor/mcp.json:\n  {{\"mcpServers\": {{\"agile-md\": {{\"command\": \"{amd}\", \"args\": [\"mcp\"]}}}}}}\n\n"
        ));
    }
    if wants("codex") {
        out.push_str(&format!(
            "Codex: add to ~/.codex/config.toml:\n  [mcp_servers.agile-md]\n  command = \"{amd}\"\n  args = [\"mcp\"]\n\n"
        ));
    }
    if wants("gemini") {
        out.push_str(&format!(
            "Gemini CLI: add to ~/.gemini/settings.json:\n  {{\"mcpServers\": {{\"agile-md\": {{\"command\": \"{amd}\", \"args\": [\"mcp\"]}}}}}}\n\n"
        ));
    }
    if all {
        out.push_str(
            "Any other tool: run `amd mcp` as a stdio MCP server, started in the project's folder.\n",
        );
    }
    Ok(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_agrees_a_version_and_lists_tools() {
        let reply = handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#).unwrap();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        let newer = handle(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#).unwrap();
        assert_eq!(newer["result"]["protocolVersion"], VERSIONS[0]);

        let list = handle(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#).unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "move_ticket"
            && t["annotations"]["readOnlyHint"] == false
            && t["inputSchema"]["required"][0] == "ticket"));
        assert!(
            tools
                .iter()
                .any(|t| t["name"] == "board" && t["annotations"]["readOnlyHint"] == true)
        );
    }

    #[test]
    fn notifications_get_no_reply_and_mistakes_get_errors() {
        assert_eq!(
            handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            None
        );
        assert_eq!(handle("not json").unwrap()["error"]["code"], -32700);
        assert_eq!(
            handle(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#).unwrap()["error"]["code"],
            -32601
        );
        let unknown =
            handle(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}}"#)
                .unwrap();
        assert_eq!(unknown["result"]["isError"], true);
        assert!(
            unknown["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no tool called nope")
        );
    }

    #[test]
    fn setup_names_each_tool() {
        let all = setup(None).unwrap();
        for name in ["Claude Code", "VS Code", "Cursor", "Codex", "Gemini"] {
            assert!(all.contains(name), "{name}");
        }
        assert_eq!(
            setup(Some("claude")).unwrap(),
            "Claude Code (in a terminal, or amdui's AI tab):\n  claude mcp add --scope user agile-md -- amd mcp"
        );
        assert!(setup(Some("emacs")).is_err());
    }
}
