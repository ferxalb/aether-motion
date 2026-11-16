//! `agent setup|status`: (re)install skills and MCP config in an existing project.

use super::args::Args;
use super::doctor;
use crate::setup;
use anyhow::{bail, Result};
use std::path::Path;

pub fn run(raw: &[String]) -> Result<()> {
    let args = Args::parse(raw, &[], &["--all", "--force"])?;
    match args.positional.first().map(String::as_str) {
        Some("setup") | None => {
            for step in setup::setup_agents(Path::new("."), args.has("--all"), args.has("--force"))?
            {
                println!("{}", step.line());
            }
            Ok(())
        }
        Some("status") => doctor::run(),
        Some(other) => bail!("unknown agent command: {other} (use setup or status)"),
    }
}
