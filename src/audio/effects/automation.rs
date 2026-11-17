//! Volume automation (`data-hf-volume-ramp`): a gain curve over clip time.
//!
//! Each keyframe sets the linear gain reached at `at` seconds after the clip start; the segment
//! that ends at a keyframe interpolates the gain with that keyframe's easing (the
//! `data-hf-keyframes` names), and the first and last gains hold outside the keyed interval.
//! The curve multiplies `data-volume` and the fades, so it never replaces them. The mixer reads it
//! at control rate (`effects::BLOCK`), interpolating linearly between block edges.
use crate::model::Easing;
use anyhow::{bail, Result};

/// Largest automation gain (about +18 dB), matching `data-volume`.
pub const MAX_GAIN: f64 = 8.0;
/// Latest keyframe time, in seconds.
pub const MAX_SECONDS: f64 = 3600.0;
/// Most keyframes in one ramp.
pub const MAX_KEYS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeKey {
    pub at: f64,
    /// Linear gain reached at `at`.
    pub gain: f64,
    /// Easing of the segment that ends at this keyframe.
    pub easing: Easing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolumeCurve {
    keys: Vec<VolumeKey>,
}

impl VolumeCurve {
    pub fn new(keys: Vec<VolumeKey>) -> Result<Self> {
        if keys.is_empty() || keys.len() > MAX_KEYS {
            bail!("needs 1 to {MAX_KEYS} keyframes");
        }
        let mut previous = None;
        for (index, key) in keys.iter().enumerate() {
            if !(0.0..=MAX_SECONDS).contains(&key.at) {
                bail!("keyframe {index}: at must be between 0 and {MAX_SECONDS}");
            }
            if previous.is_some_and(|at| key.at <= at) {
                bail!("keyframe {index}: at must be strictly increasing");
            }
            if !(0.0..=MAX_GAIN).contains(&key.gain) {
                bail!("keyframe {index}: volume must be between 0 and {MAX_GAIN} (db at most +18)");
            }
            previous = Some(key.at);
        }
        Ok(Self { keys })
    }

    pub fn keys(&self) -> &[VolumeKey] {
        &self.keys
    }

    /// Gain at `t` seconds after the clip start.
    pub fn gain(&self, t: f64) -> f64 {
        let next = self.keys.partition_point(|k| k.at <= t);
        if next == 0 {
            return self.keys[0].gain;
        }
        if next == self.keys.len() {
            return self.keys[next - 1].gain;
        }
        let (a, b) = (self.keys[next - 1], self.keys[next]);
        if a.gain == b.gain {
            return a.gain;
        }
        let u = ((t - a.at) / (b.at - a.at)).clamp(0.0, 1.0);
        let eased = f64::from(b.easing.apply(u as f32));
        (a.gain + (b.gain - a.gain) * eased).clamp(0.0, MAX_GAIN)
    }

    /// `[{"at", "volume", "db"}]` for `check`.
    pub fn to_json(&self) -> serde_json::Value {
        self.keys
            .iter()
            .map(
                |k| serde_json::json!({"at": k.at, "volume": k.gain, "db": super::gain_db(k.gain)}),
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(at: f64, gain: f64) -> VolumeKey {
        VolumeKey {
            at,
            gain,
            easing: Easing::Linear,
        }
    }

    #[test]
    fn interpolates_and_holds_the_edges() {
        let curve = VolumeCurve::new(vec![key(1.0, 1.0), key(3.0, 0.2)]).unwrap();
        assert_eq!(curve.gain(0.0), 1.0);
        assert!((curve.gain(2.0) - 0.6).abs() < 1e-6);
        assert_eq!(curve.gain(3.0), 0.2);
        assert_eq!(curve.gain(9.0), 0.2);
    }

    #[test]
    fn rejects_unordered_or_loud_keys() {
        assert!(VolumeCurve::new(vec![key(1.0, 1.0), key(1.0, 0.5)]).is_err());
        assert!(VolumeCurve::new(vec![key(0.0, 9.0)]).is_err());
        assert!(VolumeCurve::new(Vec::new()).is_err());
    }
}
