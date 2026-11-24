//! Rig motion: the built-in motion presets (looping cycles of anatomical key poses), the `timeline`
//! keyframe parameter, and sampling either into poses at given times.
//!
//! A preset is a set of periodic curves per channel through key poses at phases 0..1, interpolated
//! with periodic cubic Hermite splines (Catmull-Rom tangents), so velocity is continuous around the
//! loop. A timeline is a list of absolute poses at times in seconds; every key holds every channel
//! (omitted channels keep the previous key's value), and the easing of a key shapes the segment that
//! ends there. Both are pure functions of time.

use super::skeleton::*;
use anyhow::{anyhow, bail, Context, Result};

/// The built-in motions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    None,
    Idle,
    Walk,
    Run,
    Wave,
    Cheer,
    Jump,
    Sit,
    Point,
}

pub const MOTION_NAMES: [&str; 9] = [
    "none", "idle", "walk", "run", "wave", "cheer", "jump", "sit", "point",
];

impl Motion {
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "none" => Self::None,
            "idle" => Self::Idle,
            "walk" => Self::Walk,
            "run" => Self::Run,
            "wave" => Self::Wave,
            "cheer" => Self::Cheer,
            "jump" => Self::Jump,
            "sit" => Self::Sit,
            "point" => Self::Point,
            other => bail!("unknown motion '{other}'; use {}", MOTION_NAMES.join(", ")),
        })
    }

    /// Whether the feet push the body along (so `travel=auto` has a speed to match).
    pub fn travels(self) -> bool {
        matches!(self, Self::Walk | Self::Run)
    }

    /// For a gait, the phase of the right foot's mid-stance: the right sole is flat and planted
    /// and the left foot is high in its swing, the moment to stop on.
    fn stop_phase(self) -> Option<f64> {
        match self {
            Self::Walk => Some(0.3),
            Self::Run => Some(0.14),
            _ => None,
        }
    }
}

/// Which projection a preset is shaped for: channels a view cannot show are replaced by ones it can
/// (a side view raises a waving arm forward instead of sideways).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Side,
    Front,
    Space,
}

/// A looping preset: a period and periodic key curves per channel (absolute values).
pub struct Preset {
    pub cycle: f64,
    tracks: Vec<(usize, Vec<(f64, f64)>)>,
    /// Ballistic lifts `(channel, start, end, height)`: a parabola between the two phases and
    /// exactly zero outside them, added to the channel (negative is up).
    flights: Vec<(usize, f64, f64, f64)>,
}

impl Preset {
    fn new(cycle: f64) -> Self {
        Self {
            cycle,
            tracks: Vec::new(),
            flights: Vec::new(),
        }
    }

    /// Sets channel `ch` to the periodic curve through `(phase, value)` keys.
    fn set(&mut self, ch: usize, keys: &[(f64, f64)]) {
        self.tracks.retain(|(c, _)| *c != ch);
        // Sorted once here, so `periodic` can read the keys in place on every evaluation.
        let mut sorted = keys.to_vec();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.tracks.push((ch, sorted));
    }

    fn hold(&mut self, ch: usize, value: f64) {
        self.set(ch, &[(0.0, value)]);
    }

    /// Sets the right channel to `keys` and the left one to the same curve shifted half a cycle.
    fn gait(&mut self, right: usize, left: usize, keys: &[(f64, f64)]) {
        self.set(right, keys);
        let shifted: Vec<(f64, f64)> = keys.iter().map(|(p, v)| ((p + 0.5) % 1.0, *v)).collect();
        self.set(left, &shifted);
    }

    /// Sets both sides to the same curve.
    fn both(&mut self, right: usize, left: usize, keys: &[(f64, f64)]) {
        self.set(right, keys);
        self.set(left, keys);
    }

    /// The pose at `phase` (any real; wrapped into 0..1).
    pub fn pose(&self, phase: f64) -> Pose {
        let mut pose = rest();
        let p = phase.rem_euclid(1.0);
        for (ch, keys) in &self.tracks {
            pose[*ch] = periodic(keys, p);
        }
        for (ch, start, end, height) in &self.flights {
            if p > *start && p < *end {
                let u = (p - start) / (end - start);
                pose[*ch] -= height * 4.0 * u * (1.0 - u);
            }
        }
        pose
    }
}

/// A periodic cubic Hermite curve through `keys` (phases in 0..1, sorted by phase), with
/// Catmull-Rom tangents over non-uniform spacing.
fn periodic(keys: &[(f64, f64)], p: f64) -> f64 {
    if keys.len() == 1 {
        return keys[0].1;
    }
    debug_assert!(
        keys.windows(2).all(|w| w[0].0 <= w[1].0),
        "keys must be sorted"
    );
    let k = keys;
    let n = k.len();
    // Key i, unrolled around the loop so neighbors exist on both sides.
    let at = |i: isize| -> (f64, f64) {
        let wrap = i.rem_euclid(n as isize) as usize;
        let laps = i.div_euclid(n as isize) as f64;
        (k[wrap].0 + laps, k[wrap].1)
    };
    let mut i = (n as isize) - 1;
    for (j, key) in k.iter().enumerate() {
        if key.0 <= p {
            i = j as isize;
        }
    }
    if p < k[0].0 {
        i = -1;
    }
    let (p0, v0) = at(i);
    let (p1, v1) = at(i + 1);
    let tangent = |j: isize| -> f64 {
        let (pa, va) = at(j - 1);
        let (pb, vb) = at(j + 1);
        (vb - va) / (pb - pa)
    };
    hermite(p0, v0, tangent(i), p1, v1, tangent(i + 1), p)
}

