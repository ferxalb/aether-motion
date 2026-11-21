//! Command-line entry point: short project commands live in `cli`; this file keeps the legacy flag-based `inspect`, `render --output`, `bench`, `block`, and `mcp`.

use aether_motion::cli::progress::{with_progress, ProgressMode};
use aether_motion::encode::{
    assemble_parts, encode_native_with_progress, render_part_with_progress, segment_count,
    EncodeOptions, Quality, PART_EXTENSION,
};
use aether_motion::parse::parse_file_with;
use aether_motion::render::{OutputSize, Renderer};
use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::fs;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

fn usage() -> &'static str {
    concat!("AETHER Motion ", env!("CARGO_PKG_VERSION"), "\n\
Quick start:\n\
  aether-motion init [dir]                 create a project with a sample video, agent skills, and MCP config\n\
  aether-motion preview [file] [--at 2.5] [--watch]   render one frame to .aether/preview.png and open it\n\
  aether-motion render [file] [-o out.mp4] [--size 1080p|4k] [--fps 30] [--quality standard]   (defaults from aether.json; writes out/<name>.mp4)\n\
  --variables '{\"title\":\"Q4\"}' or --variables-file values.json   (render, preview, check, bench, inspect: values for data-composition-variables)\n\
  aether-motion check [file]               validate (summary on a terminal, JSON when piped or with --json)\n\
  aether-motion add [block] [--set k=v]    list blocks, or append one to the composition\n\
  aether-motion captions <file> [--to srt]  convert a transcript (JSON, SRT, VTT) to JSON, SRT, or VTT\n\
  aether-motion agent setup|status         install skills and MCP config for coding agents\n\
  aether-motion doctor                     diagnose the installation and project\n\
  aether-motion --version                  print the installed version\n\
`file` defaults to aether.json \"entry\", then index.html.\n\
Advanced:\n\
  aether-motion inspect <composition.html>\n\
  aether-motion render <composition.html> --output <video.mp4|frame.png> [--fps 30] [--scale 1 | --size 1080p|4k] [--threads N (default: cores/4, 1 to 4)] [--priority low|normal (default: low)] [--encoder native|ffmpeg] [--quality draft|standard|high|max (default: standard)] [--audio-bitrate 192] [--frame N | --frames N,N,... with {frame} in the PNG name] [--progress auto|json|off (stderr; auto: a status line on a terminal)] [--3d-backend cpu|gpu|auto (data-hf-3d scenes; default cpu)]\n\
  aether-motion render <composition.html> --output <part.aseg> --segments A..B [render options]   (distributed render: encode one-second segments A..B; `inspect --fps N` prints the segment count)\n\
  aether-motion render <composition.html> --output <video.mp4> --parts a.aseg,b.aseg,... [--variables ...] [--audio-bitrate 192]   (assemble parts into the MP4 a single render would write)\n\
  aether-motion bench <composition.html> [--fps 30] [--scale 1 | --size 1080p|4k] [--frame START --count N]\n\
  aether-motion check <composition.html>   (JSON; exits with 1 on failure)\n\
  aether-motion block [list | <name> [--set key=value]...]   (print a ready-made block or theme as HTML)\n\
  aether-motion mcp                        (MCP server over stdio for agents)\n\
The native backend accepts an explicit subset of HyperFrames HTML/CSS.\n\
MP4 uses built-in OpenH264 and AAC-LC by default (--quality sets the H.264 quantizer: draft 30, standard 26, high 22, max 18; --audio-bitrate in kbit/s, 64 to 320). --encoder ffmpeg requires FFmpeg on PATH and does not support <audio>.")
}

#[derive(Debug)]
struct Options {
    command: String,
    input: PathBuf,
    output: Option<PathBuf>,
    fps: u32,
    scale: Option<f32>,
    size: Option<OutputSize>,
    threads: u32,
    encoder: String,
    /// AAC bitrate in kbit/s for compositions with `<audio>`.
    audio_bitrate: u32,
    /// H.264 quality preset for MP4 output.
    quality: Quality,
    low_priority: bool,
    frame: Option<usize>,
    /// `bench` only: how many frames to draw from `frame` (default: all of them).
    count: Option<usize>,
    /// `--variables`: a JSON object of values for the composition's declared variables.
    variables: Option<String>,
    /// `--variables-file`: a JSON file holding that object.
    variables_file: Option<String>,
    /// `--frames`: several PNG frames from one process (the output names `{frame}`).
    frames: Option<Vec<usize>>,
    /// `--segments A..B`: the segment range of a part file (`.aseg` output).
    segments: Option<Range<usize>>,
    /// `--parts`: part files to assemble into the MP4 output.
    parts: Option<Vec<PathBuf>>,
    /// Whether `--fps` was given (`inspect` prints the segment plan only then).
    fps_given: bool,
    /// `--progress`: how native MP4 and part renders report progress on stderr.
    progress: ProgressMode,
}

