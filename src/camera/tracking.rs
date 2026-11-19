//! Subject tracking for the camera rig: the followed point over a camera clip, baked once on a fixed
//! time grid and then read as a pure function of time.
//!
//! The subject's position is a pure function of time (its own and its ancestors' motion), but a
//! dead zone and a smoothing lag are naturally stateful. To keep every frame independent of every
//! other, the stateful part runs once, when the composition is loaded, from the clip start forward
//! on a grid of [`RATE`] samples per second that does not depend on the output frame rate, thread
//! count, render order, or render split. Frames then read the baked path with a Catmull–Rom spline
//! (continuous velocity). The smoothing is two cascaded first-order lags (each half of `lag`), which
//! never overshoots or oscillates, and starts in its steady state for the subject's initial
//! velocity, so a moving subject is framed consistently from the first frame.
use anyhow::{bail, Result};

/// Samples per second of the baked path.
pub const RATE: f64 = 240.0;
/// Longest camera clip that can track a subject, in samples (about 69 minutes).
const MAX_SAMPLES: usize = 1_000_000;
/// Shortest window (seconds) over which the initial velocity is estimated (it is twice the lag
/// when that is longer).
const VELOCITY_WINDOW: f64 = 0.25;

/// Camera-level follow options (`data-hf-camera-*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowSettings {
    /// Seconds the camera trails the subject (0 locks onto it).
    pub lag: f64,
    /// Seconds ahead along the subject's own motion that the camera aims at.
    pub lookahead: f64,
    /// Half-extent (px, camera space) the subject may drift before the camera reacts.
    pub deadzone: [f64; 2],
    /// Px added to the subject's box center (camera space).
    pub offset: [f64; 2],
    /// Followed axes `[x, y]`.
    pub axis: [bool; 2],
}

impl Default for FollowSettings {
    fn default() -> Self {
        Self {
            lag: 0.3,
            lookahead: 0.0,
            deadzone: [0.0; 2],
            offset: [0.0; 2],
            axis: [true; 2],
        }
    }
}

/// The followed point over a camera clip, in px from the camera's center (unzoomed camera space).
#[derive(Clone, Debug, PartialEq)]
pub struct SubjectTrack {
    samples: Vec<[f64; 2]>,
}

impl SubjectTrack {
    /// Bakes the followed point over `0..=length` seconds of the camera clip. `subject(t)` is the
    /// subject's point at clip time `t` (a pure function).
    pub fn bake(
        settings: &FollowSettings,
        length: f64,
        subject: impl Fn(f64) -> [f64; 2],
    ) -> Result<Self> {
        let count = (length.max(0.0) * RATE).ceil() as usize + 1;
        if count > MAX_SAMPLES {
            bail!("camera subject tracking is limited to camera clips of 69 minutes");
        }
        let dt = 1.0 / RATE;
        let target = |i: usize| {
            let t = ((i as f64) * dt + settings.lookahead).min(length);
            let [x, y] = subject(t);
            [x + settings.offset[0], y + settings.offset[1]]
        };
        // Dead zone: the aim point only moves when the subject leaves the box around it.
        let mut held = Vec::with_capacity(count);
        let mut aim = target(0);
        for i in 0..count {
            let point = target(i);
            for axis in 0..2 {
                let zone = settings.deadzone[axis];
                let d = point[axis] - aim[axis];
                if d > zone {
                    aim[axis] = point[axis] - zone;
                } else if d < -zone {
                    aim[axis] = point[axis] + zone;
                }
            }
            held.push(aim);
        }
        if settings.lag <= 0.0 {
            return Ok(Self { samples: held });
        }
        let alpha = 1.0 - (-dt / (settings.lag / 2.0)).exp();
        // Least-squares slope over the opening window: stepped motion (a walk advancing once per
        // cel) gets its average speed, not the speed of whichever step the window ends on.
        let span = VELOCITY_WINDOW.max(2.0 * settings.lag);
        let window = ((span * RATE).round() as usize).min(count - 1);
        let mut first = [0.0; 2];
        let mut second = [0.0; 2];
        for axis in 0..2 {
            let velocity = if window == 0 {
                0.0
            } else {
                let n = (window + 1) as f64;
                let mean_t = window as f64 * dt / 2.0;
                let mean_x = held[..=window].iter().map(|p| p[axis]).sum::<f64>() / n;
                let (mut covariance, mut variance) = (0.0, 0.0);
                for (i, p) in held[..=window].iter().enumerate() {
                    let dt_i = i as f64 * dt - mean_t;
                    covariance += dt_i * (p[axis] - mean_x);
                    variance += dt_i * dt_i;
                }
                covariance / variance
            };
            // Steady-state lag of one discrete stage on a ramp of slope `velocity`.
            let lag = velocity * dt * (1.0 - alpha) / alpha;
            first[axis] = held[0][axis] - lag;
            second[axis] = held[0][axis] - 2.0 * lag;
        }
        let mut samples = Vec::with_capacity(count);
        for (i, point) in held.iter().enumerate() {
            if i > 0 {
                for axis in 0..2 {
                    first[axis] += alpha * (point[axis] - first[axis]);
                    second[axis] += alpha * (first[axis] - second[axis]);
                }
            }
            samples.push(second);
        }
        Ok(Self { samples })
    }

