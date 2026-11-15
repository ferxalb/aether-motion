//! Text properties: color, font size and family, and alignment.

use crate::model::{Length, LineHeight, Style, WhiteSpace};
use crate::parse::values::color::parse_color;
use crate::parse::values::length::parse_length;
use crate::parse::values::shadow::parse_shadow;
use anyhow::{bail, Result};

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
        "font-family" => style.font_family = Some(value.to_string()),
        "text-align" => {
            if !matches!(value, "left" | "center" | "right") {
                bail!("unsupported text-align: {value}");
            }
            style.text_align = Some(value.to_string());
        }
        "text-shadow" => style.text_shadow = Some(parse_shadow(value, key)?),
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
