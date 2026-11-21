//! `init`: scaffold a ready-to-render project and wire up coding agents in one command.

use super::agent_cmd::print_steps;
use super::args::{usage, Args};
use super::project::CONFIG;
use super::ui::{heading, line, Mark, Style};
use crate::agent;
use crate::setup::{self, Step};
use anyhow::Result;
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
    Ok(if existed {
        Step::Updated(rel.to_string())
    } else {
        Step::Created(rel.to_string())
    })
}

/// `init [dir] [--template promo|blank] [--no-agent] [--all] [--force]`.
pub fn run(raw: &[String]) -> Result<()> {
    let args = Args::parse(
        raw,
        &["--template"],
        &["--no-agent", "--all", "--force", "--yes"],
    )?;
    if args.positional.len() > 1 {
        return Err(usage(format!(
            "init takes one directory, got {}",
            args.positional.join(" ")
        )));
    }
    let dir = args.positional.first().map_or(".", String::as_str);
    let force = args.has("--force");
    let template = match args.value("--template").unwrap_or("promo") {
        "promo" => PROMO,
        "blank" => BLANK,
        other => {
            return Err(usage(format!(
                "unknown template: {other} (use promo or blank)"
            )))
        }
    };
    let root = Path::new(dir);
    fs::create_dir_all(root)?;
    let files = vec![
        write_file(root, CONFIG, CONFIG_JSON, force)?,
        write_file(root, "index.html", template, force)?,
        write_file(root, "assets/.gitkeep", "", force)?,
        write_file(root, "AGENTS.md", AGENTS_MD, force)?,
        write_file(root, ".gitignore", GITIGNORE, force)?,
    ];
    let agents = if args.has("--no-agent") {
        Vec::new()
    } else {
        setup::setup_agents(root, args.has("--all"), force)?
    };
    let style = Style::out();
    let place = if dir == "." {
        "the current directory".to_string()
    } else {
        dir.to_string()
    };
    println!("{}\n", style.bold(&format!("Project ready in {place}")));
    print_steps("Project files", &files);
    if !agents.is_empty() {
        println!();
        print_steps("Agent setup", &agents);
    }
    println!();
    let report = agent::check(&root.join("index.html"));
    if report["ok"] == true {
        println!(
            "{}",
            line(
                style,
                Mark::Ok,
                "index.html is valid",
                &format!(
                    "{:.1} s · {}x{}",
                    report["duration"].as_f64().unwrap_or(0.0),
                    report["width"],
                    report["height"]
                ),
            )
        );
    } else {
        println!(
            "{}",
            line(
                style,
                Mark::Fail,
                "index.html does not pass check",
                report["error"].as_str().unwrap_or("check failed"),
            )
        );
    }
    println!("\n{}", heading(style, "Next steps"));
    let cmd = crate::setup::mcp_config::command_prefix();
    let mut next: Vec<(String, &str)> = Vec::new();
    if dir != "." {
        next.push((format!("cd {dir}"), ""));
    }
    next.push((
        format!("{cmd} preview"),
        "Look at a frame (add --watch to follow edits)",
    ));
    next.push((format!("{cmd} render"), "Export out/index.mp4"));
    next.push((format!("{cmd} help"), "Every command"));
    let pad = next
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    for (command, what) in next {
        if what.is_empty() {
            println!("  {}", style.bold(&command));
        } else {
            let gap = " ".repeat(pad - command.chars().count() + 2);
            println!("  {}{gap}{}", style.bold(&command), style.dim(what));
        }
    }
    Ok(())
}
