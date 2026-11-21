//! `--encoder ffmpeg`: pipe raw RGBA frames into an FFmpeg libx264 process (no audio).

use crate::encode::Quality;
use crate::render::Renderer;
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// libx264 CRF for each preset; `standard` keeps the historical CRF 25.
fn crf(quality: Quality) -> &'static str {
    match quality {
        Quality::Draft => "28",
        Quality::Standard => "25",
        Quality::High => "21",
        Quality::Max => "18",
    }
}

/// Encodes every frame of `renderer` into `output` through FFmpeg, writing a partial file first and
/// renaming it into place only when FFmpeg succeeds.
pub fn encode(
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
            crf(quality),
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
