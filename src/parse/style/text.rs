//! Text properties: color, font size and family, and alignment.

use crate::model::{Length, Style};
use crate::parse::values::color::parse_color;
use crate::parse::values::length::parse_length;
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
        _ => return Ok(false),
    }
    Ok(true)
}
