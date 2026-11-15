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
Uso:\n\
  aether-hyperframes inspect <composicion.html>\n\
  aether-hyperframes render <composicion.html> --output <video.mp4|frame.png> [--fps 30] [--scale 1 | --size 1080p|4k] [--threads N (por defecto: núcleos, máx. 4)] [--encoder native|ffmpeg] [--frame N]\n\
  aether-hyperframes bench <composicion.html> [--fps 30] [--scale 1 | --size 1080p|4k]\n\
  aether-hyperframes check <composicion.html>   (JSON; sale con 1 si falla)\n\
  aether-hyperframes mcp                        (servidor MCP por stdio, para agentes)\n\
El backend nativo acepta un subconjunto explícito de HTML/CSS de HyperFrames.\n\
MP4 usa OpenH264 integrado por defecto; --encoder ffmpeg requiere FFmpeg en PATH."
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
        bail!("comando desconocido: {command}\n{}", usage());
    }
    let input = args
        .next()
        .ok_or_else(|| anyhow!("falta <composicion.html>"))?
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
        frame: None,
    };
    while let Some(arg) = args.next() {
        let value = args.next().ok_or_else(|| anyhow!("falta valor de {arg}"))?;
        match arg.as_str() {
            "--output" => opts.output = Some(value.into()),
            "--fps" => opts.fps = value.parse().context("--fps inválido")?,
            "--scale" => opts.scale = Some(value.parse().context("--scale inválido")?),
            "--size" => opts.size = Some(value.parse()?),
            "--threads" => opts.threads = value.parse().context("--threads inválido")?,
            "--encoder" => opts.encoder = value,
            "--frame" => opts.frame = Some(value.parse().context("--frame inválido")?),
            _ => bail!("opción desconocida: {arg}"),
        }
    }
    if opts.size.is_some() && opts.scale.is_some() {
        bail!("elige --size o --scale");
    }
    if opts.fps == 0 || opts.fps > 240 {
        bail!("--fps debe estar entre 1 y 240");
    }
    if opts.threads == 0 || opts.threads > 64 {
        bail!("--threads debe estar entre 1 y 64");
    }
    if opts.encoder != "native" && opts.encoder != "ffmpeg" {
        bail!("--encoder debe ser native o ffmpeg");
    }
    Ok(Some(opts))
}

fn png(renderer: &mut Renderer<'_>, output: &Path, frame: usize, fps: u32) -> Result<()> {
    let (w, h) = renderer.dimensions();
    let pixels = renderer.render_frame(frame, fps)?;
    image::save_buffer(output, pixels, w, h, ColorType::Rgba8)
        .with_context(|| format!("no se pudo guardar {}", output.display()))?;
    Ok(())
}

fn mp4_ffmpeg(renderer: &mut Renderer<'_>, output: &Path, fps: u32, threads: u32) -> Result<()> {
    let (w, h) = renderer.dimensions();
    if w % 2 != 0 || h % 2 != 0 {
        bail!("MP4 yuv420p necesita dimensiones pares; cambia --scale");
    }
    if output.exists() {
        bail!("salida ya existe: {}", output.display());
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        bail!("directorio de salida no existe: {}", parent.display());
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
        .context("no se pudo iniciar FFmpeg; instala FFmpeg y añádelo a PATH")?;
    let result = (|| -> Result<()> {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("pipe de FFmpeg no disponible"))?;
        for frame in 0..renderer.frame_count(fps) {
            stdin
                .write_all(renderer.render_frame(frame, fps)?)
                .with_context(|| format!("FFmpeg cerró el pipe en frame {frame}"))?;
        }
        drop(stdin);
        let status = child.wait().context("no se pudo esperar a FFmpeg")?;
        if !status.success() {
            bail!("FFmpeg terminó con {status}");
        }
        fs::rename(&partial, output).context("no se pudo guardar MP4")?;
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
        Some("check") => {
            let file = raw
                .next()
                .ok_or_else(|| anyhow!("falta <composicion.html>"))?;
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
            "{}x{} | {:.3} s | {} elementos",
            composition.width,
            composition.height,
            composition.duration,
            composition.elements.len()
        );
        for element in &composition.elements {
            if element.parent.is_some() && (element.end - element.start) < composition.duration {
                println!(
                    "  {} <{}> {:.3}..{:.3} s, pista {}",
                    element.id.as_deref().unwrap_or("sin id"),
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
            .ok_or_else(|| anyhow!("render requiere --output"))?;
        match output.extension().and_then(|x| x.to_str()) {
            Some("png") => png(&mut renderer, output, opts.frame.unwrap_or(0), opts.fps)?,
            Some("mp4") => {
                if opts.frame.is_some() {
                    bail!("--frame solo aplica a PNG");
                }
                if opts.encoder == "native" {
                    encode_native(&mut renderer, output, opts.fps, opts.threads)?;
                } else {
                    mp4_ffmpeg(&mut renderer, output, opts.fps, opts.threads)?;
                }
            }
            _ => bail!("salida debe terminar en .png o .mp4"),
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let stats = renderer.stats();
    println!(
        "{}x{} | {frames} frames | {:.3} s | rasterizados {} | reutilizados {}",
        w, h, elapsed, stats.rasterized, stats.reused
    );
    Ok(())
}