    /// The followed point at clip time `t` (clamped to the clip).
    pub fn at(&self, t: f64) -> [f64; 2] {
        let last = self.samples.len() - 1;
        let position = (t * RATE).clamp(0.0, last as f64);
        let i = (position.floor() as usize).min(last);
        let u = position - i as f64;
        // Beyond the ends the path continues along its end segments, so the start and end keep
        // their real velocity instead of half of it.
        let p = |k: isize| {
            if last == 0 {
                return self.samples[0];
            }
            if k < 0 {
                let (a, b) = (self.samples[0], self.samples[1]);
                return [2.0 * a[0] - b[0], 2.0 * a[1] - b[1]];
            }
            if k as usize > last {
                let (a, b) = (self.samples[last], self.samples[last - 1]);
                return [2.0 * a[0] - b[0], 2.0 * a[1] - b[1]];
            }
            self.samples[k as usize]
        };
        let (p0, p1, p2, p3) = (
            p(i as isize - 1),
            p(i as isize),
            p(i as isize + 1),
            p(i as isize + 2),
        );
        let mut out = [0.0; 2];
        for axis in 0..2 {
            let (a, b, c, d) = (p0[axis], p1[axis], p2[axis], p3[axis]);
            out[axis] = b + 0.5
                * u
                * (c - a + u * (2.0 * a - 5.0 * b + 4.0 * c - d + u * (3.0 * (b - c) + d - a)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_follow_starts_in_place_and_tracks_a_constant_walk() {
        let settings = FollowSettings {
            lag: 0.4,
            lookahead: 0.4,
            ..Default::default()
        };
        let track = SubjectTrack::bake(&settings, 6.0, |t| [100.0 + 50.0 * t, 20.0]).unwrap();
        // Look-ahead equal to the lag cancels the steady-state trail: the point sits on the
        // subject, up to a constant half-sample offset of the discrete filter, from the first frame.
        let trail = track.at(0.0)[0] - 100.0;
        assert!(trail.abs() < 0.25, "{trail}");
        for t in [0.0, 1.0, 3.3, 5.0] {
            let [x, y] = track.at(t);
            assert!((x - (100.0 + 50.0 * t) - trail).abs() < 1e-6, "{x} at {t}");
            assert!((y - 20.0).abs() < 1e-9);
        }
    }

    #[test]
    fn stepped_motion_is_smoothed_without_overshoot() {
        let settings = FollowSettings::default();
        // 12 steps per second of 3.75 px, like a stepped walk cycle.
        let step = |t: f64| [(t * 12.0).floor() * 3.75, 0.0];
        let track = SubjectTrack::bake(&settings, 4.0, step).unwrap();
        let mut previous = track.at(0.0)[0];
        let mut previous_speed = 0.0_f64;
        for i in 1..=4000 {
            let x = track.at(i as f64 / 1000.0)[0];
            let speed = (x - previous) * 1000.0;
            assert!(speed >= -1e-6, "camera moved backwards at {i} ms");
            assert!(x <= step(4.0)[0] + 1e-9);
            if i > 1 {
                // No acceleration spikes: the speed changes by a small amount per millisecond.
                assert!((speed - previous_speed).abs() < 2.0, "jerk at {i} ms");
            }
            previous = x;
            previous_speed = speed;
        }
    }

    #[test]
    fn dead_zone_ignores_small_drift() {
        let settings = FollowSettings {
            lag: 0.0,
            deadzone: [10.0, 10.0],
            ..Default::default()
        };
        let track =
            SubjectTrack::bake(&settings, 2.0, |t| [(t * 9.0).sin() * 8.0, t * 30.0]).unwrap();
        assert_eq!(track.at(1.3)[0], 0.0);
        assert!((track.at(2.0)[1] - 50.0).abs() < 1e-6);
    }
}
