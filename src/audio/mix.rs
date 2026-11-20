//! Sample-accurate mixing of `<audio>` clips onto one 48 kHz stereo timeline.
//!
//! Every time is converted to a whole sample with `round(seconds * 48000)`, and a clip's length is
//! the difference of its rounded end and start, so adjacent clips neither overlap nor leave gaps.
//! Fades are linear in amplitude and evaluated per sample. Mono sources play on both channels at
//! unity gain. Mixing is plain summation in `f32` in document order, so output is deterministic.
//! Clips with `data-hf-speed` or `data-hf-speed-ramp` read the source through `varispeed`; clips at
//! 1x keep the direct sample copy. Clips with effects (`data-hf-volume-ramp`, `data-hf-eq`,
//! `data-hf-duck`) run through `effects::Chain`; ducked clips are mixed after their triggers.
use super::decode::{decode_file, Pcm};
use super::effects::duck::{self, Key};
use super::effects::Chain;
use super::resample::resample;
use super::varispeed::{Edge, Varispeed, SILENT_BELOW};
use super::SAMPLE_RATE;
use crate::model::{AudioClip, Composition};
use anyhow::{anyhow, bail, Context, Result};
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
    /// Source seconds played (after the trim, counting every loop pass).
    pub media_seconds: f64,
    /// The clip's speed curve as `check` reports it; `None` at 1x.
    pub speed: Option<serde_json::Value>,
    /// Volume automation, equalizer, and ducking as `check` reports them; `None` without effects.
    pub effects: Option<serde_json::Value>,
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
///
/// Clips are mixed in document order, except that every clip another clip ducks under
/// (`data-hf-duck`) is mixed first, so its post-effects level can key the ducking.
pub fn mix(composition: &Composition, total_frames: usize) -> Result<Mix> {
    if total_frames == 0 || total_frames > MAX_FRAMES {
        bail!(
            "audio length of {total_frames} samples is outside 1..={MAX_FRAMES} (about 22 minutes)"
        );
    }
    let mut left = vec![0.0_f32; total_frames];
    let mut right = vec![0.0_f32; total_frames];
    let mut cache = HashMap::new();
    let audio = &composition.audio;
    let varispeed = audio.iter().any(|c| c.speed.is_some()).then(Varispeed::new);
    let triggers: Vec<&[usize]> = audio
        .iter()
        .map(|c| c.effects.duck.as_ref().map_or(&[][..], |d| &d.sources[..]))
        .collect();
    let order = duck::processing_order(&triggers).map_err(|i| {
        anyhow!(
            "audio clip {}: data-hf-duck forms a cycle",
            label(&audio[i])
        )
    })?;
    let keyed: Vec<bool> = (0..audio.len())
        .map(|i| triggers.iter().any(|t| t.contains(&i)))
        .collect();
    let mut keys: HashMap<usize, Key> = HashMap::new();
    let mut reports: Vec<Option<ClipReport>> = vec![None; audio.len()];
    for index in order {
        let clip = &audio[index];
        load(&clip.src, &mut cache)?;
        let source = &cache[&clip.src];
        let mut key = keyed[index].then(Vec::new);
        let report = (|| -> Result<ClipReport> {
            let placement = place(clip, source, total_frames)?;
            let envelope = clip.effects.duck.as_ref().map(|d| {
                if placement.length == 0 {
                    return duck::Envelope {
                        gains: Vec::new(),
                        regions: Vec::new(),
                    };
                }
                let sources: Vec<&Key> = d.sources.iter().map(|s| &keys[s]).collect();
                d.envelope(&sources, placement.start, placement.length, total_frames)
            });
            if let Some(key) = &mut key {
                key.resize(placement.length, 0.0);
            }
            let start = placement.start;
            let report = render(
                clip,
                source,
                &placement,
                varispeed.as_ref(),
                envelope,
                key.as_deref_mut(),
                &mut left,
                &mut right,
            )?;
            if let Some(level) = key.take() {
                keys.insert(index, Key { start, level });
            }
            Ok(report)
        })()
        .with_context(|| format!("audio clip {}", label(clip)))?;
        reports[index] = Some(report);
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
        clips: reports
            .into_iter()
            .map(|r| r.expect("every clip is mixed"))
            .collect(),
    })
}

fn label(clip: &AudioClip) -> &str {
    clip.id.as_deref().unwrap_or("no id")
}

/// Where one clip plays on the timeline and which source samples it reads, in mix samples.
struct Placement {
    start: usize,
    trim: usize,
    length: usize,
    /// Source samples after the trim.
    available: usize,
    /// Loop passes restart at `loop_start` (never after the trim) and run to the source end.
    loop_start: usize,
    loop_span: usize,
}

