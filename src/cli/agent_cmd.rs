//! `agent setup|status`: (re)install skills and MCP config in an existing project, and the shared
//! printer of setup steps that `init` uses too.

use super::args::{Args, UsageError};
use super::doctor;
use super::ui::{heading, line, Mark, Style};
use crate::setup::{self, Step};
use anyhow::Result;
use std::path::Path;

pub fn run(raw: &[String]) -> Result<()> {
    let args = Args::parse(raw, &[], &["--all", "--force"])?;
    match args.positional.first().map(String::as_str) {
        Some("setup") | None => {
            let steps =
                setup::setup_agents(Path::new("."), args.has("--all"), args.has("--force"))?;
            print_steps("Agent setup", &steps);
            Ok(())
        }
        Some("status") => doctor::run(),
        Some(other) => Err(UsageError::new(format!("unknown agent command: {other}"))
            .with_hint("use `aether-motion agent setup` or `aether-motion agent status`")
            .into()),
    }
}

/// Prints a heading and one marked line per step: `+` created, `~` updated, `=` unchanged,
/// `!` skipped with its reason.
pub fn print_steps(title: &str, steps: &[Step]) {
    let style = Style::out();
    println!("{}", heading(style, title));
    for step in steps {
        let text = match step {
            Step::Created(path) => line(style, Mark::Added, path, "created"),
            Step::Updated(path) => line(style, Mark::Changed, path, "updated"),
            Step::Unchanged(path) => line(style, Mark::Same, path, "already set up"),
            Step::Skipped(path, why) => line(style, Mark::Warn, path, &format!("skipped: {why}")),
        };
        println!("{text}");
    }
}
