//! `check`: validate the composition, then probe text against safe zones; a readable summary on a
//! terminal, JSON when piped or `--json`.
//!
//! Zone flags: `--safe-zone tiktok|reels|shorts|standard|off` (comma list allowed; default is the
//! three platform zones on 9:16 frames and `standard` elsewhere), `--caption-zone "x0=..;y0=..;x1=..;y1=.."`
//! (optional `severity=error`), `--samples N` (default 9 midpoints), `--at t1,t2`, and `--strict`.
//! `--variables JSON` or `--variables-file PATH` check the composition with those variable values.
//! Exit codes: 0 success, 1 the composition is invalid or a gating finding exists, 2 usage error.
//! Safe-zone findings are warnings and never fail, even with `--strict`.

use super::args::{usage as usage_error, Args};
use super::project;
use super::ui::{self, display_path, Mark, Style};
use crate::agent::{self, zones};
use anyhow::Result;
use serde_json::Value;
use std::io::IsTerminal;
use std::path::Path;

/// Returns the process exit code (0 when the composition is valid).
pub fn run(raw: &[String]) -> Result<i32> {
    let parsed = Args::parse(
        raw,
        &[
            "--safe-zone",
            "--caption-zone",
            "--samples",
            "--at",
            "--variables",
            "--variables-file",
        ],
        &["--json", "--strict"],
    );
    let args = match parsed {
        Ok(args) => args,
        Err(error) => return usage(error),
    };
    let file = match args.file("check") {
        Ok(file) => file,
        Err(error) => return usage(error),
    };
    // A named file is checked standalone; without one, aether.json supplies entry and variables.
    let project = project::select(file)?;
    // Values that cannot be read are usage errors; values the composition rejects fail the check.
    let values = match super::variables::values(
        args.value("--variables"),
        args.value("--variables-file"),
        &project.variables,
    ) {
        Ok(values) => values,
        Err(error) => return usage(error),
    };
    let mut report = agent::check_with(&project.entry, &values);
    let ok = report["ok"] == true;
    let mut exit = i32::from(!ok);
    let mut findings: Vec<Value> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    if ok {
        let request = (|| -> Result<zones::ZoneRequest> {
            let at = match args.value("--at") {
                Some(list) => list
                    .split(',')
                    .map(|t| {
                        t.trim().parse::<f64>().map_err(|_| {
                            usage_error(format!("--at must be a comma list of seconds: {t}"))
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                None => Vec::new(),
            };
            Ok(zones::ZoneRequest {
                variables: Some(&values),
                safe_zone: args.value("--safe-zone"),
                caption_zone: args.value("--caption-zone"),
                samples: args.number::<usize>("--samples")?,
                at,
            })
        })();
        let request = match request {
            Ok(request) => request,
            Err(error) => return usage(error),
        };
        // Zone flags that do not parse are usage errors; probing failures are real errors.
        let (width, height) = (
            report["width"].as_u64().unwrap_or(0) as u32,
            report["height"].as_u64().unwrap_or(0) as u32,
        );
        if let Err(error) = request.plan(width, height, report["duration"].as_f64().unwrap_or(0.0))
        {
            return usage(error);
        }
        if let Some(outcome) = zones::attach(&mut report, &project.entry, &request)? {
            findings.extend(outcome.findings.iter().cloned());
            notes.extend(outcome.notes.iter().cloned());
            if outcome.caption_errors || (args.has("--strict") && outcome.caption_collisions > 0) {
                exit = 1;
            }
        }
    }
    if args.has("--json") || !std::io::stdout().is_terminal() {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        summary(&project.entry, &report, &findings, &notes, exit);
    }
    Ok(exit)
}

/// The terminal view of a check: verdict, composition facts, then layout findings and notes.
fn summary(entry: &Path, report: &Value, findings: &[Value], notes: &[String], exit: i32) {
    let style = Style::out();
    let name = display_path(entry);
    if report["ok"] != true {
        println!("{}", ui::line(style, Mark::Fail, &name, "invalid"));
        println!(
            "    {}  {}",
            style.red("error"),
            report["error"].as_str().unwrap_or("unknown")
        );
        if let Some(fix) = report["fix"].as_str() {
            println!("    {}    {fix}", style.cyan("fix"));
        }
        return;
    }
    let verdict = if exit == 0 { Mark::Ok } else { Mark::Fail };
    let word = if exit == 0 {
        "valid"
    } else {
        "valid, but a caption-zone finding fails the check"
    };
    println!("{}", ui::line(style, verdict, &name, word));
    for row in ui::rows(
        style,
        4,
        &[
            (
                "Canvas",
                format!("{}x{}", report["width"], report["height"]),
            ),
            (
                "Duration",
                format!("{:.1} s", report["duration"].as_f64().unwrap_or(0.0)),
            ),
            ("Elements", report["elements"].to_string()),
        ],
    ) {
        println!("{row}");
    }
    if !findings.is_empty() {
        let label = if findings.len() == 1 {
            "finding"
        } else {
            "findings"
        };
        println!(
            "\n{}",
            ui::heading(style, &format!("Layout · {} {label}", findings.len()))
        );
        for finding in findings {
            let mark = match finding["severity"].as_str() {
                Some("error") => Mark::Fail,
                _ => Mark::Warn,
            };
            let subject = format!(
                "{} {}",
                finding["code"].as_str().unwrap_or(""),
                finding["element"].as_str().unwrap_or("")
            );
            let detail = format!(
                "\"{}\" at {} s",
                finding["text"].as_str().unwrap_or(""),
                finding["time"]
            );
            println!("{}", ui::line(style, mark, subject.trim_end(), &detail));
        }
    }
    for note in notes {
        println!("{}", ui::line(style, Mark::Info, note, ""));
    }
}

/// Bad flags or values: print the problem and exit with code 2.
fn usage(error: anyhow::Error) -> Result<i32> {
    let error = match error.downcast::<super::args::UsageError>() {
        Ok(usage) => usage.into(),
        Err(other) => usage_error(format!("{other:#}")),
    };
    Ok(super::print_error(&error, Some("check")))
}
