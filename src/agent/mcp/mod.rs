//! MCP stdio server facade. `protocol.rs` handles JSON-RPC envelopes and version negotiation,
//! `tools.rs` the `aether_*` tool catalog and handlers, `progress.rs` render progress notifications.
//! `serve` runs the newline-delimited stdio loop; `run` is the CLI entry that also explains
//! itself when started by hand in a terminal and offers a `--selftest` handshake check.
mod progress;
mod protocol;
mod tools;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal, Read, Write};

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
    let mut input = std::io::stdin().lock();
    let mut buffer = Vec::new();
    loop {
        let parsed = match read_message(&mut input, &mut buffer)? {
            Message::End => break,
            Message::TooLarge => Err(format!(
                "message exceeds the {} MiB limit",
                MAX_MESSAGE_BYTES >> 20
            )),
            Message::Line => match std::str::from_utf8(&buffer) {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => serde_json::from_str::<Value>(line).map_err(|e| e.to_string()),
                Err(error) => Err(format!("message is not UTF-8: {error}")),
            },
        };
        let request = match parsed {
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

/// Largest accepted JSON-RPC message (one line). A composition sent to `aether_write` is a few
/// hundred KiB at most; the cap keeps a runaway or hostile client from growing memory without bound.
const MAX_MESSAGE_BYTES: u64 = 32 * 1024 * 1024;

enum Message {
    /// `buffer` holds one line without its terminator.
    Line,
    /// The line was longer than [`MAX_MESSAGE_BYTES`]; the rest of it was skipped.
    TooLarge,
    /// End of input.
    End,
}

/// Reads one newline-delimited message into `buffer`, never holding more than
/// [`MAX_MESSAGE_BYTES`] of it in memory.
fn read_message(input: &mut impl BufRead, buffer: &mut Vec<u8>) -> std::io::Result<Message> {
    buffer.clear();
    let read = input
        .by_ref()
        .take(MAX_MESSAGE_BYTES + 1)
        .read_until(b'\n', buffer)?;
    if read == 0 {
        return Ok(Message::End);
    }
    if buffer.last() == Some(&b'\n') {
        buffer.pop();
        if buffer.last() == Some(&b'\r') {
            buffer.pop();
        }
        return Ok(Message::Line);
    }
    if (buffer.len() as u64) <= MAX_MESSAGE_BYTES {
        // The final line had no terminator.
        return Ok(Message::Line);
    }
    // Discard the remainder of the oversized line without buffering it.
    buffer.clear();
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        if let Some(end) = chunk.iter().position(|&b| b == b'\n') {
            input.consume(end + 1);
            break;
        }
        let len = chunk.len();
        input.consume(len);
    }
    Ok(Message::TooLarge)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_message_splits_lines_and_skips_oversized_ones() {
        let big = "x".repeat(MAX_MESSAGE_BYTES as usize + 10);
        let data = format!("{{\"a\":1}}\r\n{big}\nnext\nlast");
        let mut input = std::io::Cursor::new(data.into_bytes());
        let mut buffer = Vec::new();
        assert!(matches!(
            read_message(&mut input, &mut buffer).unwrap(),
            Message::Line
        ));
        assert_eq!(buffer, b"{\"a\":1}");
        assert!(matches!(
            read_message(&mut input, &mut buffer).unwrap(),
            Message::TooLarge
        ));
        assert!(matches!(
            read_message(&mut input, &mut buffer).unwrap(),
            Message::Line
        ));
        assert_eq!(buffer, b"next");
        assert!(matches!(
            read_message(&mut input, &mut buffer).unwrap(),
            Message::Line
        ));
        assert_eq!(buffer, b"last");
        assert!(matches!(
            read_message(&mut input, &mut buffer).unwrap(),
            Message::End
        ));
    }
}
