//! MCP stdio server facade. `protocol.rs` handles JSON-RPC envelopes and version negotiation,
//! `tools.rs` the `aether_*` tool catalog and handlers, `progress.rs` render progress notifications.
//! `serve` runs the newline-delimited stdio loop; `run` is the CLI entry that also explains
//! itself when started by hand in a terminal and offers a `--selftest` handshake check.
mod progress;
mod protocol;
mod tools;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal, Write};

/// Serves newline-delimited MCP JSON-RPC messages over stdin and stdout. Startup and malformed
/// input are reported on stderr (never stdout, which carries only protocol messages), where
/// clients show server logs.
pub fn serve() -> Result<()> {
    eprintln!(
        "aether-motion MCP server {} ready on stdio (protocol {}, {} tools)",
        env!("CARGO_PKG_VERSION"),
        protocol::PROTOCOL_VERSION,
        tools::tool_names().count()
    );
    let mut output = std::io::stdout();
    let mut legacy_initialized = false;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Value>(&line) {
            Ok(request) => request,
            Err(error) => {
                let response = json!({"jsonrpc": "2.0", "id": null, "error": protocol::rpc_error(-32700, format!("parse error: {error}"), None)});
                writeln!(output, "{response}")?;
                output.flush()?;
                continue;
            }
        };
        if let Some(response) = protocol::process_request(&request, &mut legacy_initialized) {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// Handshake check without a client: runs discovery, the legacy `initialize`, and `tools/list`
/// in-process and prints what a client would see.
fn selftest() -> Result<()> {
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": protocol::PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let ask = |id: u32, method: &str, params: Value, legacy: &mut bool| {
        protocol::process_request(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
            legacy,
        )
        .ok_or_else(|| anyhow!("{method}: no response"))
    };
    let mut legacy = false;
    let discover = ask(1, "server/discover", json!({"_meta": meta}), &mut legacy)?;
    let modern = ask(2, "tools/list", json!({"_meta": meta}), &mut legacy)?;
    let init = ask(
        3,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "selftest", "version": "0"}}),
        &mut legacy,
    )?;
    let legacy_list = ask(4, "tools/list", json!({}), &mut legacy)?;
    let count = |response: &Value| response["result"]["tools"].as_array().map_or(0, Vec::len);
    if discover.get("error").is_some()
        || init.get("error").is_some()
        || count(&modern) == 0
        || count(&modern) != count(&legacy_list)
    {
        return Err(anyhow!("MCP handshake failed: {discover} {init}"));
    }
    println!(
        "ok: protocol {} (stateless) and {} (legacy initialize), {} tools:",
        protocol::PROTOCOL_VERSION,
        init["result"]["protocolVersion"].as_str().unwrap_or("?"),
        count(&modern)
    );
    for tool in modern["result"]["tools"].as_array().into_iter().flatten() {
        println!(
            "  {:<16} {}",
            tool["name"].as_str().unwrap_or(""),
            tool["title"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

/// What `aether-motion mcp` prints in a terminal instead of waiting silently on stdin.
fn explain_terminal() {
    let prefix = crate::setup::mcp_config::command_prefix();
    eprintln!("aether-motion mcp is an MCP server: it talks JSON-RPC over stdin/stdout to a coding agent,");
    eprintln!(
        "so it is started by the agent, not by hand (a bare run would just wait for input).\n"
    );
    eprintln!("Register it:");
    eprintln!("  claude mcp add aether-motion -- {prefix} mcp     (Claude Code)");
    eprintln!("  {prefix} agent setup --all                      (project config for Claude Code, Cursor, VS Code)\n");
    eprintln!("Verify it:");
    eprintln!("  {prefix} mcp --selftest    handshake and tool list, no client needed");
    eprintln!("  {prefix} doctor            installation, skills, and MCP config\n");
    eprintln!("Force serving in this terminal: {prefix} mcp --stdio");
}

/// CLI entry for `aether-motion mcp [--stdio | --selftest]`.
pub fn run(args: &[String]) -> Result<()> {
    let mut force = false;
    let mut test = false;
    for arg in args {
        match arg.as_str() {
            "--stdio" => force = true,
            "--selftest" => test = true,
            other => return Err(anyhow!("unknown option for mcp: {other}")),
        }
    }
    if test {
        return selftest();
    }
    if !force && std::io::stdin().is_terminal() {
        explain_terminal();
        return Ok(());
    }
    serve()
}
