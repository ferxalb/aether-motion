//! Binds an `animation` declaration to its `@keyframes` and resolves frame values
//! (`transform`, `opacity`, `filter`, and `animation-timing-function`).
use crate::model::FilterChannel;
use crate::parse::animation::timing::parse_timing;
use crate::parse::animation::{AnimationDecl, AnimationSpec, CssFrame};
use crate::parse::css::vars::substitute;
use crate::parse::css::{RawFrame, Vars};
use crate::parse::values::filter::parse_filter;
use crate::parse::values::transform::parse_transform;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;

/// Resolves `@keyframes` for one element: values (with `var()`), merge of equal offsets.
pub(in crate::parse) fn resolve_animation(
    decl: AnimationDecl,
    keyframes: &HashMap<String, Vec<RawFrame>>,
    vars: &Vars,
) -> Result<Option<AnimationSpec>> {
    let Some(name) = decl.name.clone() else {
        return Ok(None);
    };
    let raw = keyframes
        .get(&name)
        .ok_or_else(|| anyhow!("animation-name: @keyframes {name} is not defined"))?;
    let duration = decl
        .duration
        .ok_or_else(|| anyhow!("animation {name}: animation-duration is missing"))?;
    if duration < 0.001 {
        bail!("animation {name}: animation-duration must be at least 1ms");
    }
    let mut ordered: Vec<&RawFrame> = raw.iter().collect();
    ordered.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let mut frames: Vec<CssFrame> = Vec::new();
    for frame in ordered {
        if frames
            .last()
            .is_none_or(|last| (last.offset - frame.offset).abs() > 1e-9)
        {
            frames.push(CssFrame {
                offset: frame.offset,
                transform: None,
                opacity: None,
                filter: None,
                easing: None,
            });
        }
        let current = frames.last_mut().unwrap();
        for (key, raw_value) in &frame.declarations {
            let value = substitute(raw_value, vars, 0)?;
            match key.as_str() {
                "transform" => current.transform = Some(parse_transform(&value)?),
                "opacity" => {
                    let n: f32 = value.parse().context("invalid opacity")?;
                    if !n.is_finite() || !(0.0..=1.0).contains(&n) {
                        bail!("opacity is outside 0..1");
                    }
                    current.opacity = Some(n);
                }
                "animation-timing-function" => current.easing = Some(parse_timing(&value)?),
                "filter" | "-webkit-filter" => {
                    current.filter = Some(
                        keyframe_filter(&value)
                            .with_context(|| format!("@keyframes {name}: filter"))?,
                    )
                }
                _ => bail!(
                    "@keyframes {name}: unsupported property: {key} (only transform, opacity, filter, and animation-timing-function)"
                ),
            }
        }
    }
    Ok(Some(AnimationSpec {
        name,
        frames,
        decl,
        duration,
    }))
}

/// Amounts of a keyframe's `filter` list by channel. `blur()` and each color function may appear
/// once; `opacity()` and `drop-shadow()` are not animatable.
fn keyframe_filter(value: &str) -> Result<[Option<f32>; 8]> {
    let chain = parse_filter(value, "filter")?;
    if chain.drop_shadow.is_some() {
        bail!("drop-shadow() is not animatable; set it in the element's static filter");
    }
    let mut values = [None; 8];
    if value.contains("blur(") {
        values[FilterChannel::Blur.index()] = Some(chain.blur);
    }
    for filter in chain.colors {
        let channel = FilterChannel::of(filter)
            .ok_or_else(|| anyhow!("opacity() is not animatable; animate opacity instead"))?;
        if values[channel.index()]
            .replace(FilterChannel::amount(filter))
            .is_some()
        {
            bail!("each filter function may appear once per keyframe: {value}");
        }
    }
    Ok(values)
}
