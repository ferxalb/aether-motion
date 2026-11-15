//! Binds an `animation` declaration to its `@keyframes` and resolves frame values.
use crate::parse::animation::timing::parse_timing;
use crate::parse::animation::{AnimationDecl, AnimationSpec, CssFrame};
use crate::parse::css::vars::substitute;
use crate::parse::css::{RawFrame, Vars};
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
                _ => bail!(
                    "@keyframes {name}: unsupported property: {key} (only transform, opacity and animation-timing-function)"
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
