//! One animated camera channel (zoom, focus, framing anchor, or follow weight) as a pure function
//! of time.
//!
//! A curve passes through its keys. Each segment is shaped by the key that ends it: `Smooth`
//! segments are cubic Hermite pieces whose tangents are chosen automatically with the monotone
//! (Fritsch–Butland, "PCHIP") rule, so velocity is continuous across keys joined by smooth
//! segments, a value never overshoots its neighbouring keys, and a key between a rise and a fall
//! (or next to a held value) is crossed at rest. The first and last keys are reached at rest.
//! `Eased` segments use a keyframe easing exactly as `data-hf-keyframes` does (linear,
//! `cubic-bezier`, spring, ...); the smooth side of a key next to one starts or ends at rest.
use crate::model::Easing;

/// How the segment that ends at a key is interpolated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interp {
    /// Cubic Hermite with automatic, monotone tangents.
    Smooth,
    /// A keyframe easing between the two key values.
    Eased(Easing),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    at: Vec<f64>,
    value: Vec<f64>,
    /// Tangent (value per second) at each key.
    slope: Vec<f64>,
    /// Interpolation of the segment ending at each key (the first entry is unused).
    interp: Vec<Interp>,
}

impl Curve {
    /// `points` are `(at, value, interp)` with strictly increasing `at`.
    pub fn new(points: &[(f64, f64, Interp)]) -> Self {
        let at: Vec<f64> = points.iter().map(|p| p.0).collect();
        let value: Vec<f64> = points.iter().map(|p| p.1).collect();
        let interp: Vec<Interp> = points.iter().map(|p| p.2).collect();
        let n = points.len();
        let mut slope = vec![0.0; n];
        for k in 1..n.saturating_sub(1) {
            if interp[k] != Interp::Smooth || interp[k + 1] != Interp::Smooth {
                continue;
            }
            let (h0, h1) = (at[k] - at[k - 1], at[k + 1] - at[k]);
            let (d0, d1) = (
                (value[k] - value[k - 1]) / h0,
                (value[k + 1] - value[k]) / h1,
            );
            if d0 * d1 <= 0.0 {
                continue;
            }
            // Weighted harmonic mean of the secants: monotone and C1 (Fritsch–Butland).
            let (w0, w1) = (2.0 * h1 + h0, h1 + 2.0 * h0);
            slope[k] = (w0 + w1) / (w0 / d0 + w1 / d1);
        }
        Self {
            at,
            value,
            slope,
            interp,
        }
    }

    /// A curve that holds `value` at all times.
    pub fn constant(value: f64) -> Self {
        Self::new(&[(0.0, value, Interp::Smooth)])
    }

    pub fn eval(&self, t: f64) -> f64 {
        let n = self.at.len();
        if n == 0 {
            return 0.0;
        }
        if t <= self.at[0] {
            return self.value[0];
        }
        if t >= self.at[n - 1] {
            return self.value[n - 1];
        }
        let i = self.at.partition_point(|&at| at <= t) - 1;
        let h = self.at[i + 1] - self.at[i];
        let u = (t - self.at[i]) / h;
        let (a, b) = (self.value[i], self.value[i + 1]);
        match self.interp[i + 1] {
            Interp::Smooth => {
                let (u2, u3) = (u * u, u * u * u);
                let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
                let h10 = u3 - 2.0 * u2 + u;
                let h01 = -2.0 * u3 + 3.0 * u2;
                let h11 = u3 - u2;
                h00 * a + h10 * h * self.slope[i] + h01 * b + h11 * h * self.slope[i + 1]
            }
            Interp::Eased(easing) => a + (b - a) * f64::from(easing.apply(u as f32)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smooth(points: &[(f64, f64)]) -> Curve {
        let p: Vec<_> = points
            .iter()
            .map(|&(t, v)| (t, v, Interp::Smooth))
            .collect();
        Curve::new(&p)
    }

    #[test]
    fn passes_through_keys_and_holds_outside() {
        let c = smooth(&[(0.0, 1.0), (2.0, 3.0), (4.0, 2.0)]);
        assert_eq!(c.eval(-1.0), 1.0);
        assert_eq!(c.eval(0.0), 1.0);
        assert!((c.eval(2.0) - 3.0).abs() < 1e-12);
        assert_eq!(c.eval(9.0), 2.0);
    }

    #[test]
    fn velocity_is_continuous_and_values_never_overshoot() {
        let c = smooth(&[(0.0, 0.0), (1.0, 1.0), (3.0, 4.0), (3.5, 4.0), (5.0, -2.0)]);
        let dt = 1e-5;
        for &key in &[1.0, 3.0, 3.5] {
            let left = (c.eval(key) - c.eval(key - dt)) / dt;
            let right = (c.eval(key + dt) - c.eval(key)) / dt;
            assert!(
                (left - right).abs() < 1e-3,
                "slope jump at {key}: {left} vs {right}"
            );
        }
        for step in 0..=5000 {
            let t = step as f64 / 1000.0;
            let v = c.eval(t);
            assert!((-2.0..=4.0 + 1e-12).contains(&v), "{v} at {t}");
        }
        // A held value is never left: the segment between equal keys is flat.
        for step in 0..=50 {
            assert!((c.eval(3.0 + step as f64 * 0.01) - 4.0).abs() < 1e-12);
        }
    }

    #[test]
    fn eased_segments_match_keyframe_easing() {
        let c = Curve::new(&[
            (0.0, 0.0, Interp::Smooth),
            (1.0, 10.0, Interp::Eased(Easing::Linear)),
        ]);
        assert!((c.eval(0.25) - 2.5).abs() < 1e-9);
    }
}
