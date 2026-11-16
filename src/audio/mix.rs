//! Sample-accurate mixing of `<audio>` clips onto one 48 kHz stereo timeline.
//!
//! Every time is converted to a whole sample with `round(seconds * 48000)`, and a clip's length is
//! the difference of its rounded end and start, so adjacent clips neither overlap nor leave gaps.
//! Fades are linear in amplitude and evaluated per sample. Mono sources play on both channels at
//! unity gain. Mixing is plain summation in `f32` in document order, so output is deterministic.
use super::decode::{decode_file, Pcm};
use super::resample::resample;
use super::SAMPLE_RATE;
use crate::model::{AudioClip, Composition};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

/// Mixed audio is held as two `f32` planes; this caps it at 512 MiB (about 22 minutes).
const MAX_FRAMES: usize = 64 * 1024 * 1024;

/// Whole samples at 48 kHz for a time in seconds.
pub fn frames_for(seconds: f64) -> usize {
    (seconds * f64::from(SAMPLE_RATE)).round() as usize
}

/// One source file converted to the mix rate.
struct Source {
    channels: Vec<Vec<f32>>,
    rate: u32,
}

impl Source {
    fn frames(&self) -> usize {
        self.channels[0].len()
    }
}

/// Where and how long one clip plays, in mix samples, for `check` and tests.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipReport {
    pub id: Option<String>,
    pub src: PathBuf,
    pub source_rate: u32,
    pub source_channels: usize,
    pub source_seconds: f64,
    pub start_frame: usize,
    pub frames: usize,
}

/// The mixed timeline: left and right planes of the same length.
pub struct Mix {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Largest absolute sample before conversion to 16 bits.
    pub peak: f32,
    /// Samples (per channel) whose absolute value exceeds full scale and will be clamped.
    pub clipped: usize,
    pub clips: Vec<ClipReport>,
}

impl Mix {
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    /// Interleaved 16-bit samples, rounded to nearest and clamped.
    pub fn to_i16_interleaved(&self) -> Vec<i16> {
        let convert = |x: f32| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        self.left
            .iter()
            .zip(&self.right)
            .flat_map(|(l, r)| [convert(*l), convert(*r)])
            .collect()
    }
}

fn load(path: &PathBuf, cache: &mut HashMap<PathBuf, Source>) -> Result<()> {
    if cache.contains_key(path) {
        return Ok(());
    }
    let Pcm { rate, channels } = decode_file(path)?;
    let channels = channels
        .iter()
        .map(|plane| resample(plane, rate, SAMPLE_RATE))
        .collect();
    cache.insert(path.clone(), Source { channels, rate });
    Ok(())
}

/// Mixes the composition's clips into exactly `total_frames` samples per channel.
pub fn mix(composition: &Composition, total_frames: usize) -> Result<Mix> {
    if total_frames == 0 || total_frames > MAX_FRAMES {
        bail!(
            "audio length of {total_frames} samples is outside 1..={MAX_FRAMES} (about 22 minutes)"
        );
    }
    let mut left = vec![0.0_f32; total_frames];
    let mut right = vec![0.0_f32; total_frames];
    let mut cache = HashMap::new();
    let mut clips = Vec::with_capacity(composition.audio.len());
    for clip in &composition.audio {
        load(&clip.src, &mut cache)?;
        let source = &cache[&clip.src];
        let report = add_clip(clip, source, &mut left, &mut right)
            .with_context(|| format!("audio clip {}", clip.id.as_deref().unwrap_or("no id")))?;
        clips.push(report);
    }
    let (mut peak, mut clipped) = (0.0_f32, 0);
    for (l, r) in left.iter().zip(&right) {
        let loudest = l.abs().max(r.abs());
        peak = peak.max(loudest);
        clipped += usize::from(loudest > 1.0);
    }
    Ok(Mix {
        left,
        right,
        peak,
        clipped,
        clips,
    })
}

