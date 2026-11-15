use crate::render::Renderer;
use anyhow::{anyhow, bail, Context, Result};
use mp4::{AvcConfig, Bytes, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
use openh264::encoder::{
    Complexity, Encoder, EncoderConfig, FrameRate, FrameType, QpRange, RateControlMode, VuiConfig,
};
use openh264::formats::{RgbaSliceU8, YUVBuffer};
use openh264::{OpenH264API, Timestamp};
use std::fs::{self, File};
use std::path::Path;
use std::sync::{mpsc, Arc};

#[derive(Default)]
struct NalFrame {
    sample: Vec<u8>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

impl NalFrame {
    fn push(&mut self, unit: &[u8]) -> Result<()> {
        let nal = unit
            .strip_prefix(&[0, 0, 0, 1])
            .or_else(|| unit.strip_prefix(&[0, 0, 1]))
            .ok_or_else(|| anyhow!("NAL H.264 sin start code"))?;
        if nal.is_empty() {
            return Ok(());
        }
        match nal[0] & 0x1f {
            7 => self.sps = Some(nal.to_vec()),
            8 => self.pps = Some(nal.to_vec()),
            9 => {} // Access unit delimiters are not needed inside MP4 samples.
            _ => {
                let len = u32::try_from(nal.len()).context("NAL demasiado grande")?;
                self.sample.extend_from_slice(&len.to_be_bytes());
                self.sample.extend_from_slice(nal);
            }
        }
        Ok(())
    }
}

pub fn encode_native(
    renderer: &mut Renderer<'_>,
    output: &Path,
    fps: u32,
    threads: u32,
) -> Result<()> {
    if !(1..=60).contains(&fps) {
        bail!("OpenH264 admite de 1 a 60 fps");
    }
    if !(1..=64).contains(&threads) {
        bail!("threads debe estar entre 1 y 64");
    }
    let (width, height) = renderer.dimensions();
    if width.max(height) > 3840 || width.min(height) > 2160 {
        bail!("OpenH264 admite hasta 3840×2160 o 2160×3840; usa --size 4k o --scale");
    }
    if width % 2 != 0 || height % 2 != 0 {
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
    let frame_count = renderer.frame_count(fps);
    let result = (|| -> Result<()> {
        if threads > 1 && width as u64 * height as u64 >= 1920 * 1080 {
            std::thread::scope(|scope| -> Result<()> {
                let (sender, receiver) = mpsc::sync_channel(1);
                let producer = std::thread::Builder::new()
                    .name("aether-raster".into())
                    .spawn_scoped(scope, move || {
                        for frame in video_frames(renderer, fps) {
                            let failed = frame.is_err();
                            if sender.send(frame).is_err() || failed {
                                break;
                            }
                        }
                    })
                    .context("no se pudo iniciar el hilo de dibujo")?;
                let result = write_video(
                    &partial,
                    (width, height),
                    fps,
                    threads - 1,
                    frame_count,
                    receiver,
                );
                producer
                    .join()
                    .map_err(|_| anyhow!("falló el hilo de dibujo"))?;
                result?;
                Ok(())
            })?;
        } else {
            write_video(
                &partial,
                (width, height),
                fps,
                threads,
                frame_count,
                video_frames(renderer, fps),
            )?;
        }
        fs::rename(&partial, output).context("no se pudo guardar MP4")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

// Keep a small pool because the encoder can still be reading a previous buffer.
fn video_frames<'a, 'composition>(
    renderer: &'a mut Renderer<'composition>,
    fps: u32,
) -> impl Iterator<Item = Result<Arc<YUVBuffer>>> + use<'a, 'composition> {
    let (width, height) = renderer.dimensions();
    let dimensions = (width as usize, height as usize);
    let mut pool: Vec<Arc<YUVBuffer>> = Vec::new();
    let mut current = None;
    let mut last_rasterized = None;
    (0..renderer.frame_count(fps)).map(move |frame| {
        let (rgba, rasterized) = renderer.render_frame_with_revision(frame, fps)?;
        if last_rasterized != Some(rasterized) {
            // The pool owns the last buffer. A single reference means it is safe to refill.
            let index = match pool
                .iter()
                .position(|buffer| Arc::strong_count(buffer) == 1)
            {
                Some(index) => index,
                None => {
                    pool.push(Arc::new(YUVBuffer::new(dimensions.0, dimensions.1)));
                    pool.len() - 1
                }
            };
            let yuv = Arc::get_mut(&mut pool[index]).unwrap();
            yuv.read_rgba8(RgbaSliceU8::new(rgba, dimensions));
            current = Some(index);
            last_rasterized = Some(rasterized);
        }
        Ok(Arc::clone(&pool[current.unwrap()]))
    })
}

fn write_video(
    partial: &Path,
    (width, height): (u32, u32),
    fps: u32,
    threads: u32,
    frame_count: usize,
    frames: impl IntoIterator<Item = Result<Arc<YUVBuffer>>>,
) -> Result<()> {
    let config = EncoderConfig::new()
        .skip_frames(false)
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .num_threads(threads as u16)
        .rate_control_mode(RateControlMode::Off)
        .qp(QpRange::new(24, 32))
        .complexity(Complexity::Low)
        // openh264::formats converts RGB to limited-range BT.601 YUV.
        .vui(VuiConfig::bt601());
    let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config)
        .map_err(|e| anyhow!("OpenH264: {e}"))?;
    let mut writer = None;
    let mut written = 0;
    for (frame, yuv) in frames.into_iter().enumerate() {
        let yuv = yuv?;
        let timestamp = Timestamp::from_millis(frame as u64 * 1000 / u64::from(fps));
        let encoded = encoder
            .encode_at(yuv.as_ref(), timestamp)
            .map_err(|e| anyhow!("OpenH264 frame {frame}: {e}"))?;
        let frame_type = encoded.frame_type();
        if matches!(frame_type, FrameType::Skip | FrameType::Invalid) {
            bail!("OpenH264 omitió el frame {frame}: {frame_type:?}");
        }
        let mut nal = NalFrame::default();
        for i in 0..encoded.num_layers() {
            let layer = encoded.layer(i).unwrap();
            for j in 0..layer.nal_count() {
                nal.push(layer.nal_unit(j).unwrap())
                    .with_context(|| format!("frame {frame}"))?;
            }
        }
        if nal.sample.is_empty() {
            bail!("OpenH264 no produjo datos de video para el frame {frame}");
        }
        if writer.is_none() {
            if frame_type != FrameType::IDR {
                bail!("el primer frame H.264 no es IDR");
            }
            let (sps, pps) = (
                nal.sps.ok_or_else(|| anyhow!("falta SPS H.264"))?,
                nal.pps.ok_or_else(|| anyhow!("falta PPS H.264"))?,
            );
            let file = File::create(partial).context("no se pudo crear MP4")?;
            let container = Mp4Config {
                major_brand: "isom".parse()?,
                minor_version: 512,
                compatible_brands: ["isom", "iso2", "avc1", "mp41"]
                    .into_iter()
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()?,
                timescale: fps,
            };
            let mut mp4 = Mp4Writer::write_start(file, &container)?;
            let mut track: TrackConfig = AvcConfig {
                width: width as u16,
                height: height as u16,
                seq_param_set: sps,
                pic_param_set: pps,
            }
            .into();
            track.timescale = fps;
            mp4.add_track(&track)?;
            writer = Some(mp4);
        }
        writer.as_mut().unwrap().write_sample(
            1,
            &Mp4Sample {
                start_time: frame as u64,
                duration: 1,
                rendering_offset: 0,
                is_sync: frame_type == FrameType::IDR,
                bytes: Bytes::from(nal.sample),
            },
        )?;
        written += 1;
    }
    if written != frame_count {
        bail!("se produjeron {written} de {frame_count} fotogramas");
    }
    let mut writer = writer.ok_or_else(|| anyhow!("composición sin frames"))?;
    writer.write_end()?;
    drop(writer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_annex_b_to_length_prefixed_sample() {
        let packet = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4, 5,
        ];
        let mut frame = NalFrame::default();
        for unit in openh264::nal_units(&packet) {
            frame.push(unit).unwrap();
        }
        assert_eq!(frame.sps.unwrap(), [0x67, 1, 2]);
        assert_eq!(frame.pps.unwrap(), [0x68, 3]);
        assert_eq!(frame.sample, [0, 0, 0, 3, 0x65, 4, 5]);
    }
}
