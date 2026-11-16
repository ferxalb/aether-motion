//! Project-scoped MCP registration: detects coding agents and merges the `aether-motion` server.

use super::Step;
use anyhow::Result;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

/// Agent whose project-level MCP file we know how to write.
struct Agent {
    name: &'static str,
    /// Config path relative to the project root.
    file: &'static str,
    /// Top-level key holding the server map.
    key: &'static str,
    /// Directory whose presence marks the agent as used by the project.
    marker: &'static str,
    /// VS Code requires an explicit `"type": "stdio"`.
    typed: bool,
    /// Always configured, even when not detected.
    default: bool,
}

const AGENTS: [Agent; 3] = [
    Agent { name: "Claude Code", file: ".mcp.json", key: "mcpServers", marker: ".claude", typed: false, default: true },
    Agent { name: "Cursor", file: ".cursor/mcp.json", key: "mcpServers", marker: ".cursor", typed: false, default: false },
    Agent { name: "VS Code", file: ".vscode/mcp.json", key: "servers", marker: ".vscode", typed: true, default: false },
];

const SERVER: &str = "aether-motion";

fn entry(typed: bool) -> Value {
    if typed {
        json!({"type": "stdio", "command": "aether-motion", "args": ["mcp"]})
    } else {
        json!({"command": "aether-motion", "args": ["mcp"]})
    }
}

/// Returns `doc` with the server entry set, or `None` when it already matches.
pub fn merge(doc: &str, key: &str, server: Value) -> Result<Option<String>, String> {
    let mut root: Value = if doc.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(doc).map_err(|e| format!("not valid JSON ({e})"))?
    };
    let obj = root.as_object_mut().ok_or("top level is not an object")?;
    let servers = obj
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| format!("`{key}` is not an object"))?;
    if servers.get(SERVER) == Some(&server) {
        return Ok(None);
    }
    servers.insert(SERVER.to_string(), server);
    Ok(Some(serde_json::to_string_pretty(&root).unwrap() + "\n"))
}

/// Configures every detected agent (all known ones with `all`); invalid files are skipped, not replaced.
pub fn install(root: &Path, all: bool) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for agent in &AGENTS {
        if !(all || agent.default || root.join(agent.marker).is_dir()) {
            continue;
        }
        let path = root.join(agent.file);
        let existing = fs::read_to_string(&path).ok();
        let label = format!("{} ({})", agent.file, agent.name);
        match merge(existing.as_deref().unwrap_or(""), agent.key, entry(agent.typed)) {
            Ok(None) => steps.push(Step::Unchanged(label)),
            Ok(Some(text)) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, text)?;
                steps.push(if existing.is_some() { Step::Updated(label) } else { Step::Created(label) });
            }
            Err(why) => steps.push(Step::Skipped(label, why)),
        }
    }
    Ok(steps)
}

/// Whether the project's config for `agent_file` already registers the server.
pub fn status(root: &Path) -> Vec<(&'static str, bool)> {
    AGENTS
        .iter()
        .filter(|a| root.join(a.file).exists())
        .map(|a| {
            let ok = fs::read_to_string(root.join(a.file))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .is_some_and(|v| v[a.key][SERVER].is_object());
            (a.name, ok)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_other_servers_and_is_idempotent() {
        let doc = r#"{"mcpServers":{"other":{"command":"x"}},"extra":1}"#;
        let merged = merge(doc, "mcpServers", entry(false)).unwrap().unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"][SERVER]["args"][0], "mcp");
        assert_eq!(v["extra"], 1);
        assert_eq!(merge(&merged, "mcpServers", entry(false)).unwrap(), None);
    }

    #[test]
    fn invalid_json_is_rejected_not_replaced() {
        assert!(merge("{oops", "mcpServers", entry(false)).is_err());
        assert!(merge("[]", "mcpServers", entry(false)).is_err());
    }

    #[test]
    fn empty_file_gets_a_fresh_document() {
        let text = merge("", "servers", entry(true)).unwrap().unwrap();
        assert!(text.contains("\"type\": \"stdio\""));
    }
}
