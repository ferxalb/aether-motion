//! `doctor`: report the state of the installation, the current project, and agent setup, grouped
//! into sections with a fix for each problem and a one-line verdict at the end.

use super::help;
use super::project;
use super::ui::{display_path, heading, line, Mark, Style};
use crate::agent;
use crate::setup::{mcp_config, skills};
use anyhow::Result;
use std::path::Path;
use std::process::{Command, Stdio};

/// Collects lines per section and counts the problems.
struct Report {
    style: Style,
    problems: usize,
}

impl Report {
    fn check(&mut self, ok: bool, subject: &str, detail: &str, fix: &str) {
        if ok {
            println!("{}", line(self.style, Mark::Ok, subject, detail));
        } else {
            self.problems += 1;
            println!("{}", line(self.style, Mark::Fail, subject, detail));
            println!("      {} {fix}", self.style.cyan("fix:"));
        }
    }

    fn section(&self, title: &str) {
        println!("\n{}", heading(self.style, title));
    }
}

/// CPU, instruction sets, caches, and container limits, as the renderer sees them.
fn system_section(report: &Report) {
    let style = report.style;
    report.section("System");
    let cpu = crate::system::cpu();
    let name = [cpu.vendor.as_str(), cpu.brand.as_str()]
        .iter()
        .rev()
        .find(|s| !s.is_empty())
        .map_or("CPU".to_string(), |s| s.to_string());
    let cores = match (cpu.logical, cpu.physical) {
        (Some(l), Some(p)) => format!("{l} CPUs available, {p} physical cores"),
        (Some(l), None) => format!("{l} CPUs available"),
        _ => String::new(),
    };
    println!("{}", line(style, Mark::Info, &name, &cores));
    let features = cpu.feature_names();
    if !features.is_empty() {
        println!(
            "{}",
            line(style, Mark::Info, "instruction sets", &features.join(" "))
        );
    }
    let kib = |bytes: Option<u64>| bytes.map(|b| format!("{} KiB", b / 1024));
    let caches: Vec<String> = [("L1d", cpu.l1d), ("L2", cpu.l2), ("L3", cpu.l3)]
        .into_iter()
        .filter_map(|(level, bytes)| kib(bytes).map(|size| format!("{level} {size}")))
        .collect();
    if !caches.is_empty() {
        println!("{}", line(style, Mark::Info, "caches", &caches.join(", ")));
    }
    let mut limits = Vec::new();
    if let Some(cpus) = crate::system::cpu_limit() {
        limits.push(format!("{cpus:.2} CPUs"));
    }
    if let Some(bytes) = crate::system::memory_limit() {
        limits.push(format!("{} MiB of memory", bytes / (1024 * 1024)));
    }
    let detail = if limits.is_empty() {
        "none".to_string()
    } else {
        limits.join(", ")
    };
    println!("{}", line(style, Mark::Info, "container limits", &detail));
    if !crate::system::NATIVE {
        println!(
            "{}",
            line(
                style,
                Mark::Info,
                "native host layer",
                "not built (feature native-system)"
            )
        );
    }
}

pub fn run() -> Result<()> {
    let style = Style::out();
    let mut report = Report { style, problems: 0 };
    println!("{}", style.bold(&help::version()));

    report.section("Installation");
    let binary = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    println!("{}", line(style, Mark::Ok, "aether-motion", &binary));
    let ffmpeg = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok();
    println!(
        "{}",
        line(
            style,
            Mark::Info,
            &format!("ffmpeg {}", if ffmpeg { "found" } else { "not found" }),
            "optional, only for --encoder ffmpeg",
        )
    );

    system_section(&report);

    report.section("Project");
    let root = Path::new(".");
    match project::resolve(root, None) {
        Err(e) => report.check(
            false,
            "composition",
            &format!("{e:#}"),
            "run `aether-motion init`, or pass a file to the commands",
        ),
        Ok(p) => {
            let checked = agent::check(&p.entry);
            let name = display_path(&p.entry);
            if checked["ok"] == true {
                report.check(
                    true,
                    &name,
                    &format!(
                        "valid · {:.1} s · {}x{}",
                        checked["duration"].as_f64().unwrap_or(0.0),
                        checked["width"],
                        checked["height"]
                    ),
                    "",
                );
            } else {
                report.check(
                    false,
                    &name,
                    checked["error"].as_str().unwrap_or("check failed"),
                    checked["fix"]
                        .as_str()
                        .unwrap_or("run `aether-motion check` for the full report"),
                );
            }
        }
    }

    report.section("Coding agents");
    for target in skills::targets() {
        report.check(
            skills::installed(root, target),
            &format!("skills in {target}"),
            "",
            "run `aether-motion agent setup`",
        );
    }
    let mcp = mcp_config::status(root);
    if mcp.is_empty() {
        report.check(
            false,
            "MCP config",
            "none found",
            "run `aether-motion agent setup`",
        );
    }
    for (name, ok) in mcp {
        report.check(
            ok,
            &format!("MCP server for {name}"),
            "",
            "run `aether-motion agent setup`",
        );
    }

    println!();
    match report.problems {
        0 => println!("{}", style.green("Everything is set up.")),
        1 => println!("{}", style.yellow("1 problem found.")),
        n => println!("{}", style.yellow(&format!("{n} problems found."))),
    }
    Ok(())
}
