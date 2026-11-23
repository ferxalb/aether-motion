//! `render`: export a PNG, several PNGs, an MP4, or a part file, in one of two explicit modes.
//!
//! - Project mode, `render` without a file: the composition is the project's entry and `aether.json`
//!   supplies fps, size, quality, the output folder, and variable values.
//! - Direct mode, `render <file>`: only the command line counts; `aether.json` is never read, so the
//!   result does not depend on project files around the composition.
//!
//! Everything else is shared: the flags (`cli::options`, where `-o` and `--output` are one flag),
//! their validation, the collision policy (`output`), and the reports. `ffmpeg` holds the optional
//! FFmpeg encoder. `write_png`, `file_size`, and `backend_note` serve `preview` and `bench` too.

mod cloud;
mod ffmpeg;
mod output;

use super::args::usage;
use super::options::{self, Options};
use super::progress::with_progress;
use super::project;
use super::ui::{self, display_path, Mark, Style};
use crate::encode::{
    assemble_parts, encode_native_with_progress, render_part_with_progress, EncodeOptions,
};
use crate::model::Composition;
use crate::parse::{parse_file_with, VariableValues};
use crate::render::{OutputSize, Renderer};
use anyhow::{Context, Result};
use image::ColorType;
use output::Kind;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Saves one frame as PNG, replacing any file at `output` only once the new one is written (a
/// failed draw or write leaves the previous PNG intact).
pub fn write_png(renderer: &mut Renderer<'_>, output: &Path, frame: usize, fps: u32) -> Result<()> {
    let (w, h) = renderer.dimensions();
    let pixels = renderer.render_frame(frame, fps)?;
    let temp = output::sibling(output, "partial");
    let saved = image::save_buffer(&temp, pixels, w, h, ColorType::Rgba8)
        .map_err(anyhow::Error::from)
        .and_then(|()| fs::rename(&temp, output).map_err(anyhow::Error::from));
    if saved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    saved.with_context(|| format!("could not save {}", output.display()))
}

/// The settings a mode resolves before anything is drawn.
struct Target {
    entry: PathBuf,
    fps: u32,
    /// A size preset from `aether.json`, applied only to compositions of a matching aspect.
    project_size: Option<OutputSize>,
    quality: crate::encode::Quality,
    values: VariableValues,
    output: PathBuf,
    /// Whether the user named the output (`-o`/`--output`), which decides collisions.
    named: bool,
}

/// Resolves the mode: project with no file, direct with one.
fn target(opts: &Options) -> Result<Target> {
    let named = opts.output.is_some();
    let flags = |base: &VariableValues| {
        super::variables::values(
            opts.variables.as_deref(),
            opts.variables_file.as_deref(),
            base,
        )
        .map_err(|e| usage(format!("{e:#}")))
    };
    let default_output = |out_dir: &Path, entry: &Path| {
        let stem = entry
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video");
        out_dir.join(format!("{stem}.mp4"))
    };
    // A named file runs standalone with built-in defaults; without one, aether.json applies.
    let project = project::select(opts.input.as_deref())?;
    Ok(Target {
        output: opts
            .output
            .clone()
            .unwrap_or_else(|| default_output(&project.out_dir, &project.entry)),
        fps: opts.fps.unwrap_or(project.fps),
        project_size: project.size,
        quality: opts.quality.unwrap_or(project.quality),
        values: flags(&project.variables)?,
        entry: project.entry,
        named,
    })
}

/// Rejects flag combinations the output kind cannot honor, before the composition is parsed.
fn check_flags(opts: &Options, kind: Kind, output: &Path) -> Result<()> {
    if opts.frame.is_some() && kind != Kind::Png {
        return Err(usage("--frame only applies to PNG output"));
    }
    if opts.frames.is_some() {
        if kind != Kind::Png {
            return Err(usage("--frames only applies to PNG output"));
        }
        if opts.frame.is_some() {
            return Err(usage("use either --frame or --frames"));
        }
        if !output.to_string_lossy().contains("{frame}") {
            return Err(usage(
                "--frames needs {frame} in the output name, such as 'frame-{frame}.png'",
            ));
        }
    }
    if opts.segments.is_some() != (kind == Kind::Part) {
        return Err(usage("--segments and an .aseg output go together"));
    }
    if opts.parts.is_some() && kind != Kind::Mp4 {
        return Err(usage("--parts assembles into an .mp4 output"));
    }
    if opts.ffmpeg && (kind != Kind::Mp4 || opts.parts.is_some()) {
        return Err(usage("--encoder ffmpeg only renders whole MP4 files"));
    }
    Ok(())
}

/// The renderer at the requested size: an explicit `--size` or `--scale` wins; a project size
/// preset applies only to 16:9 and 9:16 compositions, others render at their own size.
fn renderer<'a>(
    c: &'a Composition,
    opts: &Options,
    project_size: Option<OutputSize>,
) -> Result<Renderer<'a>> {
    match (opts.size, opts.scale, project_size) {
        (Some(size), _, _) => Renderer::with_output_size(c, size),
        (None, Some(scale), _) => Renderer::new(c, scale),
        (None, None, Some(size)) if size.dimensions(c.width, c.height).is_ok() => {
            Renderer::with_output_size(c, size)
        }
        _ => Renderer::new(c, 1.0),
    }
}

