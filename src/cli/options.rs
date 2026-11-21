//! The one flag parser of `render`, `inspect`, and `bench`.
//!
//! Built on [`Args`], so every spelling of a flag (`-o`/`--output`, `-f`/`--force`) is canonical
//! before anything reads it: an alias can never change behavior. Values are validated here, once,
//! for every command and render mode; settings that may come from `aether.json` stay `None` so
//! the caller applies its own defaults. Mistakes are usage errors (exit code 2).

use super::args::{usage, Args};
use super::progress::ProgressMode;
use crate::encode::Quality;
use crate::render::OutputSize;
use anyhow::Result;
use std::ops::Range;
use std::path::PathBuf;

/// Flags that take a value.
const VALUES: &[&str] = &[
    "--output",
    "--fps",
    "--scale",
    "--size",
    "--quality",
    "--frame",
    "--frames",
    "--count",
    "--threads",
    "--priority",
    "--encoder",
    "--audio-bitrate",
    "--segments",
    "--parts",
    "--progress",
    "--3d-backend",
    "--variables",
    "--variables-file",
];

/// Flags without a value.
const SWITCHES: &[&str] = &["--force"];

#[derive(Debug)]
pub struct Options {
    /// The composition named on the command line; `render` without one uses the project.
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    /// `None`: the project's fps in project mode, else 30.
    pub fps: Option<u32>,
    pub scale: Option<f32>,
    pub size: Option<OutputSize>,
    /// `None`: the project's quality in project mode, else `standard`.
    pub quality: Option<Quality>,
    pub frame: Option<usize>,
    /// `--frames`: several PNG frames from one process (the output names `{frame}`).
    pub frames: Option<Vec<usize>>,
    /// `bench` only: how many frames to draw from `frame` (default: all of them).
    pub count: Option<usize>,
    pub threads: u32,
    pub low_priority: bool,
    /// `--encoder ffmpeg` instead of the native encoder.
    pub ffmpeg: bool,
    /// AAC bitrate in kbit/s.
    pub audio_bitrate: u32,
    /// `--segments A..B`: the segment range of a part file (`.aseg` output).
    pub segments: Option<Range<usize>>,
    /// `--parts`: part files to assemble into the MP4 output.
    pub parts: Option<Vec<PathBuf>>,
    pub progress: ProgressMode,
    /// `--force`: replace an existing MP4 or part file that was named with `-o`/`--output`.
    pub force: bool,
    pub variables: Option<String>,
    pub variables_file: Option<String>,
}

/// Parses `A..B` (segment indexes, end exclusive).
fn range(value: &str) -> Result<Range<usize>> {
    let bad = || usage(format!("--segments must look like A..B (got {value})"));
    let (a, b) = value.split_once("..").ok_or_else(bad)?;
    let range = a.parse().map_err(|_| bad())?..b.parse().map_err(|_| bad())?;
    if range.is_empty() {
        return Err(usage(
            "--segments must not be empty (A must be less than B)",
        ));
    }
    Ok(range)
}

/// Parses a comma-separated list, naming `flag` in errors.
fn list<T: std::str::FromStr>(value: &str, flag: &str) -> Result<Vec<T>> {
    let items = value
        .split(',')
        .map(|item| {
            item.trim()
                .parse()
                .map_err(|_| usage(format!("invalid {flag} item: {item}")))
        })
        .collect::<Result<Vec<T>>>()?;
    if items.len() > 1000 {
        return Err(usage(format!("{flag} takes 1 to 1000 items")));
    }
    Ok(items)
}

