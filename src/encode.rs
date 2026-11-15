use crate::render::Renderer;
use anyhow::{anyhow, bail, Context, Result};
use mp4::{AvcConfig, Bytes, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
use openh264::encoder::{
    Complexity, Encoder, EncoderConfig, FrameRate, FrameType, QpRange, RateControlMode, VuiConfig,
};
use openh264::formats::{RgbaSliceU8, YUVBuffer};
use openh264::{nal_units, OpenH264API};
use std::fs::{self, File};
use std::path::Path;

struct NalFrame {
    sample: Vec<u8>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

fn nal_frame(annex_b: &[u8]) -> Result<NalFrame> {
    let mut result = NalFrame {
        sample: Vec::new(),
        sps: None,
        pps: None,
    };
    for unit in nal_units(annex_b) {
        let nal = unit
            .strip_prefix(&[0, 0, 0, 1])
            .or_else(|| unit.strip_prefix(&[0, 0, 1]))
            .ok_or_else(|| anyhow!("NAL H.264 sin start code"))?;
        if nal.is_empty() {
            continue;
        }
        match nal[0] & 0x1f {
            7 => result.sps = Some(nal.to_vec()),
            8 => result.pps = Some(nal.to_vec()),
            9 => {} // Access unit delimiters are not needed inside MP4 samples.
            _ => {
                let len = u32::try_from(nal.len()).context("NAL demasiado grande")?;
                result.sample.extend_from_slice(&len.to_be_bytes());
                result.sample.extend_from_slice(nal);
            }
        }
    }
    if result.sample.is_empty() {
        bail!("OpenH264 no produjo un frame de video");
    }
    Ok(result)
}

pub fn encode_native(
    renderer: &mut Renderer<'_>,
    output: &Path,
    fps: u32,
    threads: u32,
) -> Result<()> {
    let (width, height) = renderer.dimensions();
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
    let result = (|| -> Result<()> {
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
        let mut yuv = YUVBuffer::new(width as usize, height as usize);
        let mut writer = None;
        for frame in 0..renderer.frame_count(fps) {
            let rgba = renderer.render_frame(frame, fps)?;
            yuv.read_rgba8(RgbaSliceU8::new(rgba, (width as usize, height as usize)));
            let encoded = encoder
                .encode(&yuv)
                .map_err(|e| anyhow!("OpenH264 frame {frame}: {e}"))?;
            let frame_type = encoded.frame_type();
            if matches!(frame_type, FrameType::Skip | FrameType::Invalid) {
                bail!("OpenH264 omitió el frame {frame}: {frame_type:?}");
            }
            let nal = nal_frame(&encoded.to_vec()).with_context(|| format!("frame {frame}"))?;
            if writer.is_none() {
                if frame_type != FrameType::IDR {
                    bail!("el primer frame H.264 no es IDR");
                }
                let (sps, pps) = (
                    nal.sps.ok_or_else(|| anyhow!("falta SPS H.264"))?,
                    nal.pps.ok_or_else(|| anyhow!("falta PPS H.264"))?,
                );
                let file = File::create(&partial).context("no se pudo crear MP4")?;
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
        }
        let mut writer = writer.ok_or_else(|| anyhow!("composición sin frames"))?;
        writer.write_end()?;
        drop(writer);
        fs::rename(&partial, output).context("no se pudo guardar MP4")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_annex_b_to_length_prefixed_sample() {
        let packet = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4, 5,
        ];
        let frame = nal_frame(&packet).unwrap();
        assert_eq!(frame.sps.unwrap(), [0x67, 1, 2]);
        assert_eq!(frame.pps.unwrap(), [0x68, 3]);
        assert_eq!(frame.sample, [0, 0, 0, 3, 0x65, 4, 5]);
    }
}
