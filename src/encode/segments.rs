//! Parallel segment pipeline: a render is cut into fixed one-second segments, each rendered and
//! encoded by its own OpenH264 encoder (so it starts with an IDR frame) on a pool of forked
//! renderers, and handed to a sink in segment order.
//!
//! Segment boundaries depend only on the frame rate, so any worker count or split of the segments
//! between processes (see `parts.rs`) encodes the same frames with the same settings.

use super::{encode_one, make_encoder, Quality, Sample};
use crate::render::Renderer;
use anyhow::{anyhow, bail, Context, Result};
use openh264::encoder::FrameType;
use openh264::formats::{RgbaSliceU8, YUVBuffer};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;

/// Seconds of video per independently encoded segment. Fixed, so segment boundaries never depend on
/// the worker count or the distribution of segments.
const SEGMENT_SECONDS: usize = 1;

/// Frames per segment at `fps`.
pub fn segment_frames(fps: u32) -> usize {
    fps as usize * SEGMENT_SECONDS
}

/// Number of segments of a render of `frame_count` frames, or `None` when the render is not longer
/// than one segment (it then uses the single-encoder pipeline and cannot be split).
pub fn segment_count(frame_count: usize, fps: u32) -> Option<usize> {
    let len = segment_frames(fps);
    (len > 0 && frame_count > len).then(|| frame_count.div_ceil(len))
}

/// One encoded segment: its MP4 samples in display order and the parameter sets of its encoder.
pub(super) struct EncodedSegment {
    pub index: usize,
    pub rasterized: usize,
    pub reused: usize,
    pub samples: Vec<Sample>,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

fn encode_segment(
    renderer: &mut Renderer<'_>,
    index: usize,
    range: Range<usize>,
    (fps, quality): (u32, Quality),
) -> Result<EncodedSegment> {
    let (width, height) = renderer.dimensions();
    let dims = (width as usize, height as usize);
    let mut encoder = make_encoder(fps, 1, quality)?;
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
        sps: sps.ok_or_else(|| anyhow!("missing H.264 SPS in segment {index}"))?,
        pps: pps.ok_or_else(|| anyhow!("missing H.264 PPS in segment {index}"))?,
    })
}

/// Renders and encodes the segments in `segments` on up to `workers` threads and passes each one
/// to `sink` in index order, after checking that every segment shares the first one's SPS/PPS.
/// Returns the (rasterized, reused) frame counts.
pub(super) fn encode_segments(
    renderer: &Renderer<'_>,
    segments: Range<usize>,
    (fps, quality): (u32, Quality),
    workers: usize,
    frame_count: usize,
    mut sink: impl FnMut(EncodedSegment) -> Result<()>,
) -> Result<(usize, usize)> {
    let segment_len = segment_frames(fps);
    if segments.is_empty() || segments.end > frame_count.div_ceil(segment_len.max(1)) {
        bail!("segment range {segments:?} is outside the render");
    }
    let workers = workers.clamp(1, segments.len());
    let forks = (0..workers)
        .map(|_| renderer.fork())
        .collect::<Result<Vec<_>>>()?;
    let next = AtomicUsize::new(segments.start);
    let failed = AtomicBool::new(false);
    let (sender, receiver) = mpsc::channel::<Result<EncodedSegment>>();
    std::thread::scope(|scope| -> Result<(usize, usize)> {
        for mut fork in forks {
            let (sender, next, failed, end) = (sender.clone(), &next, &failed, segments.end);
            std::thread::Builder::new()
                .name("aether-segment".into())
                .spawn_scoped(scope, move || loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= end || failed.load(Ordering::Relaxed) {
                        break;
                    }
                    let start = index * segment_len;
                    let stop = (start + segment_len).min(frame_count);
                    let result = encode_segment(&mut fork, index, start..stop, (fps, quality));
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
        let mut parameter_sets: Option<(Vec<u8>, Vec<u8>)> = None;
        let mut want = segments.start;
        let mut totals = (0, 0);
        for message in receiver {
            let segment = message?;
            pending.insert(segment.index, segment);
            while let Some(segment) = pending.remove(&want) {
                match &parameter_sets {
                    None => parameter_sets = Some((segment.sps.clone(), segment.pps.clone())),
                    Some((sps, pps)) if *sps != segment.sps || *pps != segment.pps => {
                        bail!("H.264 segments do not share SPS/PPS")
                    }
                    Some(_) => {}
                }
                totals.0 += segment.rasterized;
                totals.1 += segment.reused;
                sink(segment)?;
                want += 1;
            }
        }
        if want != segments.end {
            bail!(
                "encoded {} of {} segments",
                want - segments.start,
                segments.len()
            );
        }
        Ok(totals)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_renders_longer_than_one_segment_are_segmented() {
        assert_eq!(segment_count(30, 30), None);
        assert_eq!(segment_count(31, 30), Some(2));
        assert_eq!(segment_count(90, 30), Some(3));
        assert_eq!(segment_count(91, 30), Some(4));
        assert_eq!(segment_frames(24), 24);
    }
}