/// Parses `A..B` (segment indexes, end exclusive).
fn parse_range(value: &str) -> Result<Range<usize>> {
    let (a, b) = value
        .split_once("..")
        .ok_or_else(|| anyhow!("--segments must look like A..B"))?;
    let range = a.parse().context("invalid --segments start")?
        ..b.parse().context("invalid --segments end")?;
    if range.is_empty() {
        bail!("--segments must not be empty");
    }
    Ok(range)
}

/// Parses a comma-separated list, naming `flag` in errors.
fn parse_list<T: std::str::FromStr>(value: &str, flag: &str) -> Result<Vec<T>> {
    let items = value
        .split(',')
        .map(|item| {
            item.trim()
                .parse()
                .map_err(|_| anyhow!("invalid {flag} item: {item}"))
        })
        .collect::<Result<Vec<T>>>()?;
    if items.is_empty() || items.len() > 1000 {
        bail!("{flag} takes 1 to 1000 items");
    }
    Ok(items)
}

fn options() -> Result<Option<Options>> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        println!("{}", usage());
        return Ok(None);
    };
    if command == "--help" || command == "-h" || command == "help" {
        println!("{}", usage());
        return Ok(None);
    }
    if command == "--version" {
        println!("AETHER Motion {}", env!("CARGO_PKG_VERSION"));
        return Ok(None);
    }
    if !matches!(command.as_str(), "inspect" | "render" | "bench") {
        bail!("unknown command: {command}\n{}", usage());
    }
    let input = args
        .next()
        .ok_or_else(|| anyhow!("missing <composition.html>"))?
        .into();
    let mut opts = Options {
        command,
        input,
        output: None,
        fps: 30,
        scale: None,
        size: None,
        threads: aether_motion::encode::default_threads(),
        encoder: "native".to_string(),
        audio_bitrate: aether_motion::audio::DEFAULT_BITRATE / 1000,
        quality: Quality::default(),
        low_priority: true,
        frame: None,
        count: None,
        variables: None,
        variables_file: None,
        frames: None,
        segments: None,
        parts: None,
        fps_given: false,
        progress: ProgressMode::default(),
    };
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| anyhow!("missing value for {arg}"))?;
        match arg.as_str() {
            "--output" => opts.output = Some(value.into()),
            "--fps" => {
                opts.fps = value.parse().context("invalid --fps")?;
                opts.fps_given = true;
            }
            "--scale" => opts.scale = Some(value.parse().context("invalid --scale")?),
            "--size" => opts.size = Some(value.parse()?),
            "--threads" => opts.threads = value.parse().context("invalid --threads")?,
            "--encoder" => opts.encoder = value,
            "--audio-bitrate" => {
                opts.audio_bitrate = value.parse().context("invalid --audio-bitrate")?
            }
            "--quality" => opts.quality = value.parse()?,
            "--priority" => {
                opts.low_priority = match value.as_str() {
                    "low" => true,
                    "normal" => false,
                    _ => bail!("--priority must be low or normal"),
                }
            }
            "--frame" => opts.frame = Some(value.parse().context("invalid --frame")?),
            "--count" => opts.count = Some(value.parse().context("invalid --count")?),
            "--variables" => opts.variables = Some(value),
            "--variables-file" => opts.variables_file = Some(value),
            "--frames" => opts.frames = Some(parse_list(&value, "--frames")?),
            "--segments" => opts.segments = Some(parse_range(&value)?),
            "--parts" => opts.parts = Some(parse_list(&value, "--parts")?),
            "--progress" => opts.progress = value.parse()?,
            "--3d-backend" => aether_motion::scene3d::backend::set_request(value.parse()?),
            _ => bail!("unknown option: {arg}"),
        }
    }
    if opts.size.is_some() && opts.scale.is_some() {
        bail!("choose either --size or --scale");
    }
    aether_motion::render::validate_fps(u64::from(opts.fps), "--fps")?;
    if opts.threads == 0 || opts.threads > 64 {
        bail!("--threads must be between 1 and 64");
    }
    if !(64..=320).contains(&opts.audio_bitrate) {
        bail!("--audio-bitrate must be between 64 and 320 (kbit/s)");
    }
    if opts.encoder != "native" && opts.encoder != "ffmpeg" {
        bail!("--encoder must be native or ffmpeg");
    }
    Ok(Some(opts))
}

