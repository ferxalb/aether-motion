//! `preview`: render one frame to `.aether/preview.png`, open it, and optionally re-render on edits.
//!
//! The watcher follows every file the last successful parse read (`Composition::dependencies`:
//! the entry, imported scenes, and their assets) plus the variables file. After a failure it also
//! follows every path the failed attempt read or tried to read, so creating a missing asset
//! triggers a new render. Compiled scenes are kept between renders in a persistent
//! [`SceneCache`]: a scene is compiled again only when one of the files it read (including those
//! of the scenes it imports) changed.

use super::args::Args;
use super::project;
use super::render_cmd::write_png;
use crate::parse::{parse_file_cached, SceneCache, VariableValues};
use crate::render::Renderer;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, SystemTime};

const OUTPUT: &str = ".aether/preview.png";

/// Renders the frame at `at` seconds (clamped to the composition) and returns its time, with the
/// files to watch next: those the composition read when it compiled (even if drawing then
/// failed, for example on a missing image), or else every path the failed compile tried to read.
fn render_once(
    entry: &Path,
    values: &VariableValues,
    at: f64,
    fps: u32,
    out: &Path,
    cache: &mut SceneCache,
) -> (Result<f64>, Vec<PathBuf>) {
    let composition = match parse_file_cached(entry, values, cache) {
        Ok(composition) => composition,
        Err(error) => return (Err(error), cache.last_reads().to_vec()),
    };
    let drawn = Renderer::new(&composition, 1.0).and_then(|mut renderer| {
        let frames = renderer.frame_count(fps);
        let frame = ((at * fps as f64).round() as usize).min(frames.saturating_sub(1));
        write_png(&mut renderer, out, frame, fps)?;
        Ok(frame as f64 / fps as f64)
    });
    (drawn, composition.dependencies)
}

/// Files to watch after a failed render: those of the last good one and those the failed attempt
/// read, sorted and without duplicates.
fn after_failure(good: &[PathBuf], attempted: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut files = attempted;
    files.extend_from_slice(good);
    files.sort();
    files.dedup();
    files
}

/// The times to compare the next poll of `next` against, after a render that started when the
/// paths `before` (the entry, then `before`, then the variables file, as `watched` lists them)
/// had the times `now`: a path watched before keeps its time from before the render, so an edit
/// made during the render is noticed; a newly listed path has no earlier time and gets `None`, so
/// it triggers one follow-up render if it exists (or appears) by the next poll.
fn baseline(
    before: &[PathBuf],
    now: &[Option<SystemTime>],
    next: &[PathBuf],
) -> Vec<Option<SystemTime>> {
    let earlier: HashMap<&PathBuf, Option<SystemTime>> =
        before.iter().zip(&now[1..]).map(|(p, t)| (p, *t)).collect();
    std::iter::once(now[0])
        .chain(next.iter().map(|path| earlier.get(path).copied().flatten()))
        .chain(now.get(1 + before.len()).copied())
        .collect()
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
    let mut cache = SceneCache::persistent();
    let (time, mut files) = match render_once(
        &project.entry,
        &values()?,
        at,
        project.fps,
        &out,
        &mut cache,
    ) {
        (Ok(time), files) => (time, files),
        (Err(error), _) => return Err(error),
    };
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
    // The files of the last good render; after a failure `files` also holds the attempted ones.
    let mut good = files.clone();
    let mut seen = watched(&files);
    loop {
        sleep(Duration::from_millis(300));
        let now = watched(&files);
        if now == seen {
            continue;
        }
        let (result, read) = match values() {
            Ok(values) => render_once(&project.entry, &values, at, project.fps, &out, &mut cache),
            Err(error) => (Err(error), Vec::new()),
        };
        let read = match result {
            Ok(t) => {
                println!("Updated ({t:.2} s)");
                good = read.clone();
                read
            }
            Err(e) => {
                println!("Error: {e:#}");
                after_failure(&good, read)
            }
        };
        // Edits made while rendering are compared against the times read before it.
        seen = if read == files {
            now
        } else {
            baseline(&files, &now, &read)
        };
        files = read;
    }
}

#[cfg(test)]
mod tests {
    use super::{after_failure, baseline};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    #[test]
    fn a_changed_file_list_keeps_earlier_times_and_marks_new_paths() {
        let t = |s| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(s));
        let [a, b, c] = ["a.html", "b.png", "c.png"].map(PathBuf::from);
        // Entry, a, b, then the variables file.
        let now = [t(1), t(2), t(3), t(4)];
        let next = baseline(&[a.clone(), b.clone()], &now, &[a.clone(), c.clone()]);
        assert_eq!(next, vec![t(1), t(2), None, t(4)]);
        // Without a variables file the list ends with the files.
        let next = baseline(std::slice::from_ref(&a), &now[..2], &[b.clone(), a.clone()]);
        assert_eq!(next, vec![t(1), None, t(2)]);
        assert_eq!(
            after_failure(&[b.clone(), a.clone()], vec![c.clone(), a.clone()]),
            [a, b, c]
        );
    }
}
