//! `add`: list ready-made blocks, or append one to the project's composition.

use super::args::UsageError;
use super::ui::{display_path, line, Mark, Style};
use super::{block_cmd, project};
use crate::blocks;
use anyhow::{anyhow, Result};
use std::fs;
use std::path::Path;

/// `add` and `add list` list blocks; `add <block> [--set k=v]...` inserts a block.
pub fn run(raw: &[String]) -> Result<()> {
    if raw.is_empty() || raw == ["list"] {
        print!("{}", block_cmd::listing(Style::out())?);
        return Ok(());
    }
    if raw[0] == "list" {
        return Err(UsageError::new("add list takes no arguments").into());
    }
    if raw[0].starts_with("theme-") {
        return Err(
            UsageError::new(format!("{} is a theme; themes are not inserted", raw[0]))
                .with_hint(format!(
                    "print it with `aether-motion block {}` and paste it into the composition",
                    raw[0]
                ))
                .into(),
        );
    }
    let html = blocks::run_cli(raw)?;
    let project = project::resolve(Path::new("."), None)?;
    let source = fs::read_to_string(&project.entry)?;
    // Blocks go at the end of the root container: just before the last `</div>` in the body.
    let at = source
        .rfind("</div>")
        .ok_or_else(|| anyhow!("no root </div> found in {}", project.entry.display()))?;
    let mut out = String::with_capacity(source.len() + html.len());
    out.push_str(&source[..at]);
    out.push_str(&html);
    if !html.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&source[at..]);
    fs::write(&project.entry, out)?;
    println!(
        "{}",
        line(
            Style::out(),
            Mark::Ok,
            &format!("Added {} to {}", raw[0], display_path(&project.entry)),
            "",
        )
    );
    Ok(())
}
