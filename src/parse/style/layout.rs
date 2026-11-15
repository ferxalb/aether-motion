//! Layout properties: offsets, size, positioning, flex container and item, gap, and padding.

use crate::model::Style;
use crate::parse::values::length::{parse_box, parse_length, spacing_px};
use anyhow::{anyhow, bail, Context, Result};

/// Applies a layout declaration to `style`; `Ok(false)` means the property is not a layout property.
pub(super) fn apply(key: &str, value: &str, style: &mut Style) -> Result<bool> {
    match key {
        "left" => style.left = Some(parse_length(value, key)?),
        "top" => style.top = Some(parse_length(value, key)?),
        "width" => style.width = Some(parse_length(value, key)?),
        "height" => style.height = Some(parse_length(value, key)?),
        "display" => {
            if value != "flex" && value != "block" {
                bail!("unsupported display: {value}");
            }
            style.display = Some(value.to_string());
        }
        "align-items" => {
            if !matches!(
                value,
                "flex-start" | "start" | "center" | "flex-end" | "end" | "stretch"
            ) {
                bail!("unsupported align-items: {value}");
            }
            style.align_items = Some(value.to_string());
        }
        "justify-content" => {
            if !matches!(
                value,
                "flex-start"
                    | "start"
                    | "center"
                    | "flex-end"
                    | "end"
                    | "space-between"
                    | "space-around"
                    | "space-evenly"
            ) {
                bail!("unsupported justify-content: {value}");
            }
            style.justify_content = Some(value.to_string());
        }
        "flex-direction" => {
            style.column = match value {
                "row" => false,
                "column" => true,
                _ => bail!("unsupported flex-direction (use row or column): {value}"),
            }
        }
        "gap" => style.gap = spacing_px(value, key)?,
        "padding" => style.padding = parse_box(value, key)?,
        "padding-top" => style.padding[0] = spacing_px(value, key)?,
        "padding-right" => style.padding[1] = spacing_px(value, key)?,
        "padding-bottom" => style.padding[2] = spacing_px(value, key)?,
        "padding-left" => style.padding[3] = spacing_px(value, key)?,
        "flex-grow" | "flex" => {
            let n: f32 = value
                .parse()
                .ok()
                .filter(|n: &f32| n.is_finite() && *n >= 0.0)
                .ok_or_else(|| anyhow!("{key}: expected a non-negative number: {value}"))?;
            style.flex_grow = n;
            style.flex_basis_zero = key == "flex";
        }
        "z-index" => style.z_index = Some(value.parse().context("invalid z-index")?),
        "position" => {
            if !matches!(value, "relative" | "absolute" | "static") {
                bail!("unsupported position: {value}");
            }
            style.position = Some(value.to_string());
        }
        _ => return Ok(false),
    }
    Ok(true)
}
