//! `data-hf-keyframes` JSON attribute parsing.
use crate::model::{Easing, Keyframe};
use crate::parse::animation::timing::parse_keyframe_easing;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;

pub(in crate::parse) fn parse_keyframes(raw: &str) -> Result<Vec<Keyframe>> {
    let frames: Vec<BTreeMap<String, serde_json::Value>> =
        serde_json::from_str(raw).context("expected a JSON array of objects")?;
    if frames.is_empty() {
        bail!("at least one keyframe is required");
    }
    let mut previous = Keyframe {
        at: -1.0,
        x: 0.0,
        y: 0.0,
        opacity: 1.0,
        easing: Easing::Linear,
    };
    let mut keyframes = Vec::with_capacity(frames.len());
    for (index, frame) in frames.into_iter().enumerate() {
        let keyframe = (|| -> Result<Keyframe> {
            for key in frame.keys() {
                if !matches!(key.as_str(), "at" | "x" | "y" | "opacity" | "easing") {
                    bail!("unknown field: {key}");
                }
            }
            let get_number = |key: &str, fallback: f64| -> Result<f64> {
                match frame.get(key) {
                    Some(value) => value
                        .as_f64()
                        .ok_or_else(|| anyhow!("{key} must be a number")),
                    None => Ok(fallback),
                }
            };
            let at = get_number("at", -1.0)?;
            if at < 0.0 || at <= previous.at {
                bail!("at is required, and times must be non-negative and increasing");
            }
            let x = get_number("x", previous.x as f64)? as f32;
            let y = get_number("y", previous.y as f64)? as f32;
            if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
                bail!("x and y must be offsets of at most 1,000,000 px");
            }
            let opacity = get_number("opacity", previous.opacity as f64)?;
            if !(0.0..=1.0).contains(&opacity) {
                bail!("opacity must be between 0 and 1");
            }
            let easing = match frame.get("easing") {
                None => Easing::Linear,
                Some(value) => parse_keyframe_easing(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("easing must be a string"))?,
                )?,
            };
            Ok(Keyframe {
                at,
                x,
                y,
                opacity: opacity as f32,
                easing,
            })
        })()
        .with_context(|| format!("keyframe {}", index + 1))?;
        keyframes.push(keyframe);
        previous = keyframe;
    }
    Ok(keyframes)
}
