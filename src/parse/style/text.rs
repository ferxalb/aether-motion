//! Text properties: color, font size, family, weight, style, and kerning, alignment, wrapping, shadow,
//! fill color, and stroke.

use crate::model::{Length, LineHeight, Style, WhiteSpace};
use crate::parse::values::color::parse_color;
use crate::parse::values::length::parse_length;
use crate::parse::values::shadow::parse_text_shadows;
use crate::parse::values::tokens::split_top;
use anyhow::{bail, Result};

/// A text stroke width: non-negative px (`thin`, `medium`, and `thick` are 1, 3, and 5 px).
fn stroke_width(value: &str, key: &str) -> Result<f32> {
    let n = match value.trim() {
        "thin" => 1.0,
        "medium" => 3.0,
        "thick" => 5.0,
        other => match parse_length(other, key)? {
            Length::Px(n) => n,
            Length::Percent(_) => bail!("{key}: only px is supported: {value}"),
        },
    };
    if !(0.0..=200.0).contains(&n) {
        bail!("{key} must be between 0 and 200 px");
    }
    Ok(n)
}

/// Applies a text declaration to `style`; `Ok(false)` means the property is not a text property.
pub(super) fn apply(key: &str, value: &str, style: &mut Style) -> Result<bool> {
    match key {
        "color" => style.color = Some(parse_color(value)?),
        "font-size" => {
            let Length::Px(n) = parse_length(value, key)? else {
                bail!("font-size: only px is supported");
            };
            style.font_size = Some(n);
        }
        "font-weight" => {
            style.font_weight = Some(match value {
                "normal" => 400,
                "bold" => 700,
                _ => match value.parse::<u16>() {
                    Ok(n) if (1..=1000).contains(&n) => n,
                    _ => bail!("unsupported font-weight: {value}; use normal, bold, or 1-1000"),
                },
            });
        }
        "letter-spacing" => {
            style.letter_spacing = Some(match value {
                "normal" => 0.0,
                _ => match parse_length(value, key)? {
                    Length::Px(n) => n,
                    Length::Percent(_) => bail!("letter-spacing: only px is supported: {value}"),
                },
            });
        }
        "font-family" => style.font_family = Some(value.to_string()),
        "font-style" => {
            style.font_italic = Some(match value {
                "normal" => false,
                "italic" | "oblique" => true,
                _ => bail!("unsupported font-style: {value}; use normal, italic, or oblique"),
            });
        }
        "font-kerning" => {
            style.font_kerning = Some(match value {
                "normal" => true,
                "auto" | "none" => false,
                _ => bail!("unsupported font-kerning: {value}; use auto, normal, or none"),
            });
        }
        "text-align" => {
            if !matches!(value, "left" | "center" | "right") {
                bail!("unsupported text-align: {value}");
            }
            style.text_align = Some(value.to_string());
        }
        "text-shadow" => style.text_shadow = Some(parse_text_shadows(value)?),
        "line-height" => {
            style.line_height = Some(match value.parse::<f32>() {
                Ok(n) if n.is_finite() && n > 0.0 => LineHeight::Multiple(n),
                Ok(_) => bail!("line-height must be positive: {value}"),
                Err(_) => match parse_length(value, key) {
                    Ok(Length::Px(n)) if n > 0.0 => LineHeight::Px(n),
                    _ => bail!(
                        "unsupported line-height: {value}; use a positive number or a px value"
                    ),
                },
            });
        }
        "-webkit-text-fill-color" => style.text_fill_color = Some(parse_color(value)?),
        "-webkit-text-stroke" | "text-stroke" => {
            let parts: Vec<&str> = split_top(value, ' ');
            let (width, color) = match parts.as_slice() {
                ["none"] => ("0", None),
                [width] => (*width, None),
                [width, color] => (*width, Some(*color)),
                _ => bail!("{key}: use <width> [<color>]: {value}"),
            };
            style.text_stroke_width = Some(stroke_width(width, key)?);
            style.text_stroke_color = color.map(parse_color).transpose()?;
        }
        "-webkit-text-stroke-width" | "text-stroke-width" => {
            style.text_stroke_width = Some(stroke_width(value, key)?);
        }
        "-webkit-text-stroke-color" | "text-stroke-color" => {
            style.text_stroke_color = Some(parse_color(value)?);
        }
        "white-space" => {
            style.white_space = Some(match value {
                "normal" => WhiteSpace::Normal,
                "nowrap" => WhiteSpace::Nowrap,
                _ => bail!("unsupported white-space: {value}; use normal or nowrap"),
            });
        }
        _ => return Ok(false),
    }
    Ok(true)
}