/// `render [file] [-o|--output <path>] [options]`; see the module documentation for the modes.
pub fn run(raw: &[String]) -> Result<()> {
    let opts = options::parse("render", raw)?;
    let target = target(&opts)?;
    let output = target.output.as_path();
    let kind = Kind::of(output)?;
    check_flags(&opts, kind, output)?;
    if opts.cloud {
        cloud::check(&opts, kind == Kind::Mp4 && !opts.ffmpeg)?;
    }
    let collision = if opts.frames.is_some() {
        output::Collision::None
    } else {
        output::plan(output, kind, target.named, opts.force)?
    };
    let composition = parse_file_with(&target.entry, &target.values)?;
    let staged = output::Staged::new(output, collision)?;
    let dest = staged.path();
    let style = Style::out();
    let start = Instant::now();
    if let Some(parts) = &opts.parts {
        assemble_parts(&composition, parts, dest, opts.audio_bitrate * 1000)?;
        staged.commit()?;
        let detail = format!("assembled {} parts · {}", parts.len(), file_size(output));
        report(style, output, &detail, start, None);
        return Ok(());
    }
    if opts.cloud {
        cloud::run(&composition, &target.entry, raw, &opts, dest)?;
        staged.commit()?;
        report(style, output, &file_size(output), start, None);
        return Ok(());
    }
    let mut renderer = renderer(&composition, &opts, target.project_size)?;
    if opts.low_priority {
        crate::encode::lower_priority();
    }
    let fps = target.fps;
    let (w, h) = renderer.dimensions();
    let frames = renderer.frame_count(fps);
    let outside = |frame: usize| {
        usage(format!(
            "frame {frame} is outside the composition ({frames} frames, 0 to {})",
            frames.saturating_sub(1)
        ))
    };
    let detail = match kind {
        Kind::Png => match &opts.frames {
            Some(list) => {
                let mut sorted = list.clone();
                sorted.sort_unstable();
                sorted.dedup();
                if let Some(&last) = sorted.last().filter(|&&f| f >= frames) {
                    return Err(outside(last));
                }
                // Ascending order: video sources decode forward.
                let pattern = output.to_string_lossy();
                for &frame in &sorted {
                    let path = PathBuf::from(pattern.replace("{frame}", &frame.to_string()));
                    write_png(&mut renderer, &path, frame, fps)?;
                }
                format!("{} frames · {w}x{h}", sorted.len())
            }
            None => {
                let frame = opts.frame.unwrap_or(0);
                if frame >= frames {
                    return Err(outside(frame));
                }
                write_png(&mut renderer, output, frame, fps)?;
                format!("frame {frame} of {frames} · {w}x{h}")
            }
        },
        Kind::Part | Kind::Mp4 => {
            println!(
                "{}",
                style.bold(&format!(
                    "Rendering {} → {}",
                    display_path(&target.entry),
                    display_path(output)
                ))
            );
            println!(
                "  {}",
                style.dim(&format!(
                    "{w}x{h} · {frames} frames @ {fps} fps · quality {}",
                    format!("{:?}", target.quality).to_lowercase()
                ))
            );
            let options = EncodeOptions {
                audio_bitrate: opts.audio_bitrate * 1000,
                quality: target.quality,
                ..EncodeOptions::new(fps, opts.threads)
            };
            if let Some(segments) = opts.segments.clone() {
                let label = format!("segments {}..{}", segments.start, segments.end);
                with_progress(opts.progress, |observer| {
                    render_part_with_progress(&mut renderer, dest, &options, segments, observer)
                })?;
                label
            } else if opts.ffmpeg {
                if !composition.audio.is_empty() {
                    return Err(usage(
                        "<audio> requires the native encoder; remove --encoder ffmpeg",
                    ));
                }
                ffmpeg::encode(&mut renderer, dest, fps, opts.threads, target.quality)?;
                file_size(dest)
            } else {
                with_progress(opts.progress, |observer| {
                    encode_native_with_progress(&mut renderer, dest, &options, observer)
                })?;
                file_size(dest)
            }
        }
    };
    staged.commit()?;
    report(style, output, &detail, start, Some(&renderer));
    Ok(())
}

