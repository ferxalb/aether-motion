//! `data-hf-speed` (constant rate) and `data-hf-speed-ramp` (JSON keyframes) on `<video>` and
//! `<audio>`, parsed into a shared `speed::SpeedCurve`.
//!
//! A ramp is `[{"at": 0, "speed": 1}, {"at": 1.2, "speed": 0.25, "easing": "ease-in-out"}]`:
//! `at` is seconds from the clip start (strictly increasing), `speed` is the rate reached there
//! (0 to 16; omitted values inherit the previous keyframe, 1 at first), and `easing` (the
//! `data-hf-keyframes` names) shapes the segment that ends at that keyframe.
use crate::model::Easing;
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::html::{attr, number};
use crate::speed::{SpeedCurve, SpeedKey};
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The element's speed curve, or `None` when it plays at 1x.
pub(in crate::parse) fn parse_speed(node: &NodeRef) -> Result<Option<Arc<SpeedCurve>>> {
    let constant = attr(node, "data-hf-speed");
    let ramp = attr(node, "data-hf-speed-ramp");
    let curve = match (constant, ramp) {
        (Some(_), Some(_)) => bail!("use either data-hf-speed or data-hf-speed-ramp, not both"),
        (Some(raw), None) => {
            let speed = number(&raw, "data-hf-speed")?;
            if speed == 1.0 {
                return Ok(None);
            }
            SpeedCurve::constant(speed)?
        }
        (None, Some(raw)) => parse_ramp(&raw).context("data-hf-speed-ramp")?,
        (None, None) => return Ok(None),
    };
    Ok(Some(Arc::new(curve)))
}

fn parse_ramp(raw: &str) -> Result<SpeedCurve> {
    let frames: Vec<BTreeMap<String, serde_json::Value>> = serde_json::from_str(raw).context(
        "expected a JSON array of objects like [{\"at\":0,\"speed\":1},{\"at\":1,\"speed\":0.25}]",
    )?;
    let mut previous = 1.0;
    let mut keys = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        let key = (|| -> Result<SpeedKey> {
            if let Some(key) = frame
                .keys()
                .find(|k| !matches!(k.as_str(), "at" | "speed" | "easing"))
            {
                bail!("unknown field: {key}");
            }
            let at = frame
                .get("at")
                .ok_or_else(|| anyhow!("at is required"))?
                .as_f64()
                .ok_or_else(|| anyhow!("at must be a number"))?;
            let speed = match frame.get("speed") {
                Some(value) => value
                    .as_f64()
                    .ok_or_else(|| anyhow!("speed must be a number"))?,
                None => previous,
            };
            let easing = match frame.get("easing") {
                None => Easing::Linear,
                Some(value) => parse_keyframe_easing(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("easing must be a string"))?,
                )?,
            };
            Ok(SpeedKey { at, speed, easing })
        })()
        .with_context(|| format!("keyframe {index}"))?;
        previous = key.speed;
        keys.push(key);
    }
    SpeedCurve::ramp(keys)
}
