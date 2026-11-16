//! Varispeed reading for the mixer: samples a 48 kHz source at fractional positions that a
//! `speed::SpeedCurve` moves at a changing rate (pitch follows the rate, like tape).
//!
//! Each output sample is a Kaiser-windowed sinc interpolation with 16 zero crossings per side at
//! 1x. When the source plays faster than 1x the cutoff drops to `1 / speed` and the kernel widens,
//! so speeding up does not alias. Kernel weights are normalized to unity DC gain. The window comes
//! from a fixed table and the sinc from a rotation recurrence, so the result is deterministic and
//! costs about `32 * max(1, speed)` multiply-adds per output sample.

/// Zero crossings of the kernel on each side at 1x.
const ZEROS: f64 = 16.0;
const BETA: f64 = 9.0;
/// Cutoff as a fraction of the (lowered) Nyquist frequency, as in `resample.rs`.
const CUTOFF: f64 = 0.95;
/// Entries of the window table over `0..=1`.
const WINDOW_STEPS: usize = 4096;
/// Below this rate the sound fades out linearly, so a ramp to 0 ends in silence (a tape stop)
/// instead of a held DC level.
pub(super) const SILENT_BELOW: f64 = 0.05;

pub(super) struct Varispeed {
    window: Vec<f64>,
}

/// How source indices outside the played region are read.
#[derive(Clone, Copy)]
pub(super) enum Edge {
    /// Outside the source is silence.
    Silence,
    /// Indices wrap inside `start..start + length` (a looped clip).
    Wrap { start: usize, length: usize },
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let quarter = x * x / 4.0;
    for k in 1..60 {
        term *= quarter / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

impl Varispeed {
    pub(super) fn new() -> Self {
        let denominator = bessel_i0(BETA);
        let window = (0..=WINDOW_STEPS)
            .map(|i| {
                let r = i as f64 / WINDOW_STEPS as f64;
                bessel_i0(BETA * (1.0 - r * r).max(0.0).sqrt()) / denominator
            })
            .collect();
        Self { window }
    }

    fn window(&self, r: f64) -> f64 {
        let x = r.abs() * WINDOW_STEPS as f64;
        if x >= WINDOW_STEPS as f64 {
            return 0.0;
        }
        let i = x as usize;
        let f = x - i as f64;
        self.window[i] + (self.window[i + 1] - self.window[i]) * f
    }

    /// Interpolated `(left, right)` at source position `pos` (in samples) read at `speed`.
    /// `right` may be the same plane as `left` (mono), which is then filtered once.
    pub(super) fn sample(
        &self,
        left: &[f32],
        right: &[f32],
        pos: f64,
        speed: f64,
        edge: Edge,
    ) -> (f32, f32) {
        let relative = (1.0 / speed.max(1e-9)).min(1.0);
        let cutoff = CUTOFF * relative;
        let radius = ZEROS / relative;
        let base = pos.floor();
        let reach = radius.ceil() as i64;
        let first = base as i64 - reach + 1;
        let last = base as i64 + reach;
        let mono = std::ptr::eq(left, right);
        let step = std::f64::consts::PI * cutoff;
        let (step_sin, step_cos) = step.sin_cos();
        let mut theta = step * (first as f64 - pos);
        let (mut s, mut c) = theta.sin_cos();
        let (mut sum, mut acc_l, mut acc_r) = (0.0, 0.0, 0.0);
        for j in first..=last {
            let distance = j as f64 - pos;
            let sinc = if theta.abs() < 1e-9 { 1.0 } else { s / theta };
            let weight = sinc * self.window(distance / radius);
            if weight != 0.0 {
                sum += weight;
                if let Some(index) = resolve(j, left.len(), edge) {
                    acc_l += weight * f64::from(left[index]);
                    if !mono {
                        acc_r += weight * f64::from(right[index]);
                    }
                }
            }
            // Advance sin/cos of theta by one tap.
            (s, c) = (s * step_cos + c * step_sin, c * step_cos - s * step_sin);
            theta += step;
        }
        if sum == 0.0 {
            return (0.0, 0.0);
        }
        let l = (acc_l / sum) as f32;
        (l, if mono { l } else { (acc_r / sum) as f32 })
    }
}

fn resolve(j: i64, frames: usize, edge: Edge) -> Option<usize> {
    match edge {
        Edge::Silence => (j >= 0 && (j as usize) < frames).then_some(j as usize),
        Edge::Wrap { start, length } => {
            let offset = (j - start as i64).rem_euclid(length as i64) as usize;
            Some(start + offset)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_positions_at_unity_return_the_sample_of_a_band_limited_signal() {
        let v = Varispeed::new();
        let plane: Vec<f32> = (0..200).map(|i| (i as f32 * 0.3).sin()).collect();
        for pos in [40usize, 41, 100] {
            let (l, r) = v.sample(&plane, &plane, pos as f64, 1.0, Edge::Silence);
            assert!((l - plane[pos]).abs() < 1e-3, "pos {pos}");
            assert_eq!(l, r);
        }
    }

    #[test]
    fn constant_signal_keeps_unity_gain_at_any_speed_and_phase() {
        let v = Varispeed::new();
        let plane = vec![0.5_f32; 4000];
        for speed in [0.1, 0.5, 1.0, 2.5, 8.0] {
            for pos in [1000.0, 1000.25, 1999.75] {
                let (l, _) = v.sample(&plane, &plane, pos, speed, Edge::Silence);
                assert!((l - 0.5).abs() < 1e-5, "speed {speed} pos {pos}: {l}");
            }
        }
    }

    #[test]
    fn wrapping_reads_inside_the_loop() {
        let v = Varispeed::new();
        let plane = vec![9.0, 1.0, 1.0, 1.0, 1.0];
        let (l, _) = v.sample(
            &plane,
            &plane,
            2.5,
            1.0,
            Edge::Wrap {
                start: 1,
                length: 4,
            },
        );
        assert!((l - 1.0).abs() < 1e-5);
    }
}
