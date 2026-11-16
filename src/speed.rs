//! Variable playback speed for `<video>` and `<audio>`: a `SpeedCurve` maps clip time (seconds
//! since the clip start) to media time (seconds advanced in the source).
//!
//! A curve is a constant rate (`data-hf-speed`) or a ramp of keyframes (`data-hf-speed-ramp`),
//! where each segment interpolates the rate with the destination keyframe's easing and the first
//! and last rates hold outside the keyed interval. Media time is the integral of the rate, so a
//! clip never jumps when the rate changes. Linear and `steps()` segments integrate in closed form;
//! other eased segments use a cumulative table built at parse time (Simpson's rule on cells of at
//! most 1 ms, and the same rule over the partial cell), so media time is continuous and monotonic,
//! every lookup is O(log n), and the result is deterministic.
//! The video frame picker (`model::VideoSource::frame_at`) and the audio mixer
//! (`audio::varispeed`) read the same curve, so picture and sound stay in sync.
use crate::model::Easing;
use anyhow::{bail, Result};

/// Fastest accepted rate.
pub const MAX_SPEED: f64 = 16.0;
/// Slowest constant rate (`data-hf-speed`); ramps may reach 0 to freeze the picture.
pub const MIN_CONSTANT_SPEED: f64 = 0.05;
/// Latest keyframe time accepted in a ramp, in seconds.
pub const MAX_RAMP_SECONDS: f64 = 3600.0;
/// Most keyframes in one ramp.
pub const MAX_KEYS: usize = 256;
/// Table cells per second of an eased segment (cell length of at most 1 ms).
const CELLS_PER_SECOND: f64 = 1000.0;

/// One ramp keyframe: the rate reached at `at` seconds after the clip start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedKey {
    pub at: f64,
    pub speed: f64,
    /// Easing of the segment that ends at this keyframe.
    pub easing: Easing,
}

/// Cumulative media time for an eased segment: `cum[i]` is the media time advanced from the
/// segment start to cell `i`.
#[derive(Clone, Debug, PartialEq)]
struct Table {
    cell: f64,
    cum: Vec<f64>,
}

/// How one segment's rate is integrated.
#[derive(Clone, Debug, PartialEq)]
enum Integral {
    /// Linear rate: closed form.
    Linear,
    /// `steps(count, jump-start | jump-end)`: piecewise constant, closed form.
    Steps { count: u32, jump_start: bool },
    /// Any other easing: cumulative Simpson table.
    Table(Table),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeedCurve {
    keys: Vec<SpeedKey>,
    /// Media time at each keyframe.
    media_at_key: Vec<f64>,
    /// `integrals[i]` integrates the segment from `keys[i]` to `keys[i + 1]`.
    integrals: Vec<Integral>,
    constant: bool,
}

impl SpeedCurve {
    /// A constant rate (`data-hf-speed`).
    pub fn constant(speed: f64) -> Result<Self> {
        if !speed.is_finite() || !(MIN_CONSTANT_SPEED..=MAX_SPEED).contains(&speed) {
            bail!("data-hf-speed must be between {MIN_CONSTANT_SPEED} and {MAX_SPEED}");
        }
        let mut curve = Self::ramp(vec![SpeedKey {
            at: 0.0,
            speed,
            easing: Easing::Linear,
        }])?;
        curve.constant = true;
        Ok(curve)
    }

    /// A ramp from keyframes with strictly increasing times and rates in `0..=MAX_SPEED`.
    pub fn ramp(keys: Vec<SpeedKey>) -> Result<Self> {
        if keys.is_empty() || keys.len() > MAX_KEYS {
            bail!("a speed ramp needs 1 to {MAX_KEYS} keyframes");
        }
        for (i, key) in keys.iter().enumerate() {
            if !key.at.is_finite() || key.at < 0.0 || key.at > MAX_RAMP_SECONDS {
                bail!("at must be between 0 and {MAX_RAMP_SECONDS} seconds");
            }
            if i > 0 && key.at <= keys[i - 1].at {
                bail!("keyframe times must be strictly increasing");
            }
            if !key.speed.is_finite() || !(0.0..=MAX_SPEED).contains(&key.speed) {
                bail!("speed must be between 0 and {MAX_SPEED}");
            }
        }
        if keys.iter().all(|k| k.speed == 0.0) {
            bail!("a speed ramp cannot stay at speed 0 everywhere");
        }
        let mut media_at_key = vec![keys[0].speed * keys[0].at];
        let mut integrals = Vec::with_capacity(keys.len().saturating_sub(1));
        for pair in keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let integral = match b.easing {
                Easing::Linear => Integral::Linear,
                Easing::Steps { count, jump_start } => Integral::Steps { count, jump_start },
                _ => Integral::Table(build_table(a, b)),
            };
            let total = integrate(&integral, a, b, b.at);
            media_at_key.push(media_at_key.last().unwrap() + total);
            integrals.push(integral);
        }
        Ok(Self {
            keys,
            media_at_key,
            integrals,
            constant: false,
        })
    }

    /// The keyframes (one at time 0 for a constant rate).
    pub fn keys(&self) -> &[SpeedKey] {
        &self.keys
    }