/// Parses the arguments of `command` (`render`, `inspect`, or `bench`).
pub fn parse(command: &str, raw: &[String]) -> Result<Options> {
    let args = Args::parse(raw, VALUES, SWITCHES)?;
    let input = match args.positional.as_slice() {
        [] if command == "render" => None,
        [] => {
            return Err(usage(format!(
                "{command} needs a composition: aether-motion {command} <file.html>"
            )))
        }
        [file] => Some(PathBuf::from(file)),
        many => {
            return Err(usage(format!(
                "{command} takes one composition, got {}",
                many.join(" ")
            )))
        }
    };
    if args.value("--size").is_some() && args.value("--scale").is_some() {
        return Err(usage("choose either --size or --scale"));
    }
    let fps = match args.number::<u64>("--fps")? {
        Some(fps) => {
            Some(crate::render::validate_fps(fps, "--fps").map_err(|e| usage(format!("{e:#}")))?)
        }
        None => None,
    };
    let threads = args
        .number("--threads")?
        .unwrap_or_else(crate::encode::default_threads);
    if !(1..=64).contains(&threads) {
        return Err(usage("--threads must be between 1 and 64"));
    }
    let audio_bitrate = args
        .number("--audio-bitrate")?
        .unwrap_or(crate::audio::DEFAULT_BITRATE / 1000);
    if !(64..=320).contains(&audio_bitrate) {
        return Err(usage("--audio-bitrate must be between 64 and 320 (kbit/s)"));
    }
    let low_priority = match args.value("--priority").unwrap_or("low") {
        "low" => true,
        "normal" => false,
        _ => return Err(usage("--priority must be low or normal")),
    };
    let ffmpeg = match args.value("--encoder").unwrap_or("native") {
        "native" => false,
        "ffmpeg" => true,
        _ => return Err(usage("--encoder must be native or ffmpeg")),
    };
    if let Some(backend) = args.choice("--3d-backend")? {
        crate::scene3d::backend::set_request(backend);
    }
    Ok(Options {
        input,
        output: args.value("--output").map(PathBuf::from),
        fps,
        scale: args.number("--scale")?,
        size: args.choice("--size")?,
        quality: args.choice("--quality")?,
        frame: args.number("--frame")?,
        frames: args
            .value("--frames")
            .map(|v| list(v, "--frames"))
            .transpose()?,
        count: args.number("--count")?,
        threads,
        low_priority,
        ffmpeg,
        audio_bitrate,
        segments: args.value("--segments").map(range).transpose()?,
        parts: args
            .value("--parts")
            .map(|v| list(v, "--parts"))
            .transpose()?,
        progress: args.choice("--progress")?.unwrap_or_default(),
        force: args.has("--force"),
        variables: args.value("--variables").map(str::to_string),
        variables_file: args.value("--variables-file").map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::UsageError;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn aliases_parse_to_the_same_options() {
        let short = parse("render", &s(&["a.html", "-o", "b.mp4", "-f"])).unwrap();
        let long = parse("render", &s(&["--output", "b.mp4", "--force", "a.html"])).unwrap();
        assert_eq!(short.output, long.output);
        assert_eq!(short.input, long.input);
        assert!(short.force && long.force);
        let project = parse("render", &s(&["-o", "b.png"])).unwrap();
        assert_eq!(project.input, None);
        assert_eq!(project.fps, None);
    }

    #[test]
    fn values_are_validated_as_usage_errors() {
        let o = parse(
            "render",
            &s(&["a.html", "--fps", "60", "--segments", "0..3"]),
        )
        .unwrap();
        assert_eq!(o.fps, Some(60));
        assert_eq!(o.segments, Some(0..3));
        for bad in [
            &["inspect"][..],
            &["a.html", "b.html"],
            &["a.html", "--fsp", "30"],
            &["a.html", "--fps"],
            &["a.html", "--fps", "0"],
            &["a.html", "--threads", "0"],
            &["a.html", "--audio-bitrate", "32"],
            &["a.html", "--segments", "3..1"],
            &["a.html", "--size", "4k", "--scale", "2"],
            &["a.html", "--quality", "ultra"],
            &["a.html", "--encoder", "x264"],
        ] {
            let command = if bad == ["inspect"] {
                "inspect"
            } else {
                "render"
            };
            let raw = if bad == ["inspect"] { &[][..] } else { bad };
            let error = parse(command, &s(raw)).unwrap_err();
            assert!(
                error.downcast_ref::<UsageError>().is_some(),
                "{bad:?}: {error}"
            );
        }
    }
}
