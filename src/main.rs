//! Command-line entry point: short project commands live in `cli`; this file keeps the legacy flag-based `inspect`, `render --output`, `bench`, `block`, and `mcp`.

use aether_motion::encode::encode_native_with_audio;
use aether_motion::parse::parse_file;
use aether_motion::render::{OutputSize, Renderer};
use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

fn usage() -> &'static str {
    concat!("AETHER Motion ", env!("CARGO_PKG_VERSION"), "\n\
Quick start:\n\
  aether-motion init [dir]                 create a project with a sample video, agent skills, and MCP config\n\
  aether-motion preview [file] [--at 2.5] [--watch]   render one frame to .aether/preview.png and open it\n\
  aether-motion render [file] [-o out.mp4] [--size 1080p|4k] [--fps 30]   (defaults from aether.json; writes out/<name>.mp4)\n\
  aether-motion check [file]               validate (summary on a terminal, JSON when piped or with --json)\n\
  aether-motion add [block] [--set k=v]    list blocks, or append one to the composition\n\
  aether-motion agent setup|status         install skills and MCP config for coding agents\n\
  aether-motion doctor                     diagnose the installation and project\n\
`file` defaults to aether.json \"entry\", then index.html.\n\
Advanced:\n\
  aether-motion inspect <composition.html>\n\
  aether-motion render <composition.html> --output <video.mp4|frame.png> [--fps 30] [--scale 1 | --size 1080p|4k] [--threads N (default: cores/4, 1 to 4)] [--priority low|normal (default: low)] [--encoder native|ffmpeg] [--audio-bitrate 192] [--frame N]\n\
  aether-motion bench <composition.html> [--fps 30] [--scale 1 | --size 1080p|4k] [--frame START --count N]\n\
  aether-motion check <composition.html>   (JSON; exits with 1 on failure)\n\
  aether-motion block [list | <name> [--set key=value]...]   (print a ready-made block or theme as HTML)\n\
  aether-motion mcp                        (MCP server over stdio for agents)\n\
The native backend accepts an explicit subset of HyperFrames HTML/CSS.\n\
MP4 uses built-in OpenH264 and AAC-LC by default (--audio-bitrate in kbit/s, 64 to 320). --encoder ffmpeg requires FFmpeg on PATH and does not support <audio>.")
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
    low_priority: bool,
    frame: Option<usize>,
    /// `bench` only: how many frames to draw from `frame` (default: all of them).
    count: Option<usize>,
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
        low_priority: true,
        frame: None,
        count: None,
    };
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| anyhow!("missing value for {arg}"))?;
        match arg.as_str() {
            "--output" => opts.output = Some(value.into()),
            "--fps" => opts.fps = value.parse().context("invalid --fps")?,
            "--scale" => opts.scale = Some(value.parse().context("invalid --scale")?),
            "--size" => opts.size = Some(value.parse()?),
            "--threads" => opts.threads = value.parse().context("invalid --threads")?,
            "--encoder" => opts.encoder = value,
            "--audio-bitrate" => {
                opts.audio_bitrate = value.parse().context("invalid --audio-bitrate")?
            }
            "--priority" => {
                opts.low_priority = match value.as_str() {
                    "low" => true,
                    "normal" => false,
                    _ => bail!("--priority must be low or normal"),
                }
            }
            "--frame" => opts.frame = Some(value.parse().context("invalid --frame")?),
            "--count" => opts.count = Some(value.parse().context("invalid --count")?),
            _ => bail!("unknown option: {arg}"),
        }
    }
    if opts.size.is_some() && opts.scale.is_some() {
        bail!("choose either --size or --scale");
    }
    if opts.fps == 0 || opts.fps > 240 {
        bail!("--fps must be between 1 and 240");
    }
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

fn mp4_ffmpeg(renderer: &mut Renderer<'_>, output: &Path, fps: u32, threads: u32) -> Result<()> {
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
            "25",
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
    let composition = parse_file(&opts.input)?;
    if opts.command == "inspect" {
        println!(
            "{}x{} | {:.3} s | {} elements",
            composition.width,
            composition.height,
            composition.duration,
            composition.elements.len()
        );
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
        match output.extension().and_then(|x| x.to_str()) {
            Some("png") => aether_motion::cli::render_cmd::write_png(&mut renderer, output, opts.frame.unwrap_or(0), opts.fps)?,
            Some("mp4") => {
                if opts.frame.is_some() {
                    bail!("--frame only applies to PNG");
                }
                if opts.encoder == "native" {
                    encode_native_with_audio(
                        &mut renderer,
                        output,
                        opts.fps,
                        opts.threads,
                        opts.audio_bitrate * 1000,
                    )?;
                } else {
                    if !composition.audio.is_empty() {
                        bail!("<audio> requires the native encoder; remove --encoder ffmpeg");
                    }
                    mp4_ffmpeg(&mut renderer, output, opts.fps, opts.threads)?;
                }
            }
            _ => bail!("output must end in .png or .mp4"),
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let stats = renderer.stats();
    println!(
        "{}x{} | {frames} frames | {:.3} s | rasterized {} | reused {}",
        w, h, elapsed, stats.rasterized, stats.reused
    );
    Ok(())
}