/// libx264 CRF for each preset; `standard` keeps the historical CRF 25.
fn ffmpeg_crf(quality: Quality) -> &'static str {
    match quality {
        Quality::Draft => "28",
        Quality::Standard => "25",
        Quality::High => "21",
        Quality::Max => "18",
    }
}

fn mp4_ffmpeg(
    renderer: &mut Renderer<'_>,
    output: &Path,
    fps: u32,
    threads: u32,
    quality: Quality,
) -> Result<()> {
    let (w, h) = renderer.dimensions();
    if w % 2 != 0 || h % 2 != 0 {
        bail!("MP4 yuv420p requires even dimensions; change --scale");
    }
    if output.exists() {
        bail!("output already exists: {}", output.display());
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        bail!("output directory does not exist: {}", parent.display());
    }
    let partial = output.with_extension(format!("aether-{}.partial.mp4", std::process::id()));
    let size = format!("{w}x{h}");
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &size,
            "-r",
            &fps.to_string(),
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            ffmpeg_crf(quality),
            "-pix_fmt",
            "yuv420p",
            "-threads",
            &threads.to_string(),
            "-movflags",
            "+faststart",
            "-y",
        ])
        .arg(&partial)
        .stdin(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdout(Stdio::null())
        .spawn()
        .context("could not start FFmpeg; install it and add it to PATH")?;
    let result = (|| -> Result<()> {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("FFmpeg pipe is unavailable"))?;
        for frame in 0..renderer.frame_count(fps) {
            stdin
                .write_all(renderer.render_frame(frame, fps)?)
                .with_context(|| format!("FFmpeg closed the pipe at frame {frame}"))?;
        }
        drop(stdin);
        let status = child.wait().context("could not wait for FFmpeg")?;
        if !status.success() {
            bail!("FFmpeg exited with {status}");
        }
        fs::rename(&partial, output).context("could not save MP4")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_file(&partial);
    }
    result
}

