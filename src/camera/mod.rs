//! Cinematic camera rig: a camera container's zoom, focus point, framing anchor, and subject
//! follow, evaluated directly from time.
//!
//! The rig extends `data-hf-camera` (see `parse::animation::camera`) and is what the shot
//! composer (`data-hf-shots`, `parse::animation::shots`) compiles to. It keeps the camera model of
//! the classic camera: the container's subtree scales by `zoom` around the container center, and
//! a translation keeps the focus point `f` (camera px from the center) at that center. A child plane
//! with `data-hf-parallax = k` pans `k` times as far, so a point `p` of that plane appears at
//! `Z (p - k f)` from the frame center, where `Z` is the camera's scale.
//!
//! Each channel is a [`curve::Curve`]: zoom (interpolated in log space, so a dolly feels uniform),
//! focus x and y, framing anchor x and y (fractions of the camera box where a followed subject
//! sits), and the follow weight (0 = keyframed focus, 1 = subject framing). With a subject, the
//! framed focus is `f = (s - A / zoom) / k`, where `s` is the baked [`tracking::SubjectTrack`]
//! point, `A` the anchor offset from the center in camera px, and `k` the subject plane's
//! parallax; the rig blends between the keyframed and framed focus by the follow weight. Every
//! value is a pure function of time; the only state, the tracking filter, is baked once when the
//! composition is loaded, so frames can be rendered in any order, on any thread, in any split.
pub mod curve;
pub mod tracking;

use anyhow::{bail, Result};
use curve::{Curve, Interp};
use tracking::{FollowSettings, SubjectTrack};

pub const MIN_ZOOM: f64 = 0.05;
pub const MAX_ZOOM: f64 = 50.0;
/// Most keys one rig accepts (keyframes plus those the shot composer generates).
pub const MAX_KEYS: usize = 512;

/// The focus a key asks for on one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FocusValue {
    /// A fixed focus, in camera px from the center.
    Fixed(f64),
    /// Where the camera is at this key: the framed focus while following, else the previous key's.
    Inherit,
    /// The framed focus at another time (used to bring a following camera to rest), else the
    /// previous key's.
    TrackedAt(f64),
}

/// One rig key: the camera state reached at `at` seconds from the camera clip start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigKey {
    pub at: f64,
    pub zoom: f64,
    pub focus: [FocusValue; 2],
    /// Where a followed subject sits, as fractions of the camera box (0.5, 0.5 is the center).
    pub anchor: [f64; 2],
    /// 0 = keyframed focus, 1 = subject framing.
    pub follow: f64,
    /// Interpolation of the segment that ends at this key.
    pub interp: Interp,
}

/// The followed subject of a rig.
#[derive(Clone, Debug, PartialEq)]
pub struct Subject {
    /// Index into `Composition::elements`.
    pub element: usize,
    /// `data-hf-parallax` of the subject's plane (the camera's child that contains it; 1 without).
    pub depth: f64,
    pub settings: FollowSettings,
}

/// Camera state at one instant: `zoom` (1 = neutral) and the focus point (camera px from center).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub zoom: f64,
    pub focus: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
struct Framing {
    path: SubjectTrack,
    /// Camera box size (camera px).
    frame: [f64; 2],
    /// Depth magnification of the subject over time, for a camera of a 2.5D space.
    gain: Option<Gain>,
}

/// A positive factor sampled on a fixed grid from the camera clip start and read back with linear
/// interpolation (the ends hold): how much depth magnifies a followed subject in a 2.5D space.
#[derive(Clone, Debug, PartialEq)]
pub struct Gain {
    rate: f64,
    samples: Vec<f64>,
}

impl Gain {
    /// Samples `value` every `1 / rate` seconds over `[0, length]`.
    pub fn bake(rate: f64, length: f64, value: impl Fn(f64) -> f64) -> Self {
        let count = (length.max(0.0) * rate).ceil() as usize + 1;
        let samples = (0..count).map(|i| value(i as f64 / rate)).collect();
        Self { rate, samples }
    }

