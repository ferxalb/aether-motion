//! Agent setup for a project: embedded skills and MCP server registration.
//!
//! `skills` writes the bundled Agent Skills into `.claude/skills` and `.agents/skills`; `mcp_config`
//! detects which coding agents a project uses and merges the `aether-motion` server into their
//! project-scoped MCP config files. Nothing here touches user-global configuration.

pub mod mcp_config;
pub mod skills;

use anyhow::Result;
use std::path::Path;

/// Outcome of one setup step; `cli::agent_cmd::print_steps` shows it for `init` and `agent setup`.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Created(String),
    Updated(String),
    Unchanged(String),
    Skipped(String, String),
}

/// Installs skills and MCP config for the agents detected in `root` (all of them with `all`).
pub fn setup_agents(root: &Path, all: bool, force: bool) -> Result<Vec<Step>> {
    let mut steps = skills::install(root, force)?;
    steps.extend(mcp_config::install(root, all)?);
    Ok(steps)
}
