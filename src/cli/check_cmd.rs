//! `check`: validate the composition, then probe text against safe zones; a readable summary on a
//! terminal, JSON when piped or `--json`.
//!
//! Zone flags: `--safe-zone tiktok|reels|shorts|standard|off` (comma list allowed; default is the
//! three platform zones on 9:16 frames and `standard` elsewhere), `--caption-zone "x0=..;y0=..;x1=..;y1=.."`
//! (optional `severity=error`), `--samples N` (default 9 midpoints), `--at t1,t2`, and `--strict`.
//! Exit codes: 0 success, 1 the composition is invalid or a gating finding exists, 2 usage error.
//! Safe-zone findings are warnings and never fail, even with `--strict`.

use super::args::Args;
use super::project;
use crate::agent::{self, zones};
use anyhow::{anyhow, Result};
use serde_json::json;
use std::io::IsTerminal;
use std::path::Path;

/// Returns the process exit code (0 when the composition is valid).
pub fn run(raw: &[String]) -> Result<i32> {
    let parsed = Args::parse(
        raw,
        &["--safe-zone", "--caption-zone", "--samples", "--at"],
        &["--json", "--strict"],
    );
    let args = match parsed {
        Ok(args) => args,
        Err(error) => return usage(error),
    };
    let project = project::resolve(Path::new("."), args.positional.first().map(String::as_str))?;
    let mut report = agent::check(&project.entry);
    let ok = report["ok"] == true;
    let mut exit = i32::from(!ok);
    let mut lines: Vec<String> = Vec::new();
    if ok {
        let (width, height) = (
            report["width"].as_u64().unwrap_or(0) as u32,
            report["height"].as_u64().unwrap_or(0) as u32,
        );
        let planned = (|| -> Result<Option<zones::ZoneOptions>> {
            let requested = zones::parse_zones(args.value("--safe-zone"))?;
            let caption = args
                .value("--caption-zone")
                .map(zones::parse_caption_zone)
                .transpose()?;
            let chosen = requested.unwrap_or_else(|| zones::default_zones(width, height));
            if chosen.is_empty() && caption.is_none() {
                return Ok(None);
            }
            let at = match args.value("--at") {
                Some(list) => list
                    .split(',')
                    .map(|t| {
                        t.trim()
                            .parse::<f64>()
                            .map_err(|_| anyhow!("--at must be a comma list of seconds: {t}"))
                    })
                    .collect::<Result<Vec<_>>>()?,
                None => Vec::new(),
            };
            let samples = args.number::<usize>("--samples")?.unwrap_or(9);
            let duration = report["duration"].as_f64().unwrap_or(0.0);
            Ok(Some(zones::ZoneOptions {
                zones: chosen,
                caption,
                times: zones::sample_times(duration, samples, &at)?,
            }))
        })();
        let options = match planned {
            Ok(options) => options,
            Err(error) => return usage(error),
        };
        if let Some(options) = options {
            let outcome = zones::check_zones(&project.entry, &options)?;
            for finding in &outcome.findings {
                lines.push(format!(
                    "{} {} {}: {} (t={})",
                    finding["severity"].as_str().unwrap_or(""),
                    finding["code"].as_str().unwrap_or(""),
                    finding["element"].as_str().unwrap_or(""),
                    finding["text"].as_str().unwrap_or(""),
                    finding["time"]
                ));
            }
            lines.extend(outcome.notes.iter().map(|n| format!("note: {n}")));
            if outcome.caption_errors || (args.has("--strict") && outcome.caption_collisions > 0) {
                exit = 1;
            }
            report["layout"] = json!({
                "findings": outcome.findings,
                "truncated": outcome.truncated,
                "notes": outcome.notes,
                "samples": outcome.times,
            });
        }
    }
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
        for line in &lines {
            println!("{line}");
        }
    } else {
        println!("error: {}", report["error"].as_str().unwrap_or("unknown"));
        if let Some(fix) = report["fix"].as_str() {
            println!("fix:   {fix}");
        }
    }
    Ok(exit)
}

/// Bad flags or values: print the problem and exit with code 2.
fn usage(error: anyhow::Error) -> Result<i32> {
    eprintln!("error: {error:#}");
    Ok(2)
}
