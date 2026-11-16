//! `add`: list ready-made blocks, or append one to the project's composition.

use super::project;
use crate::blocks;
use anyhow::{anyhow, bail, Result};
use std::fs;
use std::path::Path;

/// `add` lists blocks; `add <block> [--set k=v]...` inserts the block before the root's closing tag.
pub fn run(raw: &[String]) -> Result<()> {
    if raw.is_empty() {
        print!("{}", blocks::run_cli(&[])?);
        return Ok(());
    }
    if raw[0].starts_with("theme-") {
        bail!(
            "themes are not inserted automatically; print one with `aether-motion block {}`",
            raw[0]
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
    println!("Added {} to {}", raw[0], project.entry.display());
    Ok(())
}
