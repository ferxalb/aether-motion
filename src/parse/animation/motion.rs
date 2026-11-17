//! Compiles resolved animations and `data-hf-keyframes` into time-sampled motion tracks.
use crate::model::{
    Easing, FilterChannel, FilterMotion, FxMotion, Keyframe, Motion, Pose, Track, TrackPoint,
};
use crate::parse::animation::{AnimationSpec, CssFrame, Direction, Iterations, MAX_TRACK_POINTS};
use anyhow::{bail, Result};

/// Expands iterations, direction and fill into per-channel tracks for a clip of `clip_len` seconds.
pub(in crate::parse) fn build_motion(
    spec: &AnimationSpec,
    base: Pose,
    filter_base: [f32; 8],
    clip_len: f64,
) -> Result<Motion> {
    let decl = &spec.decl;
    let count = match decl.iterations {
        Iterations::Count(n) => n as usize,
        Iterations::Infinite => ((clip_len - decl.delay) / spec.duration).ceil().max(1.0) as usize,
    };
    let channel = |get: &dyn Fn(&CssFrame) -> Option<f32>, rest: f32| -> Result<Track> {
        let mut sparse: Vec<(f64, f32, Easing)> = spec
            .frames
            .iter()
            .filter_map(|f| get(f).map(|v| (f.offset, v, f.easing.unwrap_or(decl.easing))))
            .collect();
        if sparse.is_empty() {
            return Ok(Track::default());
        }
        if sparse[0].0 > 0.0 {
            sparse.insert(0, (0.0, rest, decl.easing));
        }
        if sparse.last().is_some_and(|last| last.0 < 1.0) {
            sparse.push((1.0, rest, decl.easing));
        }
        // The easing declared at a keyframe shapes the interval that starts there.
        let forward: Vec<(f64, f32, Easing)> = sparse
            .iter()
            .enumerate()
            .map(|(k, &(offset, value, _))| {
                let arriving = if k == 0 {
                    Easing::Linear
                } else {
                    sparse[k - 1].2
                };
                (offset, value, arriving)
            })
            .collect();
        let n = forward.len();
        if count.saturating_mul(n) > MAX_TRACK_POINTS {
            bail!(
                "animation {}: generates too many points; raise animation-duration or lower the iteration count",
                spec.name
            );
        }
        let mut points: Vec<TrackPoint> = Vec::with_capacity(count * n);
        for i in 0..count {
            let reverse = match decl.direction {
                Direction::Normal => false,
                Direction::Reverse => true,
                Direction::Alternate => i % 2 == 1,
                Direction::AlternateReverse => i % 2 == 0,
            };
            for j in 0..n {
                let (offset, value, easing) = if reverse {
                    let (offset, value, _) = forward[n - 1 - j];
                    let easing = if j == 0 {
                        Easing::Linear
                    } else {
                        forward[n - j].2.reversed()
                    };
                    (1.0 - offset, value, easing)
                } else {
                    forward[j]
                };
                let at = decl.delay + (i as f64 + offset) * spec.duration;
                // Two points may share `at` at an iteration boundary: the later one wins from there on.
                points.push(TrackPoint { at, value, easing });
            }
        }
        if points.windows(2).any(|w| w[1].at < w[0].at) {
            bail!("animation {}: keyframes are too close together", spec.name);
        }
        Ok(Track(points))
    };
    // A function missing from a keyframe's `filter` list takes its neutral amount there; keyframes
    // without `filter` do not constrain the filter channels.
    let mut tracks: [Track; 8] = Default::default();
    for kind in FilterChannel::ALL {
        let i = kind.index();
        if !spec
            .frames
            .iter()
            .any(|f| f.filter.is_some_and(|values| values[i].is_some()))
        {
            continue;
        }
        tracks[i] = channel(
            &|f| f.filter.map(|values| values[i].unwrap_or(kind.neutral())),
            filter_base[i],
        )?;
    }
    Ok(Motion {
        base,
        x: channel(&|f| f.transform.map(|t| t.0), base.x)?,
        y: channel(&|f| f.transform.map(|t| t.1), base.y)?,
        scale: channel(&|f| f.transform.map(|t| t.2), base.scale)?,
        rotate: channel(&|f| f.transform.map(|t| t.3), base.rotate)?,
        opacity: channel(&|f| f.opacity, base.opacity)?,
        reveal: Track::default(),
        wipe: None,
        filter: FilterMotion {
            base: filter_base,
            tracks,
            appended: Vec::new(),
        },
        fx: FxMotion::default(),
        hold_before: decl.backwards,
        hold_after: decl.forwards,
    })
}

/// `data-hf-keyframes` filter amounts: a channel named in any keyframe gets a point at every
/// keyframe, inheriting the previous keyframe's amount (the static amount before the first one).
fn filter_from_keyframes(filter_base: [f32; 8], frames: &[Keyframe]) -> FilterMotion {
    let tracks = FilterChannel::ALL.map(|channel| {
        let i = channel.index();
        if frames.iter().all(|k| k.filters[i].is_none()) {
            return Track::default();
        }
        let mut value = filter_base[i];
        Track(
            frames
                .iter()
                .map(|k| {
                    value = k.filters[i].unwrap_or(value);
                    TrackPoint {
                        at: k.at,
                        value,
                        easing: k.easing,
                    }
                })
                .collect(),
        )
    });
    FilterMotion {
        base: filter_base,
        tracks,
        appended: Vec::new(),
    }
}

/// `data-hf-keyframes` `glitch` and `rgbSplit`: like filter amounts, with a resting value of 0.
fn fx_from_keyframes(frames: &[Keyframe]) -> FxMotion {
    let track = |i: usize| {
        if frames.iter().all(|k| k.effects[i].is_none()) {
            return Track::default();
        }
        let mut value = 0.0;
        Track(
            frames
                .iter()
                .map(|k| {
                    value = k.effects[i].unwrap_or(value);
                    TrackPoint {
                        at: k.at,
                        value,
                        easing: k.easing,
                    }
                })
                .collect(),
        )
    };
    FxMotion {
        glitch: track(0),
        rgb_split: track(1),
        ..FxMotion::default()
    }
}

/// `data-hf-keyframes` offsets add to the resting pose; opacity multiplies it; filter amounts are
/// absolute.
pub(in crate::parse) fn motion_from_keyframes(
    base: Pose,
    filter_base: [f32; 8],
    frames: &[Keyframe],
) -> Motion {
    let track = |value: &dyn Fn(&Keyframe) -> f32| {
        Track(
            frames
                .iter()
                .map(|k| TrackPoint {
                    at: k.at,
                    value: value(k),
                    easing: k.easing,
                })
                .collect(),
        )
    };
    Motion {
        base,
        x: track(&|k| base.x + k.x),
        y: track(&|k| base.y + k.y),
        scale: Track::default(),
        rotate: Track::default(),
        opacity: track(&|k| base.opacity * k.opacity),
        reveal: Track::default(),
        wipe: None,
        filter: filter_from_keyframes(filter_base, frames),
        fx: fx_from_keyframes(frames),
        hold_before: true,
        hold_after: true,
    }
}
