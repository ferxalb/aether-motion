//! `doctor`: report the state of the installation and the current project, with a hint per problem.

use super::project;
use crate::agent;
use crate::setup::{mcp_config, skills};
use anyhow::Result;
use std::path::Path;
use std::process::{Command, Stdio};

fn line(ok: bool, text: &str, hint: &str) {
    if ok {
        println!("  ok    {text}");
    } else {
        println!("  fix   {text}: {hint}");
    }
}

pub fn run() -> Result<()> {
    println!("AETHER Motion {}", env!("CARGO_PKG_VERSION"));
    let ffmpeg = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    println!(
        "  info  ffmpeg: {} (optional, only for --encoder ffmpeg)",
        if ffmpeg { "found" } else { "not found" }
    );
    let root = Path::new(".");
    match project::resolve(root, None) {
        Err(e) => line(false, "project", &format!("{e:#}")),
        Ok(p) => {
            let report = agent::check(&p.entry);
            let ok = report["ok"] == true;
            let hint = format!(
                "{} ({})",
                report["error"].as_str().unwrap_or(""),
                report["fix"].as_str().unwrap_or("")
            );
            line(ok, &format!("composition {}", p.entry.display()), &hint);
        }
    }
    for target in skills::targets() {
        line(
            skills::installed(root, target),
            &format!("skills in {target}"),
            "run `aether-motion agent setup`",
        );
    }
    let mcp = mcp_config::status(root);
    if mcp.is_empty() {
        line(false, "MCP config", "run `aether-motion agent setup`");
    }
    for (name, ok) in mcp {
        line(
            ok,
            &format!("MCP server for {name}"),
            "run `aether-motion agent setup`",
        );
    }
    Ok(())
}
