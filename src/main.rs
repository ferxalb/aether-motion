//! Command-line entry point: `inspect`, `render`, `bench`, `check`, `block`, and `mcp`.

use aether_hyperframes::encode::encode_native;
use aether_hyperframes::parse::parse_file;
use aether_hyperframes::render::{OutputSize, Renderer};
use anyhow::{anyhow, bail, Context, Result};
use image::ColorType;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

fn usage() -> &'static str {
    "aether-hyperframes 0.1\n\
Usage:\n\
  aether-hyperframes inspect <composition.html>\n\
  aether-hyperframes render <composition.html> --output <video.mp4|frame.png> [--fps 30] [--scale 1 | --size 1080p|4k] [--threads N (default: cores/4, 1 to 4)] [--priority low|normal (default: low)] [--encoder native|ffmpeg] [--frame N]\n\
  aether-hyperframes bench <composition.html> [--fps 30] [--scale 1 | --size 1080p|4k]\n\
  aether-hyperframes check <composition.html>   (JSON; exits with 1 on failure)\n\
  aether-hyperframes block [list | <name> [--set key=value]...]   (print a ready-made block or theme as HTML)\n\
  aether-hyperframes mcp                        (MCP server over stdio for agents)\n\
The native backend accepts an explicit subset of HyperFrames HTML/CSS.\n\
MP4 uses built-in OpenH264 by default; --encoder ffmpeg requires FFmpeg on PATH."
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
    low_priority: bool,
    frame: Option<usize>,
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
        threads: aether_hyperframes::encode::default_threads(),
        encoder: "native".to_string(),
        low_priority: true,
        frame: None,
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
            "--priority" => {
                opts.low_priority = match value.as_str() {
                    "low" => true,
                    "normal" => false,
                    _ => bail!("--priority must be low or normal"),
                }
            }
            "--frame" => opts.frame = Some(value.parse().context("invalid --frame")?),
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
    if opts.encoder != "native" && opts.encoder != "ffmpeg" {
        bail!("--encoder must be native or ffmpeg");
    }
    Ok(Some(opts))
}

fn png(renderer: &mut Renderer<'_>, output: &Path, frame: usize, fps: u32) -> Result<()> {
    let (w, h) = renderer.dimensions();
    let pixels = renderer.render_frame(frame, fps)?;
    image::save_buffer(output, pixels, w, h, ColorType::Rgba8)
        .with_context(|| format!("could not save {}", output.display()))?;
    Ok(())
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
    let mut raw = env::args().skip(1);
    match raw.next().as_deref() {
        Some("mcp") => return aether_hyperframes::agent::serve(),
        Some("block") => {
            let args: Vec<String> = raw.collect();
            print!("{}", aether_hyperframes::blocks::run_cli(&args)?);
            return Ok(());
        }
        Some("check") => {
            let file = raw
                .next()
                .ok_or_else(|| anyhow!("missing <composition.html>"))?;
            let report = aether_hyperframes::agent::check(Path::new(&file));
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
        aether_hyperframes::encode::lower_priority();
    }
    let (w, h) = renderer.dimensions();
    let frames = renderer.frame_count(opts.fps);
    let start = Instant::now();
    if opts.command == "bench" {
        for i in 0..frames {
            renderer.render_frame(i, opts.fps)?;
        }
    } else {
        let output = opts
            .output
            .as_deref()
            .ok_or_else(|| anyhow!("render requires --output"))?;
        match output.extension().and_then(|x| x.to_str()) {
            Some("png") => png(&mut renderer, output, opts.frame.unwrap_or(0), opts.fps)?,
            Some("mp4") => {
                if opts.frame.is_some() {
                    bail!("--frame only applies to PNG");
                }
                if opts.encoder == "native" {
                    encode_native(&mut renderer, output, opts.fps, opts.threads)?;
                } else {
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
