//! Element effects outside CSS: `data-hf-bloom` (a localized glow of the element's bright pixels)
//! and `data-hf-motion-blur` (motion blur of one element and its subtree), plus the checks that tie
//! them to the rest of the composition.
//!
//! - `data-hf-bloom="<intensity>"` (0 to 8) with `data-hf-bloom-threshold` (luminance 0 to 1,
//!   default 0.55), `data-hf-bloom-radius` (px, 1 to 200, default 24), and `data-hf-bloom-tint`
//!   (any supported color, multiplying the halo). `data-hf-keyframes` `bloom` animates the intensity.
//! - `data-hf-motion-blur="on"` with `data-hf-motion-blur-shutter` (degrees, above 0 to 720, default
//!   180) and `data-hf-motion-blur-samples` (2 to 64; automatic when omitted), like the root's
//!   `data-motion-blur` but for one subtree.
//!
//! Per-element motion blur cannot be combined with the root's shutter (which already blurs every
//! element), nest inside another blurred element, or sit on the root.

use crate::model::{Bloom, Element, ElementEffects, Keyframe, MotionBlur};
use crate::parse::html::{attr, number};
use crate::parse::values::color::parse_color;
use anyhow::{bail, Result};
use kuchiki::NodeRef;

/// Reads `data-hf-bloom*` and `data-hf-motion-blur*` from `node`.
pub(in crate::parse) fn parse_effects(node: &NodeRef) -> Result<ElementEffects> {
    Ok(ElementEffects {
        bloom: parse_bloom(node)?,
        motion_blur: parse_layer_blur(node)?,
    })
}

fn optional(node: &NodeRef, name: &str) -> Result<Option<f64>> {
    let value = attr(node, name).map(|raw| number(&raw, name)).transpose()?;
    if value.is_some_and(|v| !v.is_finite()) {
        bail!("{name} must be a finite number");
    }
    Ok(value)
}

fn parse_bloom(node: &NodeRef) -> Result<Option<Bloom>> {
    let threshold = optional(node, "data-hf-bloom-threshold")?;
    let radius = optional(node, "data-hf-bloom-radius")?;
    let tint = attr(node, "data-hf-bloom-tint");
    let Some(raw) = attr(node, "data-hf-bloom") else {
        if threshold.is_some() || radius.is_some() || tint.is_some() {
            bail!("data-hf-bloom-threshold, -radius, and -tint need data-hf-bloom=\"<intensity>\" on the same element");
        }
        return Ok(None);
    };
    let intensity = match raw.trim() {
        "" | "on" => 1.0,
        other => number(other, "data-hf-bloom")?,
    };
    if !(0.0..=8.0).contains(&intensity) {
        bail!("data-hf-bloom must be an intensity from 0 to 8");
    }
    let threshold = threshold.unwrap_or(0.55);
    if !(0.0..1.0).contains(&threshold) {
        bail!("data-hf-bloom-threshold must be at least 0 and below 1");
    }
    let radius = radius.unwrap_or(24.0);
    if !(1.0..=200.0).contains(&radius) {
        bail!("data-hf-bloom-radius must be between 1 and 200 px");
    }
    let tint = tint.map(|raw| parse_color(raw.trim())).transpose()?;
    Ok(Some(Bloom {
        intensity: intensity as f32,
        threshold: threshold as f32,
        radius: radius as f32,
        tint,
    }))
}

fn parse_layer_blur(node: &NodeRef) -> Result<Option<MotionBlur>> {
    let shutter = optional(node, "data-hf-motion-blur-shutter")?;
    let samples = optional(node, "data-hf-motion-blur-samples")?;
    match attr(node, "data-hf-motion-blur").as_deref().map(str::trim) {
        None | Some("off") => {
            if shutter.is_some() || samples.is_some() {
                bail!("data-hf-motion-blur-shutter and -samples need data-hf-motion-blur=\"on\"");
            }
            return Ok(None);
        }
        Some("on" | "") => {}
        Some(other) => bail!("data-hf-motion-blur must be on or off, not {other}"),
    }
    let shutter = shutter.unwrap_or(180.0);
    if !(shutter > 0.0 && shutter <= 720.0) {
        bail!("data-hf-motion-blur-shutter must be above 0 and at most 720 degrees");
    }
    if samples.is_some_and(|n| n.fract() != 0.0 || !(2.0..=64.0).contains(&n)) {
        bail!("data-hf-motion-blur-samples must be a whole number from 2 to 64");
    }
    Ok(Some(MotionBlur {
        shutter: shutter as f32,
        samples: samples.map(|n| n as u32),
    }))
}

/// Checks one element's effects once its motion is built: a keyframed `bloom` needs
/// `data-hf-bloom` (which sets threshold, radius, and tint), and starts from its intensity until the
/// first keyframe that names `bloom`.
pub(in crate::parse) fn finish_effects(
    element: &mut Element,
    keyframes: &[Keyframe],
    root: bool,
) -> Result<()> {
    if root && element.effects.bloom.is_some() {
        bail!("data-hf-bloom cannot be on the root; add a container inside it");
    }
    if root && element.effects.motion_blur.is_some() {
        bail!("data-hf-motion-blur cannot be on the root; use data-motion-blur on the root to blur the whole frame");
    }
    let track = &mut element.motion.fx.bloom;
    if track.0.is_empty() {
        return Ok(());
    }
    let Some(bloom) = element.effects.bloom else {
        bail!("keyframe bloom needs data-hf-bloom on the same element (it sets the threshold, radius, and tint)");
    };
    let leading = keyframes
        .iter()
        .take_while(|k| k.effects[2].is_none())
        .count();
    for point in track.0.iter_mut().take(leading) {
        point.value = bloom.intensity;
    }
    Ok(())
}

/// Per-element motion blur averages one subtree over the shutter window: the root's shutter would
/// average it again, and a blurred element inside another would be sampled twice.
pub(in crate::parse) fn check_layer_blur(
    elements: &[Element],
    global: Option<MotionBlur>,
) -> Result<()> {
    let name = |e: &Element| {
        e.id.as_ref()
            .map_or_else(|| format!("<{}>", e.tag), |id| format!("#{id}"))
    };
    for element in elements.iter().filter(|e| e.effects.motion_blur.is_some()) {
        if global.is_some() {
            bail!(
                "data-hf-motion-blur on {} cannot be combined with data-motion-blur on the root, which already blurs every element; remove one of them",
                name(element)
            );
        }
        let mut parent = element.parent;
        while let Some(idx) = parent {
            if elements[idx].effects.motion_blur.is_some() {
                bail!(
                    "data-hf-motion-blur on {} is inside {}, which is already motion-blurred; keep it on one of them",
                    name(element),
                    name(&elements[idx])
                );
            }
            parent = elements[idx].parent;
        }
    }
    Ok(())
}