    pub fn is_constant(&self) -> bool {
        self.constant
    }

    /// Index of the segment that contains `t` (`keys[i].at <= t < keys[i + 1].at`), or `None`
    /// before the first or after the last keyframe.
    fn segment(&self, t: f64) -> Option<usize> {
        let next = self.keys.partition_point(|k| k.at <= t);
        (next > 0 && next < self.keys.len()).then(|| next - 1)
    }

    /// Playback rate at clip time `t`.
    pub fn speed(&self, t: f64) -> f64 {
        let first = self.keys[0];
        let last = *self.keys.last().unwrap();
        if t < first.at {
            return first.speed;
        }
        match self.segment(t) {
            Some(i) => segment_speed(self.keys[i], self.keys[i + 1], t),
            None => last.speed,
        }
    }

    /// Media seconds advanced after `t` seconds of clip time (0 at `t <= 0`).
    pub fn media(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        let first = self.keys[0];
        if t <= first.at {
            return first.speed * t;
        }
        let Some(i) = self.segment(t) else {
            let last = self.keys.len() - 1;
            return self.media_at_key[last] + self.keys[last].speed * (t - self.keys[last].at);
        };
        let (a, b) = (self.keys[i], self.keys[i + 1]);
        self.media_at_key[i] + integrate(&self.integrals[i], a, b, t)
    }