/// The result line, the profile and 3D notes, and the frame reuse count.
fn report(
    style: Style,
    output: &Path,
    detail: &str,
    start: Instant,
    renderer: Option<&Renderer<'_>>,
) {
    let elapsed = start.elapsed().as_secs_f64();
    crate::profile::report();
    backend_note(renderer);
    println!(
        "{}",
        ui::line(
            style,
            Mark::Ok,
            &display_path(output),
            &format!("{detail} in {elapsed:.1} s")
        )
    );
    if let Some(renderer) = renderer {
        let stats = renderer.stats();
        println!(
            "    {}",
            style.dim(&format!(
                "{} frames rasterized · {} reused",
                stats.rasterized, stats.reused
            ))
        );
    }
}

/// The note printed when 3D scenes fell back from the GPU to the CPU.
pub fn backend_note(renderer: Option<&Renderer<'_>>) {
    if crate::scene3d::backend::strict() {
        eprintln!("AETHER_3D_PROOF {}", crate::scene3d::backend::proof());
    }
    let status = renderer.map_or_else(crate::scene3d::backend::status, Renderer::scene3d_status);
    if let (_, Some(reason)) = status {
        let style = Style::err();
        eprintln!(
            "{} 3D scenes were drawn on the CPU: {reason}",
            style.yellow("note:")
        );
    }
}

/// A file's size in decimal units (`820 KB`, `4.2 MB`, `1.30 GB`), or `size unknown`.
pub fn file_size(path: &Path) -> String {
    fs::metadata(path).map_or_else(|_| "size unknown".to_string(), |m| bytes(m.len()))
}

fn bytes(n: u64) -> String {
    let n = n as f64;
    if n < 1e6 {
        format!("{:.0} KB", (n / 1e3).max(1.0))
    } else if n < 1e9 {
        format!("{:.1} MB", n / 1e6)
    } else {
        format!("{:.2} GB", n / 1e9)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes_use_readable_units() {
        assert_eq!(super::bytes(200), "1 KB");
        assert_eq!(super::bytes(820_400), "820 KB");
        assert_eq!(super::bytes(4_210_000), "4.2 MB");
        assert_eq!(super::bytes(1_300_000_000), "1.30 GB");
    }
}
