use crate::render::Renderer;
use anyhow::{anyhow, bail, Context, Result};
use mp4::{AvcConfig, Bytes, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
use openh264::encoder::{
    Complexity, Encoder, EncoderConfig, FrameRate, FrameType, QpRange, RateControlMode, VuiConfig,
};
use openh264::formats::{RgbaSliceU8, YUVBuffer};
use openh264::{OpenH264API, Timestamp};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
            .ok_or_else(|| anyhow!("H.264 NAL unit has no start code"))?;
        if nal.is_empty() {
            return Ok(());
        }
        match nal[0] & 0x1f {
            7 => self.sps = Some(nal.to_vec()),
            8 => self.pps = Some(nal.to_vec()),
            9 => {} // Access unit delimiters are not needed inside MP4 samples.
            _ => {
                let len = u32::try_from(nal.len()).context("NAL unit is too large")?;
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
        bail!("OpenH264 supports 1 to 60 fps");
    }
    if !(1..=64).contains(&threads) {
        bail!("threads must be between 1 and 64");
    }
    let (width, height) = renderer.dimensions();
    if width.max(height) > 3840 || width.min(height) > 2160 {
        bail!("OpenH264 supports up to 3840×2160 or 2160×3840; use --size 4k or --scale");
    }
    if width % 2 != 0 || height % 2 != 0 {
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
    let frame_count = renderer.frame_count(fps);
    let result = (|| -> Result<()> {
        if frame_count > fps as usize * SEGMENT_SECONDS {
            // Long enough for several independent GOPs: encode them in parallel.
            let (rasterized, reused) =
                write_segmented(renderer, &partial, fps, threads as usize, frame_count)?;
            renderer.add_stats(rasterized, reused);
        } else if threads > 1 && width as u64 * height as u64 >= 1920 * 1080 {
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
                    .context("could not start the drawing thread")?;
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
                    .map_err(|_| anyhow!("drawing thread failed"))?;
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
        fs::rename(&partial, output).context("could not save MP4")?;
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

type Sample = (Vec<u8>, bool);

struct EncodedSegment {
    index: usize,
    rasterized: usize,
    reused: usize,
    samples: Vec<Sample>,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

fn make_encoder(fps: u32, threads: u32) -> Result<Encoder> {
    let config = EncoderConfig::new()
        .skip_frames(false)
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .num_threads(threads as u16)
        .rate_control_mode(RateControlMode::Off)
        .qp(QpRange::new(24, 32))
        .complexity(Complexity::Low)
        // openh264::formats converts RGB to limited-range BT.601 YUV.
        .vui(VuiConfig::bt601());
    Encoder::with_api_config(OpenH264API::from_source(), config)
        .map_err(|e| anyhow!("OpenH264: {e}"))
}

/// Encodes one frame into an MP4 sample (length-prefixed NAL units).
fn encode_one(
    encoder: &mut Encoder,
    yuv: &YUVBuffer,
    frame: usize,
    fps: u32,
) -> Result<(NalFrame, FrameType)> {
    let timestamp = Timestamp::from_millis(frame as u64 * 1000 / u64::from(fps));
    let encoded = encoder
        .encode_at(yuv, timestamp)
        .map_err(|e| anyhow!("OpenH264 frame {frame}: {e}"))?;
    let frame_type = encoded.frame_type();
    if matches!(frame_type, FrameType::Skip | FrameType::Invalid) {
        bail!("OpenH264 omitted frame {frame}: {frame_type:?}");
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
        bail!("OpenH264 produced no video data for frame {frame}");
    }
    Ok((nal, frame_type))
}

fn start_mp4(
    partial: &Path,
    (width, height): (u32, u32),
    fps: u32,
    sps: Vec<u8>,
    pps: Vec<u8>,
) -> Result<Mp4Writer<File>> {
    let file = File::create(partial).context("could not create MP4")?;
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
    Ok(mp4)
}

fn write_sample(writer: &mut Mp4Writer<File>, frame: usize, (bytes, sync): Sample) -> Result<()> {
    writer.write_sample(
        1,
        &Mp4Sample {
            start_time: frame as u64,
            duration: 1,
            rendering_offset: 0,
            is_sync: sync,
            bytes: Bytes::from(bytes),
        },
    )?;
    Ok(())
}

fn write_video(
    partial: &Path,
    dimensions: (u32, u32),
    fps: u32,
    threads: u32,
    frame_count: usize,
    frames: impl IntoIterator<Item = Result<Arc<YUVBuffer>>>,
) -> Result<()> {
    let mut encoder = make_encoder(fps, threads)?;
    let mut writer = None;
    let mut written = 0;
    for (frame, yuv) in frames.into_iter().enumerate() {
        let yuv = yuv?;
        let (nal, frame_type) = encode_one(&mut encoder, &yuv, frame, fps)?;
        if writer.is_none() {
            if frame_type != FrameType::IDR {
                bail!("the first H.264 frame is not IDR");
            }
            let (sps, pps) = (
                nal.sps.ok_or_else(|| anyhow!("missing H.264 SPS"))?,
                nal.pps.ok_or_else(|| anyhow!("missing H.264 PPS"))?,
            );
            writer = Some(start_mp4(partial, dimensions, fps, sps, pps)?);
        }
        write_sample(
            writer.as_mut().unwrap(),
            frame,
            (nal.sample, frame_type == FrameType::IDR),
        )?;
        written += 1;
    }
    if written != frame_count {
        bail!("produced {written} of {frame_count} frames");
    }
    let mut writer = writer.ok_or_else(|| anyhow!("composition has no frames"))?;
    writer.write_end()?;
    Ok(())
}

/// Seconds of video per independently encoded segment (each starts with an IDR frame).
/// Fixed, so the MP4 is byte-identical for any worker count.
const SEGMENT_SECONDS: usize = 1;

/// Worker count that fits this machine: all cores up to 4, because each 4K worker holds
/// roughly 140 MB. Old PCs get what they have; large machines do not exhaust memory.
pub fn default_threads() -> u32 {
    std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 4) as u32
}

fn encode_segment(
    renderer: &mut Renderer<'_>,
    index: usize,
    range: std::ops::Range<usize>,
    fps: u32,
) -> Result<EncodedSegment> {
    let (width, height) = renderer.dimensions();
    let dims = (width as usize, height as usize);
    let mut encoder = make_encoder(fps, 1)?;
    let mut yuv = YUVBuffer::new(dims.0, dims.1);
    let mut last_revision = None;
    let before = (renderer.stats().rasterized, renderer.stats().reused);
    let mut samples = Vec::with_capacity(range.len());
    let (mut sps, mut pps) = (None, None);
    for frame in range {
        let (rgba, revision) = renderer.render_frame_with_revision(frame, fps)?;
        if last_revision != Some(revision) {
            yuv.read_rgba8(RgbaSliceU8::new(rgba, dims));
            last_revision = Some(revision);
        }
        let (nal, frame_type) = encode_one(&mut encoder, &yuv, frame, fps)?;
        if samples.is_empty() {
            if frame_type != FrameType::IDR {
                bail!("the first frame of segment {index} is not IDR");
            }
            sps = nal.sps;
            pps = nal.pps;
        }
        samples.push((nal.sample, frame_type == FrameType::IDR));
    }
    Ok(EncodedSegment {
        index,
        rasterized: renderer.stats().rasterized - before.0,
        reused: renderer.stats().reused - before.1,
        samples,
        sps: sps.ok_or_else(|| anyhow!("missing H.264 SPS en segmento {index}"))?,
        pps: pps.ok_or_else(|| anyhow!("missing H.264 PPS en segmento {index}"))?,
    })
}

/// Renders and encodes independent segments on `workers` threads, then muxes them in order.
fn write_segmented(
    renderer: &Renderer<'_>,
    partial: &Path,
    fps: u32,
    workers: usize,
    frame_count: usize,
) -> Result<(usize, usize)> {
    let segment_len = fps as usize * SEGMENT_SECONDS;
    let segments = frame_count.div_ceil(segment_len);
    let workers = workers.min(segments);
    let forks = (0..workers)
        .map(|_| renderer.fork())
        .collect::<Result<Vec<_>>>()?;
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let dimensions = renderer.dimensions();
    let (sender, receiver) = mpsc::channel::<Result<EncodedSegment>>();
    std::thread::scope(|scope| -> Result<(usize, usize)> {
        for mut fork in forks {
            let (sender, next, failed) = (sender.clone(), &next, &failed);
            std::thread::Builder::new()
                .name("aether-segment".into())
                .spawn_scoped(scope, move || loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= segments || failed.load(Ordering::Relaxed) {
                        break;
                    }
                    let start = index * segment_len;
                    let end = (start + segment_len).min(frame_count);
                    let result = encode_segment(&mut fork, index, start..end, fps);
                    if result.is_err() {
                        failed.store(true, Ordering::Relaxed);
                    }
                    if sender.send(result).is_err() {
                        break;
                    }
                })
                .context("could not start an encoding thread")?;
        }
        drop(sender);
        let mut pending: BTreeMap<usize, EncodedSegment> = BTreeMap::new();
        let mut writer: Option<Mp4Writer<File>> = None;
        let mut parameter_sets: Option<(Vec<u8>, Vec<u8>)> = None;
        let (mut want, mut frame) = (0, 0);
        let mut totals = (0, 0);
        for message in receiver {
            let segment = message?;
            pending.insert(segment.index, segment);
            while let Some(segment) = pending.remove(&want) {
                match &parameter_sets {
                    None => {
                        writer = Some(start_mp4(
                            partial,
                            dimensions,
                            fps,
                            segment.sps.clone(),
                            segment.pps.clone(),
                        )?);
                        parameter_sets = Some((segment.sps, segment.pps));
                    }
                    Some((sps, pps)) if *sps != segment.sps || *pps != segment.pps => {
                        bail!("H.264 segments do not share SPS/PPS")
                    }
                    Some(_) => {}
                }
                totals.0 += segment.rasterized;
                totals.1 += segment.reused;
                for sample in segment.samples {
                    write_sample(writer.as_mut().unwrap(), frame, sample)?;
                    frame += 1;
                }
                want += 1;
            }
        }
        if frame != frame_count {
            bail!("produced {frame} of {frame_count} frames");
        }
        writer
            .ok_or_else(|| anyhow!("composition has no frames"))?
            .write_end()?;
        Ok(totals)
    })
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
