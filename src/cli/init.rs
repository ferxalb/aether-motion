//! `init`: scaffold a ready-to-render project and wire up coding agents in one command.

use super::args::Args;
use super::project::CONFIG;
use crate::agent;
use crate::setup::{self, Step};
use anyhow::{bail, Result};
use std::fs;
use std::path::Path;

const PROMO: &str = include_str!("../../templates/index.html");
const BLANK: &str = include_str!("../../templates/blank.html");
const AGENTS_MD: &str = include_str!("../../templates/AGENTS.md");
const GITIGNORE: &str = include_str!("../../templates/gitignore");
const CONFIG_JSON: &str = "{\n  \"entry\": \"index.html\",\n  \"fps\": 30,\n  \"size\": \"1080p\",\n  \"out\": \"out\"\n}\n";

/// Writes `content` unless the file exists (and `force` is off).
fn write_file(root: &Path, rel: &str, content: &str, force: bool) -> Result<Step> {
    let path = root.join(rel);
    let existed = path.exists();
    if existed && !force {
        return Ok(Step::Unchanged(rel.to_string()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(if existed { Step::Updated(rel.to_string()) } else { Step::Created(rel.to_string()) })
}

/// `init [dir] [--template promo|blank] [--no-agent] [--all] [--force]`.
pub fn run(raw: &[String]) -> Result<()> {
    let args = Args::parse(raw, &["--template"], &["--no-agent", "--all", "--force", "--yes"])?;
    let dir = args.positional.first().map_or(".", String::as_str);
    let force = args.has("--force");
    let template = match args.value("--template").unwrap_or("promo") {
        "promo" => PROMO,
        "blank" => BLANK,
        other => bail!("unknown template: {other} (use promo or blank)"),
    };
    let root = Path::new(dir);
    fs::create_dir_all(root)?;
    let mut steps = vec![
        write_file(root, CONFIG, CONFIG_JSON, force)?,
        write_file(root, "index.html", template, force)?,
        write_file(root, "assets/.gitkeep", "", force)?,
        write_file(root, "AGENTS.md", AGENTS_MD, force)?,
        write_file(root, ".gitignore", GITIGNORE, force)?,
    ];
    if !args.has("--no-agent") {
        steps.extend(setup::setup_agents(root, args.has("--all"), force)?);
    }
    println!("Project ready in {}", if dir == "." { "the current directory" } else { dir });
    for step in &steps {
        println!("{}", step.line());
    }
    let report = agent::check(&root.join("index.html"));
    if report["ok"] == true {
        println!("  ok  index.html is valid ({} s, {}x{})", report["duration"], report["width"], report["height"]);
    } else {
        println!("  !   index.html: {}", report["error"].as_str().unwrap_or("check failed"));
    }
    println!("\nNext:");
    if dir != "." {
        println!("  cd {dir}");
    }
    let cmd = crate::setup::mcp_config::command_prefix();
    println!("  {cmd} preview     # look at a frame");
    println!("  {cmd} render      # export out/index.mp4");
    Ok(())
}
