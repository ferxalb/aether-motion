//! Binds an `animation` declaration to its `@keyframes` and resolves frame values
//! (`transform`, `opacity`, `filter`, and `animation-timing-function`).
use crate::model::FilterChannel;
use crate::parse::animation::timing::parse_timing;
use crate::parse::animation::{AnimationDecl, AnimationSpec, CssFrame};
use crate::parse::css::vars::substitute;
use crate::parse::css::{RawFrame, Vars};
use crate::parse::values::filter::parse_filter;
use crate::parse::values::transform::parse_transform_with_percent;
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
                drop_shadow: None,
                easing: None,
            });
        }
        let current = frames.last_mut().unwrap();
        for (key, raw_value) in &frame.declarations {
            let value = substitute(raw_value, vars, 0)?;
            match key.as_str() {
                "transform" => current.transform = Some(parse_transform_with_percent(&value)?),
                "opacity" => {
                    let n: f32 = value.parse().context("invalid opacity")?;
                    if !n.is_finite() || !(0.0..=1.0).contains(&n) {
                        bail!("opacity is outside 0..1");
                    }
                    current.opacity = Some(n);
                }
                "animation-timing-function" => current.easing = Some(parse_timing(&value)?),
                "filter" | "-webkit-filter" => {
                    let (amounts, shadow) = keyframe_filter(&value)
                        .with_context(|| format!("@keyframes {name}: filter"))?;
                    current.filter = Some(amounts);
                    current.drop_shadow = shadow;
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

/// Amounts of a keyframe's `filter` list by channel, and its `drop-shadow()` as channels. `blur()`,
/// each color function (including `opacity()`), and `drop-shadow()` may appear once.
#[allow(clippy::type_complexity)]
fn keyframe_filter(
    value: &str,
) -> Result<(
    [Option<f32>; FilterChannel::COUNT],
    Option<crate::model::ShadowChannels>,
)> {
    let chain = parse_filter(value, "filter")?;
    let shadow = chain
        .drop_shadow
        .map(|shadow| crate::model::shadow_channels(Some(shadow)));
    let mut values = [None; FilterChannel::COUNT];
    if value.contains("blur(") {
        values[FilterChannel::Blur.index()] = Some(chain.blur);
    }
    for filter in chain.colors {
        let channel = FilterChannel::of(filter)
            .ok_or_else(|| anyhow!("internal: {filter:?} has no filter channel"))?;
        if values[channel.index()]
            .replace(FilterChannel::amount(filter))
            .is_some()
        {
            bail!("each filter function may appear once per keyframe: {value}");
        }
    }
    Ok((values, shadow))
}
