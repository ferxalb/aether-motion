//! Master processing of the finished mix, both opt-in on the composition root:
//! - a look-ahead peak limiter (`data-hf-audio-limiter="<ceiling dBFS>"`) that keeps every sample
//!   at or under the ceiling with a gain that starts falling `LOOKAHEAD` before a peak and recovers
//!   over `RELEASE`, so there is no clipping and no click;
//! - TPDF dither (`data-hf-audio-dither="tpdf"`) for the 16-bit conversion, from a counter-based
//!   hash of the sample index, so it is deterministic and independent of how the mix is split.
//!
//! The limiter's gain is stereo-linked. Because the whole mix is known in advance, the gain at a
//! sample is the average over the previous `LOOKAHEAD` samples of the smallest gain required in the
//! `LOOKAHEAD` samples after each of them; every one of those windows contains the sample, so the
//! average never exceeds the gain it needs. The release then limits how fast the gain rises.

use super::SAMPLE_RATE;
use std::collections::VecDeque;

/// Seconds the limiter looks ahead (and its attack ramp).
pub const LOOKAHEAD: f64 = 0.005;
/// Seconds over which the gain recovers by a factor of e after a peak.
pub const RELEASE: f64 = 0.1;
/// Accepted ceilings, in dBFS.
pub const MIN_CEILING_DB: f64 = -24.0;
pub const MAX_CEILING_DB: f64 = 0.0;

/// Master settings from the composition root.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Master {
    /// Limiter ceiling in dBFS; `None` is no limiter.
    pub limiter_db: Option<f64>,
    /// TPDF dither on the 16-bit conversion.
    pub dither: bool,
}

/// What the limiter did, for `check`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimiterReport {
    pub ceiling_db: f64,
    /// Deepest gain reduction, in dB (0 when nothing reached the ceiling).
    pub max_reduction_db: f64,
    /// Samples (per channel) whose gain was reduced.
    pub limited_samples: usize,
}

/// Limits `left` and `right` in place to `ceiling_db`.
pub fn limit(left: &mut [f32], right: &mut [f32], ceiling_db: f64) -> LimiterReport {
    let ceiling = 10f64.powf(ceiling_db / 20.0);
    let n = left.len().min(right.len());
    let lookahead = ((LOOKAHEAD * f64::from(SAMPLE_RATE)).round() as usize).max(1);
    // The gain each sample needs on its own.
    let needed: Vec<f64> = (0..n)
        .map(|i| {
            let peak = f64::from(left[i].abs().max(right[i].abs()));
            if peak > ceiling {
                ceiling / peak
            } else {
                1.0
            }
        })
        .collect();
    // Smallest needed gain over `i..=i + lookahead` (a monotonic deque, scanned backwards).
    let mut hold = vec![1.0_f64; n];
    let mut window: VecDeque<usize> = VecDeque::new();
    for i in (0..n).rev() {
        while window.back().is_some_and(|&j| needed[j] >= needed[i]) {
            window.pop_back();
        }
        window.push_back(i);
        while window.front().is_some_and(|&j| j > i + lookahead) {
            window.pop_front();
        }
        hold[i] = needed[window[0]];
    }
    // Average of `hold` over the `lookahead` samples ending at `i` (missing ones before the start
    // count as 1), then a release that only slows the rise.
    let release = 1.0 - (-1.0 / (RELEASE * f64::from(SAMPLE_RATE))).exp();
    let mut sum = lookahead as f64;
    let mut gain = 1.0_f64;
    let mut report = LimiterReport {
        ceiling_db,
        max_reduction_db: 0.0,
        limited_samples: 0,
    };
    let mut lowest = 1.0_f64;
    for i in 0..n {
        sum += hold[i]
            - if i >= lookahead {
                hold[i - lookahead]
            } else {
                1.0
            };
        let smooth = (sum / lookahead as f64).min(needed[i]);
        gain = smooth.min(gain + (1.0 - gain) * release);
        if gain < 1.0 {
            report.limited_samples += 1;
            lowest = lowest.min(gain);
            left[i] = (f64::from(left[i]) * gain) as f32;
            right[i] = (f64::from(right[i]) * gain) as f32;
        }
    }
    report.max_reduction_db = -20.0 * lowest.log10();
    report
}

/// TPDF dither for sample `index` of `channel`, in 16-bit steps (-1 to 1, triangular).
pub fn dither(index: usize, channel: usize) -> f32 {
    let uniform = |stream: u64| {
        let mut z = (index as u64)
            .wrapping_mul(4)
            .wrapping_add(channel as u64 * 2 + stream)
            .wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    };
    uniform(0) + uniform(1) - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_keeps_every_sample_under_the_ceiling_without_steps() {
        // A quiet tone with one loud burst in the middle.
        let tone = |i: usize| (i as f32 * 0.05).sin();
        let mut left: Vec<f32> = (0..9600)
            .map(|i| tone(i) * if (4000..4400).contains(&i) { 1.8 } else { 0.4 })
            .collect();
        let mut right = left.clone();
        let original = left.clone();
        let report = limit(&mut left, &mut right, -1.0);
        let ceiling = 10f32.powf(-1.0 / 20.0);
        assert!(left.iter().all(|v| v.abs() <= ceiling + 1e-6));
        assert!(report.max_reduction_db > 5.0, "{report:?}");
        // Far from the burst the signal is untouched.
        assert_eq!(left[..3000], original[..3000]);
        // The gain changes gradually: no sample moves by more than the tone's own slope allows.
        let gain = |i: usize| left[i] / original[i];
        for i in 3700..4000 {
            if original[i].abs() > 0.1 && original[i + 1].abs() > 0.1 {
                assert!((gain(i) - gain(i + 1)).abs() < 0.01, "step at {i}");
            }
        }
        assert_eq!(left, right, "stereo-linked");
    }

    #[test]
    fn quiet_mixes_pass_through() {
        let mut left = vec![0.5_f32; 1000];
        let mut right = vec![-0.5_f32; 1000];
        let report = limit(&mut left, &mut right, -1.0);
        assert_eq!(report.limited_samples, 0);
        assert_eq!(report.max_reduction_db, 0.0);
        assert!(left.iter().all(|&v| v == 0.5));
    }

    #[test]
    fn dither_is_triangular_and_repeatable() {
        let values: Vec<f32> = (0..200_000).map(|i| dither(i, 0)).collect();
        assert!(values.iter().all(|v| (-1.0..1.0).contains(v)));
        let mean = values.iter().map(|&v| f64::from(v)).sum::<f64>() / values.len() as f64;
        let variance =
            values.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / values.len() as f64;
        // Triangular on (-1, 1): mean 0, variance 1/6.
        assert!(mean.abs() < 0.01, "{mean}");
        assert!((variance - 1.0 / 6.0).abs() < 0.01, "{variance}");
        assert_eq!(dither(12345, 1), dither(12345, 1));
        assert_ne!(dither(12345, 0), dither(12345, 1));
    }
}
