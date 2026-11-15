//! `data-hf-reveal`: uncovers an element from one edge, like a wipe, a growing bar, or a mask reveal.
//!
//! `data-hf-reveal="from-left|from-right|from-top|from-bottom"` with `data-hf-reveal-at` (seconds from
//! the clip start, default 0), `data-hf-reveal-duration` (default 0.8), and `data-hf-reveal-easing`
//! (default `ease-out-expo`). The element, its content, and its children are clipped to the revealed
//! part of its box until the reveal completes. It combines with other motion on the same element,
//! except rotation.
use crate::model::{Easing, Motion, RevealFrom, Track, TrackPoint};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::html::{attr, number};
use anyhow::{bail, Result};
use kuchiki::NodeRef;

#[derive(Clone, Debug)]
pub(in crate::parse) struct RevealSpec {
    from: RevealFrom,
    at: f64,
    duration: f64,
    easing: Easing,
}

pub(in crate::parse) fn parse_edge(raw: &str) -> Result<RevealFrom> {
    Ok(match raw.trim() {
        "from-left" => RevealFrom::Left,
        "from-right" => RevealFrom::Right,
        "from-top" => RevealFrom::Top,
        "from-bottom" => RevealFrom::Bottom,
        other => {
            bail!("data-hf-reveal must be from-left, from-right, from-top, or from-bottom: {other}")
        }
    })
}

/// Reveal track: 0 until `at`, growing to 1 over `duration` with `easing`.
pub(in crate::parse) fn reveal_track(at: f64, duration: f64, easing: Easing) -> Track {
    Track(vec![
        TrackPoint {
            at,
            value: 0.0,
            easing: Easing::Linear,
        },
        TrackPoint {
            at: at + duration,
            value: 1.0,
            easing,
        },
    ])
}

/// Reads the reveal attributes of `node`; `None` when `data-hf-reveal` is absent.
pub(in crate::parse) fn parse_reveal(node: &NodeRef) -> Result<Option<RevealSpec>> {
    let Some(raw) = attr(node, "data-hf-reveal") else {
        for name in [
            "data-hf-reveal-at",
            "data-hf-reveal-duration",
            "data-hf-reveal-easing",
        ] {
            if attr(node, name).is_some() {
                bail!("{name} requires data-hf-reveal");
            }
        }
        return Ok(None);
    };
    let from = parse_edge(&raw)?;
    let num = |name: &str, default: f64| -> Result<f64> {
        let value = attr(node, name)
            .map(|raw| number(&raw, name))
            .transpose()?
            .unwrap_or(default);
        if !value.is_finite() {
            bail!("{name} must be a finite number");
        }
        Ok(value)
    };
    let at = num("data-hf-reveal-at", 0.0)?;
    let duration = num("data-hf-reveal-duration", 0.8)?;
    if at < 0.0 {
        bail!("data-hf-reveal-at must not be negative");
    }
    if !(0.01..=60.0).contains(&duration) {
        bail!("data-hf-reveal-duration must be between 0.01 and 60 seconds");
    }
    let easing = match attr(node, "data-hf-reveal-easing") {
        Some(raw) => parse_keyframe_easing(&raw)?,
        None => parse_keyframe_easing("ease-out-expo")?,
    };
    Ok(Some(RevealSpec {
        from,
        at,
        duration,
        easing,
    }))
}

impl RevealSpec {
    /// End of the reveal in seconds from the clip start.
    pub(in crate::parse) fn end(&self) -> f64 {
        self.at + self.duration
    }

    /// Adds the reveal channel to `motion` without touching its other channels.
    pub(in crate::parse) fn apply(&self, motion: &mut Motion) {
        motion.wipe = Some(self.from);
        motion.reveal = reveal_track(self.at, self.duration, self.easing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_parse_and_reject_unknown_names() {
        assert_eq!(parse_edge("from-left").unwrap(), RevealFrom::Left);
        assert_eq!(parse_edge(" from-bottom ").unwrap(), RevealFrom::Bottom);
        assert!(parse_edge("left").is_err());
    }

    #[test]
    fn reveal_holds_zero_then_one() {
        let spec = RevealSpec {
            from: RevealFrom::Left,
            at: 1.0,
            duration: 2.0,
            easing: Easing::Linear,
        };
        let mut motion = Motion::default();
        spec.apply(&mut motion);
        assert_eq!(motion.pose(0.0).reveal, 0.0);
        assert!((motion.pose(2.0).reveal - 0.5).abs() < 1e-6);
        assert_eq!(motion.pose(3.0).reveal, 1.0);
        assert_eq!(motion.pose(9.0).reveal, 1.0);
        assert_eq!(spec.end(), 3.0);
    }
}