    /// Earliest clip time at which `media` seconds of source have played, or `None` when the curve
    /// ends at speed 0 before reaching it.
    pub fn time_for(&self, media: f64) -> Option<f64> {
        if media <= 0.0 {
            return Some(0.0);
        }
        let last = self.keys.len() - 1;
        let end = self.media_at_key[last];
        if media > end {
            let speed = self.keys[last].speed;
            return (speed > 0.0).then(|| self.keys[last].at + (media - end) / speed);
        }
        // `media` is monotonic in `t`: bisect on [0, last.at].
        let (mut lo, mut hi) = (0.0_f64, self.keys[last].at);
        for _ in 0..80 {
            let mid = (lo + hi) / 2.0;
            if self.media(mid) >= media {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        Some(hi)
    }

    /// The curve for `check`: `{"speed": n}` or `{"ramp": [{"at", "speed"}, ...]}`.
    pub fn to_json(&self) -> serde_json::Value {
        if self.constant {
            return serde_json::json!({"speed": self.keys[0].speed});
        }
        let keys: Vec<_> = self
            .keys
            .iter()
            .map(|k| serde_json::json!({"at": k.at, "speed": k.speed}))
            .collect();
        serde_json::json!({"ramp": keys})
    }
}

/// Rate inside the segment `a..b` at time `t`, clamped to the accepted range so overshooting
/// easings never run the media backwards.
fn segment_speed(a: SpeedKey, b: SpeedKey, t: f64) -> f64 {
    let u = ((t - a.at) / (b.at - a.at)).clamp(0.0, 1.0);
    let eased = f64::from(b.easing.apply(u as f32));
    (a.speed + (b.speed - a.speed) * eased).clamp(0.0, MAX_SPEED)
}

/// Media time advanced from `a.at` to `t` (clamped to the segment) inside the segment `a..b`.
fn integrate(integral: &Integral, a: SpeedKey, b: SpeedKey, t: f64) -> f64 {
    let length = b.at - a.at;
    let dt = (t - a.at).clamp(0.0, length);
    match integral {
        Integral::Linear => {
            let slope = (b.speed - a.speed) / length;
            a.speed * dt + slope * dt * dt / 2.0
        }
        Integral::Steps { count, jump_start } => {
            // Step k (of n) holds the eased fraction min(1, (k + jump) / n) for length / n seconds.
            let n = f64::from(*count);
            let jump = f64::from(u8::from(*jump_start));
            let width = length / n;
            let whole = ((dt / width).floor()).min(n);
            // Sum over k < whole of (k + jump) / n; never above 1 because k + jump <= n.
            let fraction_sum = whole * (whole - 1.0 + 2.0 * jump) / (2.0 * n);
            let step = |k: f64| a.speed + (b.speed - a.speed) * ((k + jump) / n).min(1.0);
            width * (whole * a.speed + (b.speed - a.speed) * fraction_sum)
                + (dt - whole * width) * step(whole)
        }
        Integral::Table(table) => {
            let cell = ((dt / table.cell) as usize).min(table.cum.len() - 2);
            let t0 = a.at + cell as f64 * table.cell;
            table.cum[cell] + simpson(a, b, t0, a.at + dt)
        }
    }
}

/// Simpson's rule for the rate over `t0..t1` inside the segment `a..b`.
fn simpson(a: SpeedKey, b: SpeedKey, t0: f64, t1: f64) -> f64 {
    let middle = (t0 + t1) / 2.0;
    (segment_speed(a, b, t0) + 4.0 * segment_speed(a, b, middle) + segment_speed(a, b, t1))
        * (t1 - t0)
        / 6.0
}

fn build_table(a: SpeedKey, b: SpeedKey) -> Table {
    let length = b.at - a.at;
    let cells = ((length * CELLS_PER_SECOND).ceil() as usize).max(16);
    let cell = length / cells as f64;
    let mut cum = Vec::with_capacity(cells + 1);
    cum.push(0.0);
    let mut sum = 0.0;
    for i in 0..cells {
        let t0 = a.at + i as f64 * cell;
        sum += simpson(a, b, t0, t0 + cell);
        cum.push(sum);
    }
    Table { cell, cum }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(at: f64, speed: f64, easing: Easing) -> SpeedKey {
        SpeedKey { at, speed, easing }
    }

    #[test]
    fn constant_rate_scales_time() {
        let curve = SpeedCurve::constant(0.5).unwrap();
        assert_eq!(curve.media(2.0), 1.0);
        assert_eq!(curve.time_for(1.0), Some(2.0));
        assert!(SpeedCurve::constant(0.0).is_err());
        assert!(SpeedCurve::constant(17.0).is_err());
    }

    #[test]
    fn linear_ramp_integrates_exactly_and_holds_outside() {
        // 1x until 1 s, linear down to 0.25x at 2 s, then 0.25x.
        let curve = SpeedCurve::ramp(vec![
            key(1.0, 1.0, Easing::Linear),
            key(2.0, 0.25, Easing::Linear),
        ])
        .unwrap();
        assert_eq!(curve.media(1.0), 1.0);
        assert!((curve.media(2.0) - 1.625).abs() < 1e-12);
        assert!((curve.media(4.0) - 2.125).abs() < 1e-12);
        assert_eq!(curve.speed(0.5), 1.0);
        assert_eq!(curve.speed(3.0), 0.25);
        let t = curve.time_for(1.625).unwrap();
        assert!((t - 2.0).abs() < 1e-9);
    }

    #[test]
    fn eased_ramp_is_continuous_monotonic_and_accurate() {
        let curve = SpeedCurve::ramp(vec![
            key(0.0, 2.0, Easing::Linear),
            key(1.0, 0.2, Easing::EaseInOut),
        ])
        .unwrap();
        // Smoothstep from 2 to 0.2 averages to exactly 1.1 over the second.
        assert!((curve.media(1.0) - 1.1).abs() < 1e-9);
        let mut previous = 0.0;
        for i in 0..=2000 {
            let m = curve.media(i as f64 / 1000.0);
            assert!(m >= previous, "monotonic at {i}");
            previous = m;
        }
        let t = curve.time_for(0.7).unwrap();
        assert!((curve.media(t) - 0.7).abs() < 1e-9);
    }

    #[test]
    fn stepped_easing_integrates_exactly_and_stays_continuous() {
        // steps(2, jump-start) from 0x to 2x over 1 s: 1x for 0.5 s, then 2x.
        let curve = SpeedCurve::ramp(vec![
            key(0.0, 0.0, Easing::Linear),
            key(
                1.0,
                2.0,
                Easing::Steps {
                    count: 2,
                    jump_start: true,
                },
            ),
        ])
        .unwrap();
        assert!((curve.media(0.5 - 1e-9) - 0.5).abs() < 1e-8);
        assert!((curve.media(0.5) - 0.5).abs() < 1e-12);
        assert!((curve.media(0.75) - 1.0).abs() < 1e-12);
        assert!((curve.media(1.0) - 1.5).abs() < 1e-12);
        let t = curve.time_for(1.0).unwrap();
        assert!((t - 0.75).abs() < 1e-9, "{t}");
        let mut previous = 0.0;
        for i in 0..=1000 {
            let m = curve.media(i as f64 / 500.0);
            assert!(m >= previous, "monotonic at {i}");
            previous = m;
        }
    }

    #[test]
    fn eased_partial_cells_meet_the_table_at_cell_edges() {
        let curve = SpeedCurve::ramp(vec![
            key(0.0, 3.0, Easing::Linear),
            key(1.0, 0.5, Easing::Bezier([0.7, 0.0, 0.3, 1.0])),
        ])
        .unwrap();
        for i in 1..1000 {
            let edge = i as f64 / 1000.0;
            let jump = (curve.media(edge) - curve.media(edge - 1e-12)).abs();
            assert!(jump < 1e-9, "jump {jump} at {edge}");
        }
    }

    #[test]
    fn a_ramp_that_ends_frozen_never_reaches_later_media() {
        let curve = SpeedCurve::ramp(vec![
            key(0.0, 1.0, Easing::Linear),
            key(1.0, 0.0, Easing::Linear),
        ])
        .unwrap();
        assert_eq!(curve.media(5.0), 0.5);
        assert_eq!(curve.time_for(0.6), None);
    }

    #[test]
    fn rejects_invalid_ramps() {
        assert!(SpeedCurve::ramp(vec![]).is_err());
        assert!(SpeedCurve::ramp(vec![
            key(1.0, 1.0, Easing::Linear),
            key(1.0, 2.0, Easing::Linear)
        ])
        .is_err());
        assert!(SpeedCurve::ramp(vec![key(0.0, -1.0, Easing::Linear)]).is_err());
        assert!(SpeedCurve::ramp(vec![key(0.0, 0.0, Easing::Linear)]).is_err());
    }
}