fn hermite(p0: f64, v0: f64, m0: f64, p1: f64, v1: f64, m1: f64, p: f64) -> f64 {
    let h = p1 - p0;
    if h <= 0.0 {
        return v1;
    }
    let s = ((p - p0) / h).clamp(0.0, 1.0);
    let (s2, s3) = (s * s, s * s * s);
    (2.0 * s3 - 3.0 * s2 + 1.0) * v0
        + (s3 - 2.0 * s2 + s) * h * m0
        + (-2.0 * s3 + 3.0 * s2) * v1
        + (s3 - s2) * h * m1
}

/// The preset of `motion` shaped for `view`; `None` for `Motion::None`.
pub fn preset(motion: Motion, view: View) -> Option<Preset> {
    let side_view = view == View::Side;
    Some(match motion {
        Motion::None => return None,
        Motion::Idle => {
            let mut p = Preset::new(3.6);
            p.set(TORSO, &[(0.0, 0.0), (0.5, 1.6)]);
            p.set(TORSO_SIDE, &[(0.25, -0.6), (0.75, 0.6)]);
            p.set(HEAD, &[(0.1, -1.0), (0.6, 1.2)]);
            p.set(HEAD_SIDE, &[(0.3, -1.0), (0.8, 1.0)]);
            p.both(RIGHT.shoulder, LEFT.shoulder, &[(0.0, 0.0), (0.5, 2.5)]);
            p.both(
                RIGHT.shoulder_side,
                LEFT.shoulder_side,
                &[(0.0, 5.0), (0.5, 7.0)],
            );
            p.both(RIGHT.elbow, LEFT.elbow, &[(0.0, 8.0), (0.5, 12.0)]);
            p.both(RIGHT.knee, LEFT.knee, &[(0.0, 2.0), (0.5, 3.0)]);
            p
        }
        Motion::Walk => {
            let mut p = Preset::new(1.1);
            // Phase 0 is the right heel strike.
            p.gait(
                RIGHT.hip,
                LEFT.hip,
                &[
                    (0.0, 24.0),
                    (0.12, 20.0),
                    (0.3, 6.0),
                    (0.5, -10.0),
                    (0.6, -6.0),
                    (0.75, 14.0),
                    (0.88, 26.0),
                ],
            );
            p.gait(
                RIGHT.knee,
                LEFT.knee,
                &[
                    (0.0, 4.0),
                    (0.12, 16.0),
                    (0.35, 6.0),
                    (0.5, 10.0),
                    (0.62, 40.0),
                    (0.72, 60.0),
                    (0.85, 28.0),
                    (0.95, 6.0),
                ],
            );
            p.gait(
                RIGHT.ankle,
                LEFT.ankle,
                &[
                    (0.0, 2.0),
                    (0.07, -6.0),
                    (0.3, 6.0),
                    (0.5, 8.0),
                    (0.6, -14.0),
                    (0.68, -4.0),
                    (0.8, 4.0),
                    (0.95, 2.0),
                ],
            );
            // Each arm swings against the leg on its side.
            p.gait(
                RIGHT.shoulder,
                LEFT.shoulder,
                &[(0.0, -18.0), (0.25, 0.0), (0.5, 20.0), (0.75, 0.0)],
            );
            p.gait(
                RIGHT.elbow,
                LEFT.elbow,
                &[(0.0, 10.0), (0.25, 14.0), (0.5, 26.0), (0.75, 14.0)],
            );
            p.both(RIGHT.shoulder_side, LEFT.shoulder_side, &[(0.0, 6.0)]);
            p.hold(TORSO, 4.0);
            p.hold(HEAD, -4.0);
            p.set(TORSO_TWIST, &[(0.0, 4.0), (0.5, -4.0)]);
            p.set(HEAD_TURN, &[(0.0, -3.0), (0.5, 3.0)]);
            p
        }
        Motion::Run => {
            let mut p = Preset::new(0.72);
            p.gait(
                RIGHT.hip,
                LEFT.hip,
                &[
                    (0.0, 38.0),
                    (0.15, 22.0),
                    (0.3, -4.0),
                    (0.45, -18.0),
                    (0.6, 10.0),
                    (0.75, 50.0),
                    (0.9, 52.0),
                ],
            );
            p.gait(
                RIGHT.knee,
                LEFT.knee,
                &[
                    (0.0, 18.0),
                    (0.12, 34.0),
                    (0.3, 20.0),
                    (0.45, 40.0),
                    (0.6, 100.0),
                    (0.75, 110.0),
                    (0.9, 48.0),
                ],
            );
            p.gait(
                RIGHT.ankle,
                LEFT.ankle,
                &[
                    (0.0, 4.0),
                    (0.1, -4.0),
                    (0.28, -20.0),
                    (0.45, -28.0),
                    (0.6, 0.0),
                    (0.8, 8.0),
                    (0.92, 8.0),
                ],
            );
            p.gait(
                RIGHT.shoulder,
                LEFT.shoulder,
                &[(0.0, -40.0), (0.25, 2.0), (0.5, 45.0), (0.75, 2.0)],
            );
            p.gait(RIGHT.elbow, LEFT.elbow, &[(0.0, 80.0), (0.5, 96.0)]);
            p.both(RIGHT.shoulder_side, LEFT.shoulder_side, &[(0.0, 8.0)]);
            p.hold(TORSO, 12.0);
            p.hold(HEAD, -9.0);
            p.set(TORSO_TWIST, &[(0.0, 7.0), (0.5, -7.0)]);
            // Flight: the body rises between the push-off of one foot and the strike of the other.
            p.set(
                BODY_Y,
                &[
                    (0.0, 0.0),
                    (0.12, 3.0),
                    (0.3, -14.0),
                    (0.5, 0.0),
                    (0.62, 3.0),
                    (0.8, -14.0),
                ],
            );
            p
        }
        Motion::Wave => {
            let mut p = Preset::new(1.0);
            if side_view {
                p.hold(RIGHT.shoulder, 150.0);
                p.set(RIGHT.elbow, &[(0.0, 20.0), (0.5, 62.0)]);
                p.set(RIGHT.wrist, &[(0.0, 10.0), (0.5, -15.0)]);
            } else {
                p.hold(RIGHT.shoulder_side, 135.0);
                p.hold(RIGHT.shoulder, 12.0);
                p.set(RIGHT.elbow, &[(0.0, 20.0), (0.5, 58.0)]);
                p.set(RIGHT.wrist, &[(0.0, -12.0), (0.5, 14.0)]);
            }
            p.set(HEAD_SIDE, &[(0.0, 3.0), (0.5, 6.0)]);
            p.set(TORSO_SIDE, &[(0.0, -2.0), (0.5, -3.0)]);
            p.set(HEAD, &[(0.0, -3.0), (0.5, -1.0)]);
            p.hold(LEFT.shoulder_side, 6.0);
            p.hold(LEFT.elbow, 10.0);
            p
        }
        Motion::Cheer => {
            let mut p = Preset::new(0.9);
            if side_view {
                p.both(RIGHT.shoulder, LEFT.shoulder, &[(0.0, 165.0), (0.5, 150.0)]);
            } else {
                p.both(
                    RIGHT.shoulder_side,
                    LEFT.shoulder_side,
                    &[(0.0, 160.0), (0.5, 145.0)],
                );
                p.both(RIGHT.shoulder, LEFT.shoulder, &[(0.0, 10.0)]);
            }
            p.both(RIGHT.elbow, LEFT.elbow, &[(0.0, 10.0), (0.5, 34.0)]);
            p.both(RIGHT.hip, LEFT.hip, &[(0.0, 2.0), (0.5, 16.0)]);
            p.both(RIGHT.knee, LEFT.knee, &[(0.0, 4.0), (0.5, 32.0)]);
            p.both(RIGHT.ankle, LEFT.ankle, &[(0.0, -2.0), (0.5, 16.0)]);
            p.set(TORSO, &[(0.0, -4.0), (0.5, 4.0)]);
            p.set(HEAD, &[(0.0, -8.0), (0.5, -2.0)]);
            if !side_view {
                // A front view cannot show bent knees, so the bounce is a lift.
                p.set(BODY_Y, &[(0.0, -10.0), (0.5, 4.0)]);
            }
            p
        }
        Motion::Jump => {
            let mut p = Preset::new(1.8);
            p.both(
                RIGHT.hip,
                LEFT.hip,
                &[
                    (0.0, 0.0),
                    (0.08, 4.0),
                    (0.17, 72.0),
                    (0.3, -4.0),
                    (0.45, 46.0),
                    (0.58, 8.0),
                    (0.7, 66.0),
                    (0.86, 4.0),
                    (0.94, 0.0),
                ],
            );
            p.both(
                RIGHT.knee,
                LEFT.knee,
                &[
                    (0.0, 0.0),
                    (0.08, 6.0),
                    (0.17, 108.0),
                    (0.3, 0.0),
                    (0.45, 80.0),
                    (0.58, 10.0),
                    (0.7, 100.0),
                    (0.86, 4.0),
                    (0.94, 0.0),
                ],
            );
            p.both(
                RIGHT.ankle,
                LEFT.ankle,
                &[
                    (0.0, 0.0),
                    (0.08, 2.0),
                    (0.17, 34.0),
                    (0.3, -36.0),
                    (0.45, 0.0),
                    (0.58, -16.0),
                    (0.7, 30.0),
                    (0.86, 0.0),
                ],
            );
            p.set(
                TORSO,
                &[
                    (0.0, 0.0),
                    (0.17, 36.0),
                    (0.3, -4.0),
                    (0.45, 12.0),
                    (0.7, 32.0),
                    (0.86, 2.0),
                ],
            );
            p.set(
                HEAD,
                &[
                    (0.0, 0.0),
                    (0.17, -20.0),
                    (0.3, 0.0),
                    (0.7, -18.0),
                    (0.86, 0.0),
                ],
            );
            let arms = [
                (0.0, 0.0),
                (0.17, -55.0),
                (0.3, 165.0),
                (0.45, 120.0),
                (0.6, 70.0),
                (0.7, -25.0),
                (0.86, 0.0),
            ];
            if side_view {
                p.both(RIGHT.shoulder, LEFT.shoulder, &arms);
            } else {
                p.both(
                    RIGHT.shoulder_side,
                    LEFT.shoulder_side,
                    &[
                        (0.0, 5.0),
                        (0.17, 18.0),
                        (0.3, 155.0),
                        (0.45, 130.0),
                        (0.6, 60.0),
                        (0.7, 20.0),
                        (0.86, 5.0),
                    ],
                );
                if view == View::Space {
                    p.both(
                        RIGHT.shoulder,
                        LEFT.shoulder,
                        &[
                            (0.0, 0.0),
                            (0.17, -40.0),
                            (0.3, 10.0),
                            (0.7, -20.0),
                            (0.86, 0.0),
                        ],
                    );
                }
            }
            p.both(
                RIGHT.elbow,
                LEFT.elbow,
                &[
                    (0.0, 6.0),
                    (0.3, 8.0),
                    (0.45, 34.0),
                    (0.7, 22.0),
                    (0.86, 6.0),
                ],
            );
            // Flight, added after the feet are placed on the ground: zero on the ground, so the
            // feet never sink before takeoff or after landing.
            p.flights.push((BODY_Y, 0.29, 0.6, 185.0));
            p
        }
        Motion::Sit => {
            let mut p = Preset::new(4.0);
            p.both(RIGHT.hip, LEFT.hip, &[(0.0, 90.0)]);
            p.both(RIGHT.knee, LEFT.knee, &[(0.0, 90.0)]);
            p.both(RIGHT.hip_side, LEFT.hip_side, &[(0.0, 4.0)]);
            p.both(RIGHT.shoulder, LEFT.shoulder, &[(0.0, 22.0), (0.5, 24.0)]);
            p.both(RIGHT.elbow, LEFT.elbow, &[(0.0, 52.0), (0.5, 55.0)]);
            p.both(RIGHT.shoulder_side, LEFT.shoulder_side, &[(0.0, 6.0)]);
            p.set(TORSO, &[(0.0, -2.0), (0.5, 0.0)]);
            p.set(HEAD, &[(0.0, 2.0), (0.5, 0.0)]);
            p
        }
        Motion::Point => {
            let mut p = Preset::new(3.0);
            if side_view {
                p.set(RIGHT.shoulder, &[(0.0, 88.0), (0.5, 92.0)]);
            } else {
                p.set(RIGHT.shoulder_side, &[(0.0, 88.0), (0.5, 92.0)]);
                if view == View::Space {
                    p.hold(RIGHT.shoulder, 30.0);
                }
            }
            p.hold(RIGHT.elbow, 2.0);
            p.hold(RIGHT.wrist, -4.0);
            p.hold(LEFT.shoulder_side, 6.0);
            p.hold(LEFT.elbow, 12.0);
            p.set(HEAD, &[(0.0, -2.0), (0.5, 0.0)]);
            p.set(HEAD_SIDE, &[(0.0, -3.0)]);
            p.set(TORSO, &[(0.0, -1.0), (0.5, 0.5)]);
            p
        }
    })
}