fn place(clip: &AudioClip, source: &Source, total: usize) -> Result<Placement> {
    let start = frames_for(clip.start);
    let limit = clip.scene_end.map(|end| frames_for(end).min(total));
    let trim = frames_for(clip.trim);
    if start >= total && limit.is_none() {
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
    let loop_start = frames_for(clip.loop_from).min(trim);
    let rate = f64::from(SAMPLE_RATE);
    let length = match (clip.duration, &clip.speed) {
        (Some(duration), _) => frames_for(clip.start + duration).saturating_sub(start),
        (None, None) => available,
        (None, Some(curve)) => match curve.time_for(available as f64 / rate) {
            Some(time) => frames_for(clip.start + time).saturating_sub(start),
            None => {
                bail!("the speed ramp ends at speed 0 before the source ends; set data-duration")
            }
        },
    }
    .min(limit.unwrap_or(total).saturating_sub(start));
    Ok(Placement {
        start,
        trim,
        length,
        available,
        loop_start,
        loop_span: source.frames() - loop_start,
    })
}

/// Mixes one clip without ducking (used by the unit tests).
#[cfg(test)]
fn add_clip(
    clip: &AudioClip,
    source: &Source,
    varispeed: Option<&Varispeed>,
    left: &mut [f32],
    right: &mut [f32],
) -> Result<ClipReport> {
    let placement = place(clip, source, left.len())?;
    render(clip, source, &placement, varispeed, None, None, left, right)
}

/// Adds one placed clip to the mix through its effect chain; when `key` is given, also records the
/// clip's post-effects level (louder channel) for the clips it ducks.
#[allow(clippy::too_many_arguments)]
fn render(
    clip: &AudioClip,
    source: &Source,
    placement: &Placement,
    varispeed: Option<&Varispeed>,
    envelope: Option<duck::Envelope>,
    mut key: Option<&mut [f32]>,
    left: &mut [f32],
    right: &mut [f32],
) -> Result<ClipReport> {
    let &Placement {
        start,
        trim,
        length,
        available,
        loop_start,
        loop_span,
    } = placement;
    let wrap = |position: f64| {
        if clip.looped && position >= source.frames() as f64 {
            loop_start as f64 + (position - loop_start as f64) % loop_span as f64
        } else {
            position
        }
    };
    let rate = f64::from(SAMPLE_RATE);
    let (fade_in, fade_out) = (frames_for(clip.fade_in), frames_for(clip.fade_out));
    let volume = f64::from(clip.volume);
    let right_plane = source.channels.get(1).unwrap_or(&source.channels[0]);
    let fade = |k: usize| {
        let mut gain = volume;
        if k < fade_in {
            gain *= k as f64 / fade_in as f64;
        }
        if k + fade_out > length {
            gain *= (length - k) as f64 / fade_out as f64;
        }
        gain
    };
    let mut effects_report = serde_json::Map::new();
    if let Some(curve) = &clip.effects.volume {
        effects_report.insert("volumeRamp".into(), curve.to_json());
    }
    if let Some(eq) = &clip.effects.eq {
        effects_report.insert("eq".into(), eq.to_json());
    }
    if let (Some(duck), Some(envelope)) = (&clip.effects.duck, &envelope) {
        effects_report.insert("duck".into(), envelope.to_json(duck));
    }
    let mut chain = (!clip.effects.is_empty()).then(|| {
        Chain::new(
            &clip.effects,
            source.channels.len() == 1,
            envelope.map(|e| e.gains),
        )
    });
    // The one place a clip sample reaches the mix: through the chain when the clip has effects,
    // otherwise exactly the plain gain product.
    let mut emit = |k: usize, l: f32, r: f32, gain: f64| {
        let (l, r) = match &mut chain {
            Some(chain) => chain.apply(k, l, r, gain),
            None => ((f64::from(l) * gain) as f32, (f64::from(r) * gain) as f32),
        };
        left[start + k] += l;
        right[start + k] += r;
        if let Some(key) = key.as_deref_mut() {
            key[k] = l.abs().max(r.abs());
        }
    };
    let report = |media_seconds: f64| ClipReport {
        id: clip.id.clone(),
        src: clip.src.clone(),
        source_rate: source.rate,
        source_channels: source.channels.len(),
        source_seconds: source.frames() as f64 / rate,
        start_frame: start,
        frames: length,
        media_seconds,
        speed: clip.speed.as_ref().map(|curve| curve.to_json()),
        effects: (!effects_report.is_empty()).then(|| effects_report.clone().into()),
    };
    if let Some(curve) = &clip.speed {
        let varispeed = varispeed.expect("the mixer builds a varispeed reader for speed clips");
        // Without a loop, the kernel never reads before the trim or past the source end.
        let edge = if clip.looped {
            Edge::Wrap {
                start: loop_start,
                length: loop_span,
            }
        } else {
            Edge::Window {
                start: trim,
                end: source.frames(),
            }
        };
        for k in 0..length {
            let t = k as f64 / rate;
            let media = curve.media(t) * rate;
            if !clip.looped && media >= available as f64 {
                break;
            }
            let speed = curve.speed(t);
            let gain = fade(k) * (speed / SILENT_BELOW).min(1.0);
            let (l, r) = varispeed.sample(
                &source.channels[0],
                right_plane,
                wrap(trim as f64 + media),
                speed,
                edge,
            );
            emit(k, l, r, gain);
        }
        let end = if length == 0 {
            0.0
        } else {
            curve.media(length as f64 / rate).min(if clip.looped {
                f64::INFINITY
            } else {
                available as f64 / rate
            })
        };
        return Ok(report(end));
    }
    for k in 0..length {
        let index = if clip.looped {
            wrap((trim + k) as f64) as usize
        } else if k < available {
            trim + k
        } else {
            break;
        };
        emit(k, source.channels[0][index], right_plane[index], fade(k));
    }
    let played = if clip.looped {
        length
    } else {
        length.min(available)
    };
    Ok(report(played as f64 / rate))
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
            scene_end: None,
            trim: 0.0,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            looped: false,
            loop_from: 0.0,
            speed: None,
            effects: Default::default(),
        }
    }

    fn run(clip: &AudioClip, source: &Source, total: usize) -> (Vec<f32>, ClipReport) {
        let (mut l, mut r) = (vec![0.0; total], vec![0.0; total]);
        let report = add_clip(clip, source, Some(&Varispeed::new()), &mut l, &mut r).unwrap();
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
        c.loop_from = c.trim;
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
        assert!(add_clip(&c, &source(vec![1.0; 100]), None, &mut l, &mut r).is_err());
    }

    fn ramp(keys: &[(f64, f64)]) -> Option<std::sync::Arc<crate::speed::SpeedCurve>> {
        let keys = keys
            .iter()
            .map(|&(at, speed)| crate::speed::SpeedKey {
                at,
                speed,
                easing: crate::model::Easing::Linear,
            })
            .collect();
        Some(std::sync::Arc::new(
            crate::speed::SpeedCurve::ramp(keys).unwrap(),
        ))
    }

    #[test]
    fn half_speed_doubles_the_length_and_reads_every_half_sample() {
        let samples: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut c = clip();
        c.speed = Some(std::sync::Arc::new(
            crate::speed::SpeedCurve::constant(0.5).unwrap(),
        ));
        let (out, report) = run(&c, &source(samples.clone()), 48_000);
        assert_eq!(report.frames, 9600);
        assert!((report.media_seconds - 0.1).abs() < 1e-9);
        // Even outputs land on source samples; odd ones interpolate between neighbors.
        for k in (200..9000).step_by(2) {
            assert!((out[k] - samples[k / 2]).abs() < 1e-4, "sample {k}");
        }
        let mid = (samples[1000] + samples[1001]) / 2.0;
        assert!((out[2001] - mid).abs() < 1e-3);
    }

    #[test]
    fn a_ramp_to_zero_needs_a_duration_and_ends_silent() {
        let mut c = clip();
        c.speed = ramp(&[(0.0, 1.0), (0.05, 0.0)]);
        let (mut l, mut r) = (vec![0.0; 48_000], vec![0.0; 48_000]);
        let error = add_clip(
            &c,
            &source(vec![0.5; 48_000]),
            Some(&Varispeed::new()),
            &mut l,
            &mut r,
        )
        .unwrap_err();
        assert!(error.to_string().contains("data-duration"));
        c.duration = Some(0.1);
        let (out, report) = run(&c, &source(vec![0.5; 48_000]), 48_000);
        assert!((report.media_seconds - 0.025).abs() < 1e-9);
        assert!((out[100] - 0.5).abs() < 1e-4);
        assert_eq!(out[3000], 0.0, "frozen sound is silent");
    }

    #[test]
    fn a_video_sound_loop_restarts_at_the_source_start_like_the_picture() {
        let mut c = clip();
        c.looped = true;
        c.trim = 2.0 / 48_000.0;
        c.loop_from = 0.0;
        c.duration = Some(6.0 / 48_000.0);
        let (out, _) = run(&c, &source(vec![9.0, 1.0, 2.0, 3.0]), 48_000);
        assert_eq!(&out[..7], &[2.0, 3.0, 9.0, 1.0, 2.0, 3.0, 0.0]);
    }

    #[test]
    fn speed_reads_never_reach_before_the_trim() {
        // An impulse right before the trim must not leak into the interpolated output.
        let mut samples = vec![0.0_f32; 4800];
        samples[2399] = 1.0;
        let mut c = clip();
        c.trim = 0.05;
        c.duration = Some(0.01);
        c.speed = Some(std::sync::Arc::new(
            crate::speed::SpeedCurve::constant(0.5).unwrap(),
        ));
        let (out, _) = run(&c, &source(samples), 48_000);
        assert!(out.iter().all(|s| *s == 0.0), "leaked {:?}", &out[..8]);
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
