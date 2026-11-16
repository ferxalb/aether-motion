//! `check`: validate the composition; a readable summary on a terminal, JSON when piped or `--json`.

use super::args::Args;
use super::project;
use crate::agent;
use anyhow::Result;
use std::io::IsTerminal;
use std::path::Path;

/// Returns the process exit code (0 when the composition is valid).
pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(raw, &[], &["--json"])?;
    let project = project::resolve(Path::new("."), args.positional.first().map(String::as_str))?;
    let report = agent::check(&project.entry);
    let ok = report["ok"] == true;
    if args.has("--json") || !std::io::stdout().is_terminal() {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if ok {
        println!(
            "ok: {} | {}x{} | {:.1} s | {} elements",
            project.entry.display(),
            report["width"],
            report["height"],
            report["duration"].as_f64().unwrap_or(0.0),
            report["elements"]
        );
    } else {
        println!("error: {}", report["error"].as_str().unwrap_or("unknown"));
        if let Some(fix) = report["fix"].as_str() {
            println!("fix:   {fix}");
        }
    }
    Ok(if ok { 0 } else { 1 })
}
