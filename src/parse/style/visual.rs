//! Visual properties: background, border, radius, shadow, opacity, object-fit, and transform.

use crate::model::{Background, Length, Style};
use crate::parse::values::background::parse_background;
use crate::parse::values::color::parse_color;
use crate::parse::values::length::{parse_length, spacing_px};
use crate::parse::values::shadow::parse_box_shadows;
use crate::parse::values::tokens::split_top;
use crate::parse::values::transform::parse_transform;
use anyhow::{anyhow, bail, Context, Result};

/// `backdrop-filter`: only `none` or a single `blur(<px>)` (at most 200 px) is supported.
fn parse_backdrop_blur(value: &str, property: &str) -> Result<f32> {
    let value = value.trim();
    if value == "none" {
        return Ok(0.0);
    }
    let inner = value
        .strip_prefix("blur(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| anyhow!("unsupported {property} (only blur(<px>) or none): {value}"))?;
    let blur = spacing_px(inner, property)?;
    if blur > 200.0 {
        bail!("{property}: blur radius is limited to 200px");
    }
    Ok(blur)
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
            style.backdrop_blur = parse_backdrop_blur(value, key)?
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