/// Writes several frames as PNG from one renderer; `{frame}` in `output` becomes the frame number.
/// Frames are drawn in ascending order (video sources decode forward) and written as named.
fn write_pngs(
    renderer: &mut Renderer<'_>,
    output: &Path,
    frames: &[usize],
    single: Option<usize>,
    fps: u32,
) -> Result<()> {
    let pattern = output.to_string_lossy();
    if single.is_some() || !pattern.contains("{frame}") {
        bail!("--frames needs {{frame}} in the PNG --output name and no --frame");
    }
    let total = renderer.frame_count(fps);
    let mut sorted = frames.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if let Some(&last) = sorted.last().filter(|&&f| f >= total) {
        bail!("--frames {last} is outside the composition ({total} frames)");
    }
    for frame in sorted {
        let path = PathBuf::from(pattern.replace("{frame}", &frame.to_string()));
        aether_motion::cli::render_cmd::write_png(renderer, &path, frame, fps)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let all: Vec<String> = env::args().skip(1).collect();
    if let Some(result) = aether_motion::cli::run(&all) {
        let code = result?;
        if code != 0 {
            std::process::exit(code);
        }
        return Ok(());
    }
    let mut raw = env::args().skip(1);
    match raw.next().as_deref() {
        Some("mcp") => return aether_motion::agent::serve(),
        Some("block") => {
            let args: Vec<String> = raw.collect();
            print!("{}", aether_motion::blocks::run_cli(&args)?);
            return Ok(());
        }
        Some("check") => {
            let file = raw
                .next()
                .ok_or_else(|| anyhow!("missing <composition.html>"))?;
            let report = aether_motion::agent::check(Path::new(&file));
            println!("{}", serde_json::to_string_pretty(&report)?);
            std::process::exit(if report["ok"] == true { 0 } else { 1 });
        }
        _ => {}
    }
    let Some(opts) = options()? else {
        return Ok(());
    };
    let values = aether_motion::cli::variables::values(
        opts.variables.as_deref(),
        opts.variables_file.as_deref(),
        &Default::default(),
    )?;
    let composition = parse_file_with(&opts.input, &values)?;
    if opts.command == "inspect" {
        println!(
            "{}x{} | {:.3} s | {} elements",
            composition.width,
            composition.height,
            composition.duration,
            composition.elements.len()
        );
        if opts.fps_given {
            let frames = (composition.duration * opts.fps as f64).ceil() as usize;
            println!(
                "  {frames} frames @ {} fps | segments {}",
                opts.fps,
                segment_count(frames, opts.fps)
                    .map_or_else(|| "none (one piece)".to_string(), |n| n.to_string())
            );
        }
        for clip in &composition.audio {
            let range = match clip.duration {
                Some(duration) => format!("{:.3}..{:.3} s", clip.start, clip.start + duration),
                None => format!("{:.3} s..source end", clip.start),
            };
            println!(
                "  audio {} {range}, volume {}{}",
                clip.id
                    .clone()
                    .unwrap_or_else(|| clip.src.display().to_string()),
                clip.volume,
                if clip.looped { ", loop" } else { "" }
            );
        }
        for element in &composition.elements {
            if element.parent.is_some() && (element.end - element.start) < composition.duration {
                println!(
                    "  {} <{}> {:.3}..{:.3} s, track {}",
                    element.id.as_deref().unwrap_or("no id"),
                    element.tag,
                    element.start,
                    element.end,
                    element.track
                );
            }
        }
        return Ok(());
    }
    if let Some(parts) = &opts.parts {
        let output = opts
            .output
            .as_deref()
            .filter(|o| o.extension().is_some_and(|x| x == "mp4"))
            .ok_or_else(|| anyhow!("--parts requires an .mp4 --output"))?;
        let start = Instant::now();
        assemble_parts(&composition, parts, output, opts.audio_bitrate * 1000)?;
        println!(
            "assembled {} parts | {:.3} s",
            parts.len(),
            start.elapsed().as_secs_f64()
        );
        return Ok(());
    }
    let mut renderer = match opts.size {
        Some(size) => Renderer::with_output_size(&composition, size)?,
        None => Renderer::new(&composition, opts.scale.unwrap_or(1.0))?,
    };
    if opts.low_priority {
        aether_motion::encode::lower_priority();
    }
    let (w, h) = renderer.dimensions();
    let frames = renderer.frame_count(opts.fps);
    let start = Instant::now();
    if opts.command == "bench" {
        let first = opts.frame.unwrap_or(0);
        let last = opts.count.map_or(frames, |n| (first + n).min(frames));
        if first >= frames {
            bail!("--frame is outside the composition ({frames} frames)");
        }
        for i in first..last {
            renderer.render_frame(i, opts.fps)?;
        }
    } else {
        let output = opts
            .output
            .as_deref()
            .ok_or_else(|| anyhow!("render requires --output"))?;
        let extension = output.extension().and_then(|x| x.to_str());
        if opts.segments.is_some() != (extension == Some(PART_EXTENSION)) {
            bail!("--segments and an .{PART_EXTENSION} --output go together");
        }
        if opts.frames.is_some() && extension != Some("png") {
            bail!("--frames only applies to PNG");
        }
        match extension {
            Some("png") => match &opts.frames {
                Some(list) => write_pngs(&mut renderer, output, list, opts.frame, opts.fps)?,
                None => aether_motion::cli::render_cmd::write_png(
                    &mut renderer,
                    output,
                    opts.frame.unwrap_or(0),
                    opts.fps,
                )?,
            },
            Some(PART_EXTENSION) => {
                if opts.frame.is_some() || opts.encoder != "native" {
                    bail!("part files use the native encoder and take no --frame");
                }
                let options = EncodeOptions {
                    quality: opts.quality,
                    ..EncodeOptions::new(opts.fps, opts.threads)
                };
                let segments = opts.segments.clone().unwrap_or_default();
                with_progress(opts.progress, |observer| {
                    render_part_with_progress(&mut renderer, output, &options, segments, observer)
                })?;
            }
            Some("mp4") => {
                if opts.frame.is_some() {
                    bail!("--frame only applies to PNG");
                }
                if opts.encoder == "native" {
                    let options = EncodeOptions {
                        audio_bitrate: opts.audio_bitrate * 1000,
                        quality: opts.quality,
                        ..EncodeOptions::new(opts.fps, opts.threads)
                    };
                    with_progress(opts.progress, |observer| {
                        encode_native_with_progress(&mut renderer, output, &options, observer)
                    })?;
                } else {
                    if !composition.audio.is_empty() {
                        bail!("<audio> requires the native encoder; remove --encoder ffmpeg");
                    }
                    mp4_ffmpeg(&mut renderer, output, opts.fps, opts.threads, opts.quality)?;
                }
            }
            _ => bail!("output must end in .png, .mp4, or .{PART_EXTENSION}"),
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    aether_motion::profile::report();
    if let (_, Some(reason)) = aether_motion::scene3d::backend::status() {
        eprintln!("note: 3D scenes were drawn on the CPU: {reason}");
    }
    let stats = renderer.stats();
    println!(
        "{}x{} | {frames} frames | {:.3} s | rasterized {} | reused {}",
        w, h, elapsed, stats.rasterized, stats.reused
    );
    Ok(())
}
