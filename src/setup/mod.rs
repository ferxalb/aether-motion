//! Agent setup for a project: embedded skills and MCP server registration.
//!
//! `skills` writes the bundled Agent Skills into `.claude/skills` and `.agents/skills`; `mcp_config`
//! detects which coding agents a project uses and merges the `aether-motion` server into their
//! project-scoped MCP config files. Nothing here touches user-global configuration.

pub mod mcp_config;
pub mod skills;

use anyhow::Result;
use std::path::Path;

/// Outcome of one setup step, printed by `init` and `agent setup`.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Created(String),
    Updated(String),
    Unchanged(String),
    Skipped(String, String),
}

impl Step {
    pub fn line(&self) -> String {
        match self {
            Step::Created(p) => format!("  + {p}"),
            Step::Updated(p) => format!("  ~ {p}"),
            Step::Unchanged(p) => format!("  = {p} (already set up)"),
            Step::Skipped(p, why) => format!("  ! {p} skipped: {why}"),
        }
    }
}

/// Installs skills and MCP config for the agents detected in `root` (all of them with `all`).
pub fn setup_agents(root: &Path, all: bool, force: bool) -> Result<Vec<Step>> {
    let mut steps = skills::install(root, force)?;
    steps.extend(mcp_config::install(root, all)?);
    Ok(steps)
}