    pub fn at(&self, t: f64) -> f64 {
        let x = (t * self.rate).clamp(0.0, (self.samples.len() - 1) as f64);
        let i = (x.floor() as usize).min(self.samples.len().saturating_sub(2));
        let next = self.samples.get(i + 1).copied().unwrap_or(self.samples[i]);
        let f = x - i as f64;
        self.samples[i] + (next - self.samples[i]) * f
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraRig {
    keys: Vec<RigKey>,
    subject: Option<Subject>,
    /// Static CSS scale of the camera container.
    base_scale: f64,
    log_zoom: Curve,
    focus: [Curve; 2],
    anchor: [Curve; 2],
    follow: Curve,
    framing: Option<Framing>,
}

impl CameraRig {
    pub fn new(keys: Vec<RigKey>, subject: Option<Subject>, base_scale: f64) -> Result<Self> {
        if keys.is_empty() {
            bail!("a camera rig needs at least one key");
        }
        if keys.len() > MAX_KEYS {
            bail!("a camera rig accepts at most {MAX_KEYS} keys");
        }
        for pair in keys.windows(2) {
            // `partial_cmp` also rejects NaN times.
            if pair[1].at.partial_cmp(&pair[0].at) != Some(std::cmp::Ordering::Greater) {
                bail!("camera key times must be strictly increasing");
            }
        }
        for key in &keys {
            if !(MIN_ZOOM..=MAX_ZOOM).contains(&key.zoom) {
                bail!("zoom must be between {MIN_ZOOM} and {MAX_ZOOM}");
            }
            if !key.anchor.iter().all(|a| (0.0..=1.0).contains(a)) {
                bail!("anchor values must be fractions from 0 to 1");
            }
            if !(0.0..=1.0).contains(&key.follow) {
                bail!("follow must be between 0 and 1");
            }
            if key.follow > 0.0 && subject.is_none() {
                bail!("following needs a subject: add data-hf-camera-subject=\"<id>\"");
            }
        }
        let channel = |value: &dyn Fn(&RigKey) -> f64| {
            let points: Vec<_> = keys.iter().map(|k| (k.at, value(k), k.interp)).collect();
            Curve::new(&points)
        };
        let mut rig = Self {
            log_zoom: channel(&|k| k.zoom.ln()),
            anchor: [channel(&|k| k.anchor[0]), channel(&|k| k.anchor[1])],
            follow: channel(&|k| k.follow),
            focus: [Curve::constant(0.0), Curve::constant(0.0)],
            keys,
            subject,
            base_scale,
            framing: None,
        };
        rig.resolve_focus();
        Ok(rig)
    }

    pub(crate) fn offset_subject(&mut self, offset: usize) {
        if let Some(subject) = &mut self.subject {
            subject.element += offset;
        }
    }

    pub fn subject(&self) -> Option<&Subject> {
        self.subject.as_ref()
    }

    pub fn keys(&self) -> &[RigKey] {
        &self.keys
    }

    /// Whether the subject's path has been baked (always true without a subject once loaded).
    pub fn is_bound(&self) -> bool {
        self.subject.is_none() || self.framing.is_some()
    }

    /// Attaches the baked subject path and the camera box size, then resolves the keys whose
    /// focus depends on the framing.
    pub fn bind(&mut self, path: SubjectTrack, frame: [f64; 2], gain: Option<Gain>) {
        self.framing = Some(Framing { path, frame, gain });
        self.resolve_focus();
    }

    /// Focus that keeps the subject on its anchor at clip time `t`, on followed axes.
    fn framed(&self, t: f64) -> Option<[Option<f64>; 2]> {
        let framing = self.framing.as_ref()?;
        let subject = self.subject.as_ref()?;
        let (axis, depth) = (subject.settings.axis, subject.depth);
        let gain = framing.gain.as_ref().map_or(1.0, |g| g.at(t));
        let zoom = self.log_zoom.eval(t).exp() * gain;
        let point = framing.path.at(t);
        Some(std::array::from_fn(|i| {
            axis[i].then(|| {
                let anchor = (self.anchor[i].eval(t) - 0.5) * framing.frame[i];
                (point[i] - anchor / zoom) / depth
            })
        }))
    }

    fn resolve_focus(&mut self) {
        let mut values = [Vec::new(), Vec::new()];
        let mut previous = [0.0; 2];
        for key in &self.keys {
            let framed = |t: f64| self.framed(t);
            for axis in 0..2 {
                let value = match key.focus[axis] {
                    FocusValue::Fixed(v) => v,
                    FocusValue::Inherit if key.follow > 0.0 => framed(key.at)
                        .and_then(|f| f[axis])
                        .unwrap_or(previous[axis]),
                    FocusValue::Inherit => previous[axis],
                    FocusValue::TrackedAt(t) => {
                        framed(t).and_then(|f| f[axis]).unwrap_or(previous[axis])
                    }
                };
                values[axis].push((key.at, value, key.interp));
                previous[axis] = value;
            }
        }
        self.focus = values.map(|points| Curve::new(&points));
    }

    /// The camera state at clip time `t` (seconds from the camera clip start).
    pub fn pose(&self, t: f64) -> CameraPose {
        let zoom = self.log_zoom.eval(t).exp();
        let mut focus = [self.focus[0].eval(t), self.focus[1].eval(t)];
        if let Some(framed) = self.framed(t) {
            let weight = self.follow.eval(t).clamp(0.0, 1.0);
            for axis in 0..2 {
                if let Some(target) = framed[axis] {
                    focus[axis] += weight * (target - focus[axis]);
                }
            }
        }
        CameraPose { zoom, focus }
    }

    /// The container translation (parent px) and zoom at clip time `t`.
    pub fn transform(&self, t: f64) -> ([f32; 2], f32) {
        let pose = self.pose(t);
        let scale = self.base_scale * pose.zoom;
        (
            [
                (-scale * pose.focus[0]) as f32,
                (-scale * pose.focus[1]) as f32,
            ],
            pose.zoom as f32,
        )
    }

    pub fn times(&self) -> impl Iterator<Item = f64> + '_ {
        self.keys.iter().map(|key| key.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(at: f64, zoom: f64, x: FocusValue, follow: f64) -> RigKey {
        RigKey {
            at,
            zoom,
            focus: [x, FocusValue::Fixed(0.0)],
            anchor: [0.5, 0.5],
            follow,
            interp: Interp::Smooth,
        }
    }

    #[test]
    fn keyframed_rig_holds_end_poses_and_zooms_in_log_space() {
        let rig = CameraRig::new(
            vec![
                key(0.0, 1.0, FocusValue::Fixed(0.0), 0.0),
                key(2.0, 4.0, FocusValue::Fixed(100.0), 0.0),
            ],
            None,
            1.0,
        )
        .unwrap();
        assert_eq!(rig.pose(-1.0).zoom, 1.0);
        assert!((rig.pose(1.0).zoom - 2.0).abs() < 1e-12);
        assert!((rig.pose(5.0).focus[0] - 100.0).abs() < 1e-12);
        let ([x, _], zoom) = rig.transform(2.0);
        assert_eq!((x, zoom), (-400.0, 4.0));
    }

    #[test]
    fn follow_keeps_the_subject_on_its_anchor() {
        let settings = FollowSettings {
            lag: 0.0,
            axis: [true, false],
            ..Default::default()
        };
        let path = SubjectTrack::bake(&settings, 4.0, |t| [-300.0 + 40.0 * t, 90.0]).unwrap();
        let mut rig = CameraRig::new(
            vec![
                RigKey {
                    anchor: [0.4, 0.5],
                    ..key(0.0, 1.5, FocusValue::Inherit, 1.0)
                },
                RigKey {
                    anchor: [0.4, 0.5],
                    ..key(4.0, 2.0, FocusValue::Inherit, 1.0)
                },
            ],
            Some(Subject {
                element: 3,
                depth: 1.25,
                settings,
            }),
            1.2,
        )
        .unwrap();
        rig.bind(path, [1600.0, 900.0], None);
        for t in [0.0, 1.1, 2.5, 4.0] {
            let pose = rig.pose(t);
            let subject = -300.0 + 40.0 * t;
            // Screen offset from the frame center, in camera px at base scale.
            let screen = pose.zoom * (subject - 1.25 * pose.focus[0]);
            assert!(
                (screen - (0.4 - 0.5) * 1600.0).abs() < 1e-6,
                "{screen} at {t}"
            );
        }
    }

    #[test]
    fn leaving_a_follow_brings_the_camera_to_rest_without_a_jump() {
        let settings = FollowSettings {
            lag: 0.0,
            ..Default::default()
        };
        let path = SubjectTrack::bake(&settings, 3.0, |t| [50.0 * t, 0.0]).unwrap();
        let mut rig = CameraRig::new(
            vec![
                key(0.0, 1.0, FocusValue::Inherit, 1.0),
                key(1.0, 1.0, FocusValue::Inherit, 1.0),
                key(3.0, 1.0, FocusValue::Fixed(80.0), 0.0),
            ],
            Some(Subject {
                element: 1,
                depth: 1.0,
                settings,
            }),
            1.0,
        )
        .unwrap();
        rig.bind(path, [100.0, 100.0], None);
        let dt = 1e-4;
        let speed = |t: f64| (rig.pose(t + dt).focus[0] - rig.pose(t - dt).focus[0]) / (2.0 * dt);
        assert!((speed(1.0) - 50.0).abs() < 0.1);
        assert!(speed(2.9999).abs() < 1.0);
        assert!((rig.pose(3.0).focus[0] - 80.0).abs() < 1e-9);
    }
}
