//! Visual properties: background, border, radius, shadow, opacity, filters, blend mode, clip-path,
//! mask-image, object-fit, and transform.

use crate::model::{Background, BlendMode, Length, Style};
use crate::parse::values::background::parse_background;
use crate::parse::values::clip::{parse_clip_path, parse_mask_image};
use crate::parse::values::color::parse_color;
use crate::parse::values::filter::{parse_backdrop_filter, parse_filter};
use crate::parse::values::length::{parse_length, spacing_px};
use crate::parse::values::shadow::parse_box_shadows;
use crate::parse::values::tokens::split_top;
use crate::parse::values::transform::parse_transform;
use anyhow::{bail, Context, Result};

/// `mix-blend-mode` keywords; the non-separable modes (`hue`, `color`, ...) are rejected.
fn parse_blend_mode(value: &str) -> Result<BlendMode> {
    Ok(match value.trim() {
        "normal" => BlendMode::Normal,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "plus-lighter" => BlendMode::PlusLighter,
        other => bail!("unsupported mix-blend-mode: {other} (hue, saturation, color, and luminosity are not supported)"),
    })
}

/// Applies a visual declaration to `style`; `Ok(false)` means the property is not a visual property.
pub(super) fn apply(key: &str, value: &str, style: &mut Style) -> Result<bool> {
    match key {
        "background" => style.background = Some(parse_background(value)?),
        "background-color" => {
            let color = parse_color(value)?;
            // The supported gradients are opaque and cover the background color.
            if !matches!(
                style.background,
                Some(Background::Linear { .. } | Background::Gradient(_))
            ) {
                style.background = Some(Background::Solid(color));
            }
        }
        "border-radius" => match parse_length(value, key)? {
            Length::Px(n) if n >= 0.0 => style.border_radius = n,
            _ => bail!("border-radius: only one non-negative radius in px is supported"),
        },
        "opacity" => {
            let n: f32 = value.parse().context("invalid opacity")?;
            if !n.is_finite() || !(0.0..=1.0).contains(&n) {
                bail!("opacity is outside 0..1");
            }
            style.opacity = Some(n);
        }
        "box-shadow" => style.shadows = parse_box_shadows(value)?,
        "backdrop-filter" | "-webkit-backdrop-filter" => {
            style.backdrop_blur = parse_backdrop_filter(value, key)?
        }
        "filter" | "-webkit-filter" => {
            let chain = parse_filter(value, key)?;
            style.filter_blur = chain.blur;
            style.filter_colors = chain.colors;
            style.drop_shadow = chain.drop_shadow;
        }
        "mix-blend-mode" => style.blend = parse_blend_mode(value)?,
        "clip-path" | "-webkit-clip-path" => style.clip_path = parse_clip_path(value)?,
        "mask-image" | "-webkit-mask-image" => style.mask = parse_mask_image(value)?,
        "mask" | "-webkit-mask" => {
            bail!("{key}: use mask-image with a linear-gradient() or radial-gradient() (the mask shorthand is not supported)")
        }
        "border" => {
            (style.border_width, style.border_color) = (0.0, None);
            for token in split_top(value, ' ') {
                match token {
                    "solid" => {}
                    "none" => style.border_width = 0.0,
                    _ if token.starts_with(|c: char| c.is_ascii_digit() || c == '.') => {
                        style.border_width = spacing_px(token, key)?
                    }
                    _ => style.border_color = Some(parse_color(token)?),
                }
            }
        }
        "border-width" => style.border_width = spacing_px(value, key)?,
        "border-color" => style.border_color = Some(parse_color(value)?),
        "border-style" if matches!(value, "solid" | "none") => {
            if value == "none" {
                style.border_width = 0.0;
            }
        }
        "object-fit" => {
            if !matches!(value, "fill" | "contain" | "cover") {
                bail!("unsupported object-fit: {value}");
            }
            style.object_fit = Some(value.to_string());
        }
        "transform" => {
            let (tx, ty, scale, rotate) = parse_transform(value)?;
            style.rotate = rotate;
            style.translate = (tx, ty);
            style.scale = Some(scale);
        }
        "transform-origin" if matches!(value, "center" | "center center" | "50% 50%") => {}
        _ => return Ok(false),
    }
    Ok(true)
}