/// Easing of a timeline segment (named like `data-hf-keyframes` easings).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ease {
    Linear,
    In,
    Out,
    InOut,
    InCubic,
    OutCubic,
    InOutCubic,
    Smooth,
}

impl Ease {
    fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "linear" => Self::Linear,
            "ease-in" => Self::In,
            "ease-out" => Self::Out,
            "ease-in-out" => Self::InOut,
            "ease-in-cubic" => Self::InCubic,
            "ease-out-cubic" => Self::OutCubic,
            "ease-in-out-cubic" => Self::InOutCubic,
            "smooth" => Self::Smooth,
            other => bail!("unknown easing '{other}'; use linear, ease-in, ease-out, ease-in-out, ease-in-cubic, ease-out-cubic, ease-in-out-cubic, or smooth"),
        })
    }

    fn apply(self, s: f64) -> f64 {
        match self {
            Self::Linear | Self::Smooth => s,
            Self::In => s * s,
            Self::Out => 1.0 - (1.0 - s) * (1.0 - s),
            Self::InOut => s * s * (3.0 - 2.0 * s),
            Self::InCubic => s * s * s,
            Self::OutCubic => 1.0 - (1.0 - s).powi(3),
            Self::InOutCubic => {
                if s < 0.5 {
                    4.0 * s * s * s
                } else {
                    1.0 - (-2.0 * s + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// A parsed `timeline`: absolute poses at increasing times.
pub struct Timeline {
    times: Vec<f64>,
    poses: Vec<Pose>,
    eases: Vec<Ease>,
}

impl Timeline {
    /// Parses `[{"at": 0, "r-elbow": 90, "easing": "smooth"}, ...]`.
    pub fn parse(raw: &str) -> Result<Self> {
        let value: serde_json::Value =
            serde_json::from_str(raw).context("timeline must be a JSON array of keyframes")?;
        let list = value
            .as_array()
            .ok_or_else(|| anyhow!("timeline must be a JSON array of keyframes"))?;
        if list.is_empty() {
            bail!("timeline needs at least one keyframe");
        }
        if list.len() > 512 {
            bail!("timeline takes at most 512 keyframes");
        }
        let mut out = Self {
            times: Vec::new(),
            poses: Vec::new(),
            eases: Vec::new(),
        };
        let mut pose = rest();
        for (index, entry) in list.iter().enumerate() {
            let object = entry
                .as_object()
                .ok_or_else(|| anyhow!("timeline[{index}] must be an object"))?;
            let at = object
                .get("at")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| anyhow!("timeline[{index}] needs a numeric at (seconds)"))?;
            if !(0.0..=MAX_SPAN).contains(&at) {
                bail!("timeline[{index}].at must be between 0 and {MAX_SPAN} seconds");
            }
            if out.times.last().is_some_and(|last| at < last + MIN_STEP) {
                bail!("timeline times must increase by at least {MIN_STEP} s (timeline[{index}].at = {at})");
            }
            // The motion also samples t = 0, so a first key must be at 0 or a millisecond after.
            if out.times.is_empty() && at > 0.0 && at < MIN_STEP {
                bail!("timeline[{index}].at must be 0 or at least {MIN_STEP} s");
            }
            let mut ease = Ease::Linear;
            for (key, value) in object {
                match key.as_str() {
                    "at" => {}
                    "easing" => {
                        ease =
                            Ease::parse(value.as_str().ok_or_else(|| {
                                anyhow!("timeline[{index}].easing must be a string")
                            })?)?
                    }
                    name => {
                        let ch = channel(name).ok_or_else(|| {
                            anyhow!(
                                "timeline[{index}]: unknown channel '{name}'; use at, easing, {}",
                                CHANNELS.join(", ")
                            )
                        })?;
                        let v = value
                            .as_f64()
                            .ok_or_else(|| anyhow!("timeline[{index}].{name} must be a number"))?;
                        check_channel(ch, v).with_context(|| format!("timeline[{index}]"))?;
                        pose[ch] = v;
                    }
                }
            }
            out.times.push(at);
            out.poses.push(pose);
            out.eases.push(ease);
        }
        Ok(out)
    }

    /// Time of the last key (the length of one pass).
    pub fn span(&self) -> f64 {
        *self.times.last().unwrap_or(&0.0)
    }

    /// The key times.
    pub fn times(&self) -> &[f64] {
        &self.times
    }

    /// The pose at `t` seconds; the first and last poses hold outside the keys.
    pub fn pose(&self, t: f64) -> Pose {
        let n = self.times.len();
        if t <= self.times[0] {
            return self.poses[0];
        }
        if t >= self.times[n - 1] {
            return self.poses[n - 1];
        }
        let i = self.times.iter().rposition(|k| *k <= t).unwrap_or(0);
        let (t0, t1) = (self.times[i], self.times[i + 1]);
        let ease = self.eases[i + 1];
        let mut pose = [0.0; CHANNELS.len()];
        for (ch, value) in pose.iter_mut().enumerate() {
            let (v0, v1) = (self.poses[i][ch], self.poses[i + 1][ch]);
            *value = if ease == Ease::Smooth {
                // Catmull-Rom tangents through the neighbors, at rest at the ends.
                let tangent = |j: usize| -> f64 {
                    if j == 0 || j + 1 >= n {
                        0.0
                    } else {
                        (self.poses[j + 1][ch] - self.poses[j - 1][ch])
                            / (self.times[j + 1] - self.times[j - 1])
                    }
                };
                hermite(t0, v0, tangent(i), t1, v1, tangent(i + 1), t)
            } else {
                v0 + (v1 - v0) * ease.apply((t - t0) / (t1 - t0))
            };
        }
        pose
    }
}

/// Range check of one channel value (px for body offsets, degrees otherwise).
pub fn check_channel(ch: usize, value: f64) -> Result<()> {
    let limit = if ch == BODY_X || ch == BODY_Y {
        100_000.0
    } else {
        3600.0
    };
    if !value.is_finite() || value.abs() > limit {
        bail!("{} must be a number within ±{limit}", CHANNELS[ch]);
    }
    Ok(())
}

/// One step of a `sequence`: a motion held for `duration` seconds, blended in from the previous
/// step over `blend` seconds.
pub struct Step {
    motion: Motion,
    preset: Option<Preset>,
    cycle: f64,
    phase: f64,
    duration: f64,
    blend: f64,
    /// Static offsets of this step, added to its motion.
    offsets: Vec<(usize, f64)>,
}

/// A chain of motions: `[{"motion":"walk","duration":2.2},{"motion":"wave","duration":3}]`.
pub struct Sequence {
    steps: Vec<Step>,
    starts: Vec<f64>,
}

impl Sequence {
    /// Parses the `sequence` parameter; presets are shaped for `view`.
    pub fn parse(raw: &str, view: View) -> Result<Self> {
        let value: serde_json::Value =
            serde_json::from_str(raw).context("sequence must be a JSON array of steps")?;
        let list = value
            .as_array()
            .ok_or_else(|| anyhow!("sequence must be a JSON array of steps"))?;
        if list.is_empty() || list.len() > 64 {
            bail!("sequence takes 1 to 64 steps");
        }
        let mut steps: Vec<(Step, bool)> = Vec::new();
        let mut starts = Vec::new();
        let mut start = 0.0;
        for (index, entry) in list.iter().enumerate() {
            let object = entry
                .as_object()
                .ok_or_else(|| anyhow!("sequence[{index}] must be an object"))?;
            let number = |key: &str| -> Result<Option<f64>> {
                object
                    .get(key)
                    .map(|v| {
                        v.as_f64()
                            .filter(|n| n.is_finite())
                            .ok_or_else(|| anyhow!("sequence[{index}].{key} must be a number"))
                    })
                    .transpose()
            };
            let motion = Motion::parse(
                object
                    .get("motion")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| anyhow!("sequence[{index}] needs a motion name"))?,
            )
            .with_context(|| format!("sequence[{index}]"))?;
            let duration = number("duration")?
                .ok_or_else(|| anyhow!("sequence[{index}] needs a duration in seconds"))?;
            if !(0.01..=MAX_SPAN).contains(&duration) {
                bail!("sequence[{index}].duration must be between 0.01 and {MAX_SPAN} seconds");
            }
            let blend = number("blend")?.unwrap_or(if index == 0 {
                0.0
            } else {
                0.4f64.min(duration)
            });
            if blend < 0.0 || blend > duration || (blend > 0.0 && blend < 0.01) {
                bail!(
                    "sequence[{index}].blend must be 0, or between 0.01 s and the step's duration"
                );
            }
            let phase = number("phase")?;
            if phase.is_some_and(|p| !(0.0..=1.0).contains(&p)) {
                bail!("sequence[{index}].phase must be between 0 and 1");
            }
            let preset = preset(motion, view);
            let cycle = match number("cycle")? {
                Some(c) if (0.1..=60.0).contains(&c) => c,
                Some(_) => bail!("sequence[{index}].cycle must be between 0.1 and 60 seconds"),
                None => preset.as_ref().map_or(1.0, |p| p.cycle),
            };
            let mut offsets = Vec::new();
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "motion" | "duration" | "blend" | "phase" | "cycle"
                ) {
                    continue;
                }
                let ch = channel(key).ok_or_else(|| {
                    anyhow!(
                        "sequence[{index}]: unknown field '{key}'; use motion, duration, cycle, phase, blend, or a channel ({})",
                        CHANNELS.join(", ")
                    )
                })?;
                let v = value
                    .as_f64()
                    .ok_or_else(|| anyhow!("sequence[{index}].{key} must be a number"))?;
                check_channel(ch, v).with_context(|| format!("sequence[{index}]"))?;
                offsets.push((ch, v));
            }
            starts.push(start);
            start += duration;
            steps.push((
                Step {
                    motion,
                    preset,
                    cycle,
                    phase: phase.unwrap_or(0.0),
                    duration,
                    blend,
                    offsets,
                },
                phase.is_some(),
            ));
        }
        // A gait that hands over to something other than a gait ends on its stop phase (unless
        // its phase is given), so the stop plants a flat foot while the other one is lifted.
        let next_gait: Vec<bool> = steps
            .iter()
            .skip(1)
            .map(|(s, _)| s.motion.travels())
            .chain([true])
            .collect();
        let steps = steps
            .into_iter()
            .zip(next_gait)
            .map(|((mut step, fixed), next_is_gait)| {
                if let (false, false, Some(stop)) = (fixed, next_is_gait, step.motion.stop_phase())
                {
                    step.phase = (stop - step.duration / step.cycle).rem_euclid(1.0);
                }
                step
            })
            .collect();
        if start > MAX_SPAN {
            bail!("sequence lasts {start} s; the limit is {MAX_SPAN} s");
        }
        Ok(Self { steps, starts })
    }

    /// Total length.
    pub fn span(&self) -> f64 {
        self.starts.last().unwrap_or(&0.0) + self.steps.last().map_or(0.0, |s| s.duration)
    }

    fn step_pose(&self, i: usize, local: f64) -> Pose {
        let step = &self.steps[i];
        let mut pose = match &step.preset {
            Some(p) => p.pose(local / step.cycle + step.phase),
            None => rest(),
        };
        for (ch, v) in &step.offsets {
            pose[*ch] += v;
        }
        pose
    }

    /// The pose at `t` (held outside `0..span`).
    pub fn pose(&self, t: f64) -> Pose {
        let t = t.clamp(0.0, self.span());
        let i = self.starts.iter().rposition(|s| *s <= t).unwrap_or(0);
        let local = t - self.starts[i];
        let current = self.step_pose(i, local);
        let blend = self.steps[i].blend;
        if i == 0 || blend <= 0.0 || local >= blend {
            return current;
        }
        // A gait keeps running under a blend into another gait; anything else holds its last pose
        // (a walk that stops plants its feet instead of stepping on in place).
        let before = &self.steps[i - 1];
        let elapsed = t - self.starts[i - 1];
        let held = if before.motion.travels() && self.steps[i].motion.travels() {
            elapsed
        } else {
            elapsed.min(before.duration)
        };
        let previous = self.step_pose(i - 1, held);
        let s = local / blend;
        let w = s * s * (3.0 - 2.0 * s);
        let mut pose = previous;
        for (p, c) in pose.iter_mut().zip(current) {
            *p += (c - *p) * w;
        }
        pose
    }

    /// Step boundaries and blend ends, for sampling.
    fn marks(&self) -> Vec<f64> {
        self.starts
            .iter()
            .zip(&self.steps)
            .flat_map(|(s, step)| [*s, s + step.blend])
            .collect()
    }
}

/// Where poses come from.
pub enum Source {
    /// One pose for the whole clip.
    Still,
    /// A looping preset started at `phase` (0..1).
    Cycle {
        preset: Preset,
        cycle: f64,
        phase: f64,
    },
    /// A timeline, played once or looped.
    Keys { timeline: Timeline, looped: bool },
    /// A sequence of motions, played once or looped.
    Chain { sequence: Sequence, looped: bool },
}

impl Source {
    /// The pose (before offsets and ground placement) at `t` seconds after the motion starts.
    pub fn pose(&self, t: f64) -> Pose {
        match self {
            Self::Still => rest(),
            Self::Cycle {
                preset,
                cycle,
                phase,
            } => preset.pose(t / cycle + phase),
            Self::Keys { timeline, looped } => {
                let span = timeline.span();
                if *looped && span > 0.0 && t > span {
                    timeline.pose(t.rem_euclid(span))
                } else {
                    timeline.pose(t)
                }
            }
            Self::Chain { sequence, looped } => {
                let span = sequence.span();
                if *looped && span > 0.0 && t > span {
                    sequence.pose(t.rem_euclid(span))
                } else {
                    sequence.pose(t)
                }
            }
        }
    }

    /// The length of one pass (`None` for a still pose).
    pub fn period(&self) -> Option<f64> {
        match self {
            Self::Still => None,
            Self::Cycle { cycle, .. } => Some(*cycle),
            Self::Keys { timeline, .. } => (timeline.span() > 0.0).then(|| timeline.span()),
            Self::Chain { sequence, .. } => Some(sequence.span()),
        }
    }

    /// Sample times of one pass, from 0 to the period inclusive: a uniform grid of `per_cycle`
    /// steps for a cycle; for a timeline every key time plus a grid of `per_second` steps.
    pub fn pass_times(&self, per_cycle: usize, per_second: f64) -> Vec<f64> {
        match self {
            Self::Still => vec![0.0],
            Self::Cycle { cycle, .. } => (0..=per_cycle)
                .map(|i| cycle * i as f64 / per_cycle as f64)
                .collect(),
            Self::Keys { timeline, .. } => grid(timeline.span(), per_second, timeline.times()),
            Self::Chain { sequence, .. } => grid(sequence.span(), per_second, &sequence.marks()),
        }
    }

    /// Whether the motion repeats (a cycle, or a looped timeline).
    pub fn repeats(&self) -> bool {
        match self {
            Self::Still => false,
            Self::Cycle { .. } => true,
            Self::Keys { looped, .. } | Self::Chain { looped, .. } => *looped,
        }
    }
}

/// Longest timeline or sequence, in seconds: it bounds every sample grid built from them.
pub const MAX_SPAN: f64 = 600.0;
/// Smallest spacing of authored times (timeline keys): one millisecond, so every key keeps its own
/// keyframe when times and CSS offsets are written out.
pub const MIN_STEP: f64 = 0.001;

/// `marks` (authored times, all kept) plus `0..=span` every `1 / per_second` seconds, leaving out
/// generated times closer than `MIN_STEP` to a mark; sorted, without duplicates.
fn grid(span: f64, per_second: f64, marks: &[f64]) -> Vec<f64> {
    let mut marks: Vec<f64> = marks.iter().map(|m| m.clamp(0.0, span)).collect();
    marks.push(0.0);
    marks.push(span);
    marks.sort_by(f64::total_cmp);
    marks.dedup();
    let steps = (span * per_second).ceil() as usize;
    let mut times = marks.clone();
    for i in 0..=steps {
        let t = (i as f64 / per_second).min(span);
        let near = marks.iter().any(|m| (m - t).abs() < MIN_STEP);
        if !near {
            times.push(t);
        }
    }
    times.sort_by(f64::total_cmp);
    times.dedup();
    times
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_curves_pass_through_keys_and_wrap() {
        let keys = [(0.0, 10.0), (0.25, 0.0), (0.5, -10.0), (0.75, 0.0)];
        for (p, v) in keys {
            assert!((periodic(&keys, p) - v).abs() < 1e-9);
        }
        assert!((periodic(&keys, 1.0) - periodic(&keys, 0.0)).abs() < 1e-9);
        assert!((periodic(&keys, 0.999) - 10.0).abs() < 0.1);
    }

    #[test]
    fn walk_mirrors_the_legs_half_a_cycle_apart() {
        let p = preset(Motion::Walk, View::Side).unwrap();
        for phase in [0.0, 0.13, 0.4, 0.77] {
            let a = p.pose(phase);
            let b = p.pose(phase + 0.5);
            assert!((a[RIGHT.hip] - b[LEFT.hip]).abs() < 1e-9);
            assert!((a[RIGHT.knee] - b[LEFT.knee]).abs() < 1e-9);
        }
    }

    #[test]
    fn timelines_inherit_ease_and_validate() {
        let t = Timeline::parse(
            r#"[{"at":0,"r-elbow":0},{"at":1,"r-elbow":90,"easing":"ease-in-out"},{"at":2,"head":10}]"#,
        )
        .unwrap();
        assert_eq!(t.pose(0.5)[RIGHT.elbow], 45.0);
        assert_eq!(t.pose(2.0)[RIGHT.elbow], 90.0);
        assert_eq!(t.pose(1.5)[HEAD], 5.0);
        assert_eq!(t.pose(9.0)[HEAD], 10.0);
        assert!(Timeline::parse(r#"[{"at":1},{"at":1}]"#).is_err());
        assert!(Timeline::parse(r#"[{"at":0,"elbow":3}]"#).is_err());
        assert!(Timeline::parse(r#"[{"at":0,"r-knee":1e9}]"#).is_err());
        assert!(Timeline::parse("{}").is_err());
    }

    #[test]
    fn sequences_blend_between_motions() {
        let seq = Sequence::parse(
            r#"[{"motion":"none","duration":1},{"motion":"none","duration":1,"blend":0.5,"r-elbow":40}]"#,
            View::Side,
        )
        .unwrap();
        assert_eq!(seq.span(), 2.0);
        assert_eq!(seq.pose(0.9)[RIGHT.elbow], rest()[RIGHT.elbow]);
        assert_eq!(seq.pose(1.25)[RIGHT.elbow], rest()[RIGHT.elbow] + 20.0);
        assert_eq!(seq.pose(1.6)[RIGHT.elbow], rest()[RIGHT.elbow] + 40.0);
        assert!(Sequence::parse(r#"[{"motion":"walk"}]"#, View::Side).is_err());
        assert!(
            Sequence::parse(r#"[{"motion":"walk","duration":1,"arm":2}]"#, View::Side).is_err()
        );
        assert!(Sequence::parse(r#"[{"motion":"fly","duration":1}]"#, View::Side).is_err());
    }

    #[test]
    fn jump_never_pushes_the_body_below_the_ground() {
        let p = preset(Motion::Jump, View::Side).unwrap();
        for i in 0..=1000 {
            assert!(p.pose(i as f64 / 1000.0)[BODY_Y] <= 0.0);
        }
        assert!(p.pose(0.445)[BODY_Y] < -180.0);
    }

    #[test]
    fn short_steps_get_a_short_default_blend_and_keys_stay_distinct() {
        assert!(Sequence::parse(
            r#"[{"motion":"idle","duration":1},{"motion":"wave","duration":0.2}]"#,
            View::Side
        )
        .is_ok());
        assert!(Timeline::parse(r#"[{"at":0},{"at":0.0000005,"r-elbow":90}]"#).is_err());
        assert!(Timeline::parse(r#"[{"at":0.0000004},{"at":0.002}]"#).is_err());
        let t = Timeline::parse(r#"[{"at":0},{"at":0.0015,"r-elbow":90},{"at":1}]"#).unwrap();
        let times = Source::Keys {
            timeline: t,
            looped: false,
        }
        .pass_times(32, 20.0);
        assert!(times.contains(&0.0015));
        assert!(times.windows(2).all(|w| w[1] - w[0] >= MIN_STEP));
    }

    #[test]
    fn every_preset_loops_seamlessly() {
        for name in MOTION_NAMES {
            for view in [View::Side, View::Front, View::Space] {
                let Some(p) = preset(Motion::parse(name).unwrap(), view) else {
                    continue;
                };
                let (a, b) = (p.pose(0.0), p.pose(1.0));
                for ch in 0..CHANNELS.len() {
                    assert!((a[ch] - b[ch]).abs() < 1e-9, "{name} {}", CHANNELS[ch]);
                }
            }
        }
    }
}
