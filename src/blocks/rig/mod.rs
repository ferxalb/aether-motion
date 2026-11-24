//! Generated character blocks: controllable human rigs for 2D (`human-2d`) and native 3D
//! (`human-3d`).
//!
//! Unlike template blocks, a rig is computed: `skeleton` holds the shared proportions and pose
//! channels, `motion` the presets and the `timeline` parameter, `options` the validated parameters,
//! `place` the ground lock and stride matching, and `flat` and `solid` write the HTML. Limbs are
//! rigid segments joined on shared circles (spheres in 3D), so they never stretch or kink.

mod flat;
mod motion;
mod options;
mod place;
mod skeleton;
mod solid;

pub use skeleton::CHANNELS;

use anyhow::Result;
use std::collections::BTreeMap;

/// The generated rigs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rig {
    Human2d,
    Human3d,
}

impl Rig {
    pub fn description(self) -> &'static str {
        match self {
            Self::Human2d => flat::DESCRIPTION,
            Self::Human3d => solid::DESCRIPTION,
        }
    }

    /// Parameters as (name, default), in listing order.
    pub fn params(self) -> Vec<(String, String)> {
        options::params(match self {
            Self::Human2d => options::Flavor::Flat,
            Self::Human3d => options::Flavor::Solid,
        })
    }

    /// The block's HTML for `values` (unset parameters keep their defaults).
    pub fn render(self, values: &BTreeMap<String, String>) -> Result<String> {
        match self {
            Self::Human2d => flat::render(values),
            Self::Human3d => solid::render(values),
        }
    }
}

/// A number with at most three decimals and no trailing zeros (`-0` prints as `0`).
pub(crate) fn fmt(v: f64) -> String {
    let rounded = (v * 1000.0).round() / 1000.0;
    if rounded == 0.0 {
        return "0".to_string();
    }
    let text = format!("{rounded:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

/// Indices of the samples to keep so that linear interpolation between kept samples reproduces
/// every dropped one within `tolerance` on each channel. `series[i]` holds the channel values at
/// `times[i]`; the first and last samples are always kept.
pub(crate) fn keep(times: &[f64], series: &[Vec<f64>], tolerance: f64) -> Vec<usize> {
    let n = times.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let fits = |a: usize, b: usize| -> bool {
        (a + 1..b).all(|m| {
            let s = (times[m] - times[a]) / (times[b] - times[a]);
            series[m]
                .iter()
                .zip(&series[a])
                .zip(&series[b])
                .all(|((v, va), vb)| (va + (vb - va) * s - v).abs() <= tolerance)
        })
    };
    let mut kept = vec![0];
    let mut anchor = 0;
    let mut end = 2;
    while end < n {
        if !fits(anchor, end) {
            anchor = end - 1;
            kept.push(anchor);
        }
        end += 1;
    }
    kept.push(n - 1);
    kept
}

/// Output text of a generated block: a leading comment, then markup.
struct Out(String);

impl Out {
    fn new() -> Self {
        Self(String::new())
    }

    fn comment(&mut self, text: &str) {
        self.0.push_str("<!-- ");
        self.0.push_str(&text.replace("--", "-"));
        self.0.push_str(" -->\n");
    }

    fn push(&mut self, text: &str) {
        self.0.push_str(text);
    }

    fn finish(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers_compactly() {
        assert_eq!(fmt(1.0), "1");
        assert_eq!(fmt(-0.0001), "0");
        assert_eq!(fmt(12.3456), "12.346");
        assert_eq!(fmt(-2.5), "-2.5");
    }

    #[test]
    fn keeps_only_samples_that_bend_the_curve() {
        let times: Vec<f64> = (0..=10).map(f64::from).collect();
        let series: Vec<Vec<f64>> = times
            .iter()
            .map(|t| vec![if *t <= 5.0 { *t } else { 10.0 - t }])
            .collect();
        assert_eq!(keep(&times, &series, 1e-9), vec![0, 5, 10]);
    }

    #[test]
    fn rigs_render_with_defaults() {
        for rig in [Rig::Human2d, Rig::Human3d] {
            let html = rig.render(&BTreeMap::new()).unwrap();
            assert!(html.starts_with("<!--"));
            assert!(!rig.params().is_empty());
        }
    }
}
