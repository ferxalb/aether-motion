//! `preview`: render one frame to `.aether/preview.png`, open it, and optionally re-render on edits.
//!
//! The watcher follows every file the last successful parse read (`Composition::dependencies`:
//! the entry, imported scenes, and their assets) plus the variables file.

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

/// Renders the frame at `at` seconds (clamped to the composition) and returns its time and the
/// files the composition read.
fn render_once(
    entry: &Path,
    values: &VariableValues,
    at: f64,
    fps: u32,
    out: &Path,
) -> Result<(f64, Vec<PathBuf>)> {
    let composition = parse_file_with(entry, values)?;
    let mut renderer = Renderer::new(&composition, 1.0)?;
    let frames = renderer.frame_count(fps);
    let frame = ((at * fps as f64).round() as usize).min(frames.saturating_sub(1));
    write_png(&mut renderer, out, frame, fps)?;
    Ok((frame as f64 / fps as f64, composition.dependencies.clone()))
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
/// With `--watch`, edits to the composition, to any scene or asset it read, or to the variables
/// file trigger a new render. A failed parse keeps watching the files of the last good one.
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
    let variables_file = args.value("--variables-file").map(PathBuf::from);
    let watched = |files: &[PathBuf]| -> Vec<Option<SystemTime>> {
        std::iter::once(&project.entry)
            .chain(files)
            .chain(&variables_file)
            .map(|path| modified(path))
            .collect()
    };
    let at: f64 = args.number("--at")?.unwrap_or(1.0);
    if at < 0.0 {
        bail!("--at must be 0 or more seconds");
    }
    let out = PathBuf::from(OUTPUT);
    fs::create_dir_all(out.parent().unwrap())?;
    let (time, mut files) = render_once(&project.entry, &values()?, at, project.fps, &out)?;
    println!("Preview at {time:.2} s: {}", out.display());
    if !args.has("--no-open") {
        open(&out);
    }
    if !args.has("--watch") {
        return Ok(());
    }
    println!(
        "Watching {} and {} files it reads (Ctrl-C to stop)",
        project.entry.display(),
        files.len().saturating_sub(1)
    );
    let mut seen = watched(&files);
    loop {
        sleep(Duration::from_millis(300));
        let now = watched(&files);
        if now == seen {
            continue;
        }
        match values()
            .and_then(|values| render_once(&project.entry, &values, at, project.fps, &out))
        {
            Ok((t, read)) => {
                println!("Updated ({t:.2} s)");
                // Edits made while rendering are compared against the times read before it.
                seen = if read == files { now } else { watched(&read) };
                files = read;
            }
            Err(e) => {
                println!("Error: {e:#}");
                seen = now;
            }
        }
    }
}
