//! `preview`: render one frame to `.aether/preview.png`, open it, and optionally re-render on edits.

use super::args::Args;
use super::project;
use super::render_cmd::write_png;
use crate::parse::{parse_file_with, VariableValues};
use crate::render::Renderer;
use anyhow::{bail, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, SystemTime};

const OUTPUT: &str = ".aether/preview.png";

/// Renders the frame at `at` seconds (clamped to the composition) and returns its time.
fn render_once(
    entry: &Path,
    values: &VariableValues,
    at: f64,
    fps: u32,
    out: &Path,
) -> Result<f64> {
    let composition = parse_file_with(entry, values)?;
    let mut renderer = Renderer::new(&composition, 1.0)?;
    let frames = renderer.frame_count(fps);
    let frame = ((at * fps as f64).round() as usize).min(frames.saturating_sub(1));
    write_png(&mut renderer, out, frame, fps)?;
    Ok(frame as f64 / fps as f64)
}

/// Opens `path` with the platform's default viewer; failures are reported, not fatal.
fn open(path: &Path) {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", ""])
    } else {
        ("xdg-open", vec![])
    };
    let launched = Command::new(program)
        .args(args)
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !launched {
        println!("Open {} to see it.", path.display());
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// `preview [file] [--at SECONDS] [--variables JSON | --variables-file PATH] [--watch] [--no-open]`.
///
/// With `--watch`, edits to the composition or to the variables file trigger a new render.
pub fn run(raw: &[String]) -> Result<()> {
    let args = Args::parse(
        raw,
        &["--at", "--variables", "--variables-file"],
        &["--watch", "--no-open"],
    )?;
    let project = project::resolve(Path::new("."), args.positional.first().map(String::as_str))?;
    let values = || {
        super::variables::values(
            args.value("--variables"),
            args.value("--variables-file"),
            &project.variables,
        )
    };
    let watched = || {
        (
            modified(&project.entry),
            args.value("--variables-file")
                .and_then(|f| modified(Path::new(f))),
        )
    };
    let at: f64 = args.number("--at")?.unwrap_or(1.0);
    if at < 0.0 {
        bail!("--at must be 0 or more seconds");
    }
    let out = PathBuf::from(OUTPUT);
    fs::create_dir_all(out.parent().unwrap())?;
    let time = render_once(&project.entry, &values()?, at, project.fps, &out)?;
    println!("Preview at {time:.2} s: {}", out.display());
    if !args.has("--no-open") {
        open(&out);
    }
    if !args.has("--watch") {
        return Ok(());
    }
    println!("Watching {} (Ctrl-C to stop)", project.entry.display());
    let mut seen = watched();
    loop {
        sleep(Duration::from_millis(300));
        let now = watched();
        if now == seen {
            continue;
        }
        seen = now;
        match values()
            .and_then(|values| render_once(&project.entry, &values, at, project.fps, &out))
        {
            Ok(t) => println!("Updated ({t:.2} s)"),
            Err(e) => println!("Error: {e:#}"),
        }
    }
}