fn add_clip(
    clip: &AudioClip,
    source: &Source,
    left: &mut [f32],
    right: &mut [f32],
) -> Result<ClipReport> {
    let total = left.len();
    let start = frames_for(clip.start);
    let trim = frames_for(clip.trim);
    if start >= total {
        bail!("starts after the end of the audio timeline");
    }
    if trim >= source.frames() {
        bail!(
            "data-hf-trim-start ({} s) is not shorter than the source ({:.3} s)",
            clip.trim,
            source.frames() as f64 / f64::from(SAMPLE_RATE)
        );
    }
    let available = source.frames() - trim;
    let length = match clip.duration {
        Some(duration) => frames_for(clip.start + duration).saturating_sub(start),
        None => available,
    }
    .min(total - start);
    let (fade_in, fade_out) = (frames_for(clip.fade_in), frames_for(clip.fade_out));
    let volume = f64::from(clip.volume);
    let right_plane = source.channels.get(1).unwrap_or(&source.channels[0]);
    for k in 0..length {
        let index = if clip.looped {
            trim + k % available
        } else if k < available {
            trim + k
        } else {
            break;
        };
        let mut gain = volume;
        if k < fade_in {
            gain *= k as f64 / fade_in as f64;
        }
        if k + fade_out > length {
            gain *= (length - k) as f64 / fade_out as f64;
        }
        left[start + k] += (f64::from(source.channels[0][index]) * gain) as f32;
        right[start + k] += (f64::from(right_plane[index]) * gain) as f32;
    }
    Ok(ClipReport {
        id: clip.id.clone(),
        src: clip.src.clone(),
        source_rate: source.rate,
        source_channels: source.channels.len(),
        source_seconds: source.frames() as f64 / f64::from(SAMPLE_RATE),
        start_frame: start,
        frames: length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(samples: Vec<f32>) -> Source {
        Source {
            channels: vec![samples],
            rate: SAMPLE_RATE,
        }
    }

    fn clip() -> AudioClip {
        AudioClip {
            id: None,
            src: PathBuf::from("x.wav"),
            start: 0.0,
            duration: None,
            trim: 0.0,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            looped: false,
        }
    }

    fn run(clip: &AudioClip, source: &Source, total: usize) -> (Vec<f32>, ClipReport) {
        let (mut l, mut r) = (vec![0.0; total], vec![0.0; total]);
        let report = add_clip(clip, source, &mut l, &mut r).unwrap();
        assert_eq!(l, r, "mono plays identically on both channels");
        (l, report)
    }

    #[test]
    fn places_the_first_sample_exactly_at_the_start() {
        let mut c = clip();
        c.start = 0.5;
        let (out, report) = run(&c, &source(vec![1.0; 100]), 48_000);
        assert_eq!(report.start_frame, 24_000);
        assert_eq!(out[23_999], 0.0);
        assert_eq!(out[24_000], 1.0);
        assert_eq!(out[24_099], 1.0);
        assert_eq!(out[24_100], 0.0);
    }

    #[test]
    fn adjacent_clips_share_a_boundary_without_gap_or_overlap() {
        // 1/3 s lasts 16000 samples; two consecutive clips must tile exactly.
        let (mut a, mut b) = (clip(), clip());
        a.duration = Some(1.0 / 3.0);
        b.start = 1.0 / 3.0;
        b.duration = Some(1.0 / 3.0);
        let src = source(vec![1.0; 48_000]);
        let (out_a, _) = run(&a, &src, 48_000);
        let (out_b, _) = run(&b, &src, 48_000);
        for i in 0..48_000 {
            assert_eq!(out_a[i] + out_b[i], f32::from(i < 32_000), "sample {i}");
        }
    }

    #[test]
    fn trim_skips_source_samples() {
        let samples: Vec<f32> = (0..48_000).map(|i| i as f32).collect();
        let mut c = clip();
        c.trim = 0.25;
        c.duration = Some(0.01);
        let (out, report) = run(&c, &source(samples), 48_000);
        assert_eq!(out[0], 12_000.0);
        assert_eq!(report.frames, 480);
    }

    #[test]
    fn linear_fades_reach_unity_and_zero_at_the_edges() {
        let mut c = clip();
        c.duration = Some(0.1);
        c.fade_in = 0.01;
        c.fade_out = 0.01;
        let (out, _) = run(&c, &source(vec![1.0; 48_000]), 48_000);
        assert_eq!(out[0], 0.0);
        assert!((out[240] - 0.5).abs() < 1e-6);
        assert_eq!(out[480], 1.0);
        assert_eq!(out[4319], 1.0);
        assert!((out[4560] - 0.5).abs() < 1e-6);
        assert!((out[4799] - 1.0 / 480.0).abs() < 1e-6);
    }

    #[test]
    fn loops_the_trimmed_source() {
        let mut c = clip();
        c.looped = true;
        c.trim = 1.0 / 48_000.0;
        c.duration = Some(7.0 / 48_000.0);
        let (out, _) = run(&c, &source(vec![9.0, 1.0, 2.0, 3.0]), 48_000);
        assert_eq!(&out[..8], &[1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0, 0.0]);
    }

    #[test]
    fn stops_when_the_source_ends_before_the_duration() {
        let mut c = clip();
        c.duration = Some(1.0);
        let (out, _) = run(&c, &source(vec![1.0; 10]), 48_000);
        assert_eq!(out.iter().filter(|s| **s != 0.0).count(), 10);
    }

    #[test]
    fn rejects_a_trim_beyond_the_source() {
        let mut c = clip();
        c.trim = 1.0;
        let (mut l, mut r) = (vec![0.0; 48_000], vec![0.0; 48_000]);
        assert!(add_clip(&c, &source(vec![1.0; 100]), &mut l, &mut r).is_err());
    }

    #[test]
    fn converts_to_clamped_16_bit() {
        let mix = Mix {
            left: vec![0.0, 1.5, -1.5, 0.5],
            right: vec![0.0, 0.0, 0.0, -0.5],
            peak: 1.5,
            clipped: 2,
            clips: Vec::new(),
        };
        assert_eq!(
            mix.to_i16_interleaved(),
            [0, 0, 32767, 0, -32768, 0, 16384, -16384]
        );
    }
}
