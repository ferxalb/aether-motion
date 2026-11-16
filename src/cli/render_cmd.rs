//! `render`: project-aware export of an MP4 or PNG with sensible defaults and a one-line summary.

use super::args::Args;
use super::project;
use crate::encode::{encode_native_with_audio, lower_priority};
use crate::parse::parse_file;
use crate::render::Renderer;
use anyhow::{anyhow, bail, Context, Result};
use image::ColorType;
use std::fs;
use std::path::Path;
use std::time::Instant;

/// Saves one frame as PNG.
pub fn write_png(renderer: &mut Renderer<'_>, output: &Path, frame: usize, fps: u32) -> Result<()> {
    let (w, h) = renderer.dimensions();
    let pixels = renderer.render_frame(frame, fps)?;
    image::save_buffer(output, pixels, w, h, ColorType::Rgba8)
        .with_context(|| format!("could not save {}", output.display()))
}

/// `render [file] [-o out.mp4|png] [--size 1080p|4k | --scale N] [--fps N] [--frame N] [--force]`.
pub fn run(raw: &[String]) -> Result<()> {
    let args = Args::parse(
        raw,
        &["--output", "--size", "--scale", "--fps", "--threads", "--frame", "--audio-bitrate"],
        &["--force"],
    )?;
    let project = project::resolve(Path::new("."), args.positional.first().map(String::as_str))?;
    let composition = parse_file(&project.entry)?;
    let fps = args.number("--fps")?.unwrap_or(project.fps);
    if args.value("--size").is_some() && args.value("--scale").is_some() {
        bail!("choose either --size or --scale");
    }
    // An explicit --scale overrides the project's size preset.
    let size = match args.value("--size") {
        Some(s) => Some(s.parse()?),
        None if args.value("--scale").is_some() => None,
        None => project.size,
    };
    let mut renderer = match (size, args.number::<f32>("--scale")?) {
        (_, Some(scale)) => Renderer::new(&composition, scale)?,
        // A project size preset only applies to 16:9 / 9:16 compositions; others render natively.
        (Some(size), None) if args.value("--size").is_some() || size.dimensions(composition.width, composition.height).is_ok() => {
            Renderer::with_output_size(&composition, size)?
        }
        _ => Renderer::new(&composition, 1.0)?,
    };
    lower_priority();
    let explicit = args.value("--output");
    let stem = project.entry.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let output = match explicit {
        Some(o) => o.into(),
        None => project.out_dir.join(format!("{stem}.mp4")),
    };
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    if output.exists() {
        if explicit.is_some() && !args.has("--force") {
            bail!("{} already exists; pass --force to replace it", output.display());
        }
        fs::remove_file(&output)?;
    }
    let (w, h) = renderer.dimensions();
    let frames = renderer.frame_count(fps);
    let start = Instant::now();
    match output.extension().and_then(|x| x.to_str()) {
        Some("png") => {
            let frame = args.number("--frame")?.unwrap_or(0);
            if frame >= frames {
                bail!("--frame is outside the composition ({frames} frames)");
            }
            write_png(&mut renderer, &output, frame, fps)?;
        }
        Some("mp4") => {
            let threads = args.number("--threads")?.unwrap_or_else(crate::encode::default_threads);
            let kbps: u32 = args.number("--audio-bitrate")?.unwrap_or(crate::audio::DEFAULT_BITRATE / 1000);
            println!("Rendering {} -> {} ({w}x{h}, {frames} frames @ {fps} fps)", project.entry.display(), output.display());
            encode_native_with_audio(&mut renderer, &output, fps, threads, kbps * 1000)?;
        }
        _ => return Err(anyhow!("output must end in .png or .mp4")),
    }
    let bytes = fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    println!("Done: {} ({:.1} MB) in {:.1} s", output.display(), bytes as f64 / 1e6, start.elapsed().as_secs_f64());
    Ok(())
}
