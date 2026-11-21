//! `data-hf-keyframes` JSON attribute parsing: offsets, opacity, easing, and filter amounts
//! (`blur`, `brightness`, `contrast`, `saturate`, `grayscale`, `sepia`, `hueRotate`, `invert`), and
//! the `glitch` (0 to 1) and `rgbSplit` (px) pixel effects, the `data-hf-bloom` intensity
//! (`bloom`, 0 to 8), `clipPath` shapes, and the 3D values of planes in a `data-hf-space` (`z` in
//! px, `rotateX` and `rotateY` in degrees).
use crate::model::{Easing, FilterChannel, Keyframe};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::values::clip::parse_clip_path;
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
        filters: [None; 8],
        effects: [None; 3],
        clip: None,
        depth: [None; 3],
    };
    let mut keyframes = Vec::with_capacity(frames.len());
    for (index, frame) in frames.into_iter().enumerate() {
        let keyframe = (|| -> Result<Keyframe> {
            for key in frame.keys() {
                let filter = FilterChannel::ALL.iter().any(|c| c.key() == key);
                if !filter && !matches!(
                        key.as_str(),
                        "at" | "x" | "y" | "opacity" | "easing" | "glitch" | "rgbSplit" | "bloom" | "clipPath"
                        | "z" | "rotateX" | "rotateY"
                    ) {
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
            let mut filters = [None; 8];
            for channel in FilterChannel::ALL {
                let Some(value) = frame.get(channel.key()) else {
                    continue;
                };
                let key = channel.key();
                let n = value
                    .as_f64()
                    .ok_or_else(|| anyhow!("{key} must be a number"))?;
                let valid = match channel {
                    FilterChannel::Blur => (0.0..=200.0).contains(&n),
                    FilterChannel::Grayscale | FilterChannel::Sepia | FilterChannel::Invert => {
                        (0.0..=1.0).contains(&n)
                    }
                    FilterChannel::HueRotate => n.abs() <= 36_000.0,
                    _ => (0.0..=100.0).contains(&n),
                };
                if !valid {
                    bail!(
                        "{key} is out of range (blur 0 to 200 px; grayscale, sepia, invert 0 to 1; brightness, contrast, saturate 0 to 100; hueRotate in degrees up to 36000)"
                    );
                }
                filters[channel.index()] = Some(n as f32);
            }
            let mut effects = [None; 3];
            for (slot, (key, range)) in effects.iter_mut().zip([
                ("glitch", 0.0..=1.0),
                ("rgbSplit", -500.0..=500.0),
                ("bloom", 0.0..=8.0),
            ])
            {
                if let Some(value) = frame.get(key) {
                    let n = value
                        .as_f64()
                        .ok_or_else(|| anyhow!("{key} must be a number"))?;
                    if !range.contains(&n) {
                        bail!("{key} is out of range (glitch 0 to 1; rgbSplit -500 to 500 px; bloom 0 to 8)");
                    }
                    *slot = Some(n as f32);
                }
            }
            let mut depth = [None; 3];
            for (slot, key) in depth.iter_mut().zip(["z", "rotateX", "rotateY"]) {
                if let Some(value) = frame.get(key) {
                    let n = value
                        .as_f64()
                        .filter(|n| n.abs() <= crate::parse::space::MAX_DEPTH_VALUE)
                        .ok_or_else(|| {
                            anyhow!("{key} must be a number of at most 100,000 (px or degrees)")
                        })?;
                    *slot = Some(n as f32);
                }
            }
            let clip = match frame.get("clipPath") {
                None => None,
                Some(value) => {
                    let raw = value
                        .as_str()
                        .ok_or_else(|| anyhow!("clipPath must be a string such as \"circle(40% at 50% 50%)\""))?;
                    Some(
                        parse_clip_path(raw)?
                            .ok_or_else(|| anyhow!("clipPath cannot be none in a keyframe"))?,
                    )
                }
            };
            Ok(Keyframe {
                at,
                x,
                y,
                opacity: opacity as f32,
                easing,
                filters,
                effects,
                clip,
                depth,
            })
        })()
        .with_context(|| format!("keyframe {}", index + 1))?;
        previous = keyframe.clone();
        keyframes.push(keyframe);
    }
    Ok(keyframes)
}
