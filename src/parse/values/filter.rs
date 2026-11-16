//! `filter` and `backdrop-filter` parsing.
//!
//! `filter` is a list of functions in a fixed order: an optional `blur()`, then any color functions
//! (`brightness`, `contrast`, `saturate`, `grayscale`, `sepia`, `hue-rotate`, `invert`, `opacity`),
//! then an optional `drop-shadow()`. Other orders are rejected because the renderer applies the
//! three stages in that sequence. `backdrop-filter` accepts only `blur()`.

use crate::model::{ColorFilter, Shadow};
use crate::parse::values::length::spacing_px;
use crate::parse::values::shadow::parse_shadow;
use crate::parse::values::tokens::split_top;
use anyhow::{anyhow, bail, Context, Result};

/// Parsed `filter` value: the three stages in the order they are applied.
#[derive(Debug, Default, PartialEq)]
pub(in crate::parse) struct FilterChain {
    pub(in crate::parse) blur: f32,
    pub(in crate::parse) colors: Vec<ColorFilter>,
    pub(in crate::parse) drop_shadow: Option<Shadow>,
}

/// Largest blur radius (px) accepted by `filter`, `backdrop-filter`, and `drop-shadow`.
const MAX_BLUR: f32 = 200.0;

/// Splits `name(args)` into its lower-case name and argument text.
fn function(token: &str) -> Option<(String, &str)> {
    let open = token.find('(')?;
    let args = token[open + 1..].strip_suffix(')')?;
    Some((token[..open].trim().to_ascii_lowercase(), args.trim()))
}

fn blur_radius(args: &str, property: &str) -> Result<f32> {
    let blur = if args.is_empty() {
        0.0
    } else {
        spacing_px(args, property)?
    };
    if blur > MAX_BLUR {
        bail!("{property}: blur radius is limited to 200px");
    }
    Ok(blur)
}

/// `backdrop-filter`: only `none` or a single `blur(<px>)`.
pub(in crate::parse) fn parse_backdrop_filter(value: &str, property: &str) -> Result<f32> {
    let value = value.trim();
    if value == "none" {
        return Ok(0.0);
    }
    match function(value) {
        Some((name, args)) if name == "blur" && split_top(value, ' ').len() == 1 => {
            blur_radius(args, property)
        }
        _ => bail!("unsupported {property} (only blur(<px>) or none): {value}"),
    }
}

/// A `<number>` or `<percentage>` argument (`1.2`, `120%`); empty means `default`.
fn amount(args: &str, name: &str, default: f32) -> Result<f32> {
    if args.is_empty() {
        return Ok(default);
    }
    let (number, divisor) = match args.strip_suffix('%') {
        Some(n) => (n, 100.0),
        None => (args, 1.0),
    };
    let n: f32 = number
        .trim()
        .parse()
        .with_context(|| format!("filter: invalid {name}() amount: {args}"))?;
    let n = n / divisor;
    if !n.is_finite() || n < 0.0 {
        bail!("filter: {name}() needs a non-negative amount: {args}");
    }
    Ok(n)
}

/// A `hue-rotate()` angle in degrees (`deg`, `turn`, `rad`, `grad`, or a bare `0`).
fn angle(args: &str) -> Result<f32> {
    if args.is_empty() || args == "0" {
        return Ok(0.0);
    }
    let split = args
        .find(|c: char| c.is_ascii_alphabetic())
        .ok_or_else(|| anyhow!("filter: hue-rotate() needs an angle unit: {args}"))?;
    let (number, unit) = args.split_at(split);
    let n: f32 = number
        .trim()
        .parse()
        .with_context(|| format!("filter: invalid hue-rotate() angle: {args}"))?;
    let degrees = match unit {
        "deg" => n,
        "turn" => n * 360.0,
        "rad" => n.to_degrees(),
        "grad" => n * 0.9,
        _ => bail!("filter: invalid hue-rotate() angle unit: {args}"),
    };
    if !degrees.is_finite() {
        bail!("filter: hue-rotate() angle is not finite");
    }
    Ok(degrees)
}

/// Parses a `filter` value into its blur, color, and drop-shadow stages.
pub(in crate::parse) fn parse_filter(value: &str, property: &str) -> Result<FilterChain> {
    let value = value.trim();
    let mut chain = FilterChain::default();
    if value == "none" {
        return Ok(chain);
    }
    // 0 = nothing yet, 1 = after blur, 2 = after a color function, 3 = after drop-shadow.
    let mut stage = 0;
    for token in split_top(value, ' ') {
        let (name, args) = function(token)
            .ok_or_else(|| anyhow!("{property}: expected filter functions: {value}"))?;
        let color = match name.as_str() {
            "blur" => {
                if stage > 0 {
                    bail!("{property}: blur() must come first and appear once: {value}");
                }
                chain.blur = blur_radius(args, property)?;
                stage = 1;
                continue;
            }
            "drop-shadow" => {
                if stage == 3 {
                    bail!("{property}: only one drop-shadow() is supported: {value}");
                }
                let shadow = parse_shadow(args, "drop-shadow")?
                    .ok_or_else(|| anyhow!("{property}: drop-shadow() needs a shadow"))?;
                if shadow.blur > MAX_BLUR {
                    bail!("{property}: drop-shadow blur is limited to 200px");
                }
                chain.drop_shadow = Some(shadow);
                stage = 3;
                continue;
            }
            "brightness" => ColorFilter::Brightness(amount(args, &name, 1.0)?),
            "contrast" => ColorFilter::Contrast(amount(args, &name, 1.0)?),
            "saturate" => ColorFilter::Saturate(amount(args, &name, 1.0)?),
            "grayscale" => ColorFilter::Grayscale(amount(args, &name, 1.0)?.min(1.0)),
            "sepia" => ColorFilter::Sepia(amount(args, &name, 1.0)?.min(1.0)),
            "invert" => ColorFilter::Invert(amount(args, &name, 1.0)?.min(1.0)),
            "opacity" => ColorFilter::Opacity(amount(args, &name, 1.0)?.min(1.0)),
            "hue-rotate" => ColorFilter::HueRotate(angle(args)?),
            "url" => bail!("{property}: url() filters are unsupported"),
            _ => bail!("{property}: unsupported filter function {name}()"),
        };
        if stage == 3 {
            bail!("{property}: drop-shadow() must be the last function: {value}");
        }
        chain.colors.push(color);
        stage = 2;
    }
    if chain.colors.len() > 16 {
        bail!("{property}: at most 16 color functions are supported");
    }
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stages_in_order() {
        let chain = parse_filter(
            "blur(4px) brightness(120%) hue-rotate(0.5turn) drop-shadow(0 8px 12px #0008)",
            "filter",
        )
        .unwrap();
        assert_eq!(chain.blur, 4.0);
        assert_eq!(
            chain.colors,
            vec![ColorFilter::Brightness(1.2), ColorFilter::HueRotate(180.0)]
        );
        let shadow = chain.drop_shadow.unwrap();
        assert_eq!((shadow.x, shadow.y, shadow.blur), (0.0, 8.0, 12.0));
    }

    #[test]
    fn clamps_unit_amounts_and_takes_defaults() {
        let chain = parse_filter("grayscale() invert(250%) saturate(3)", "filter").unwrap();
        assert_eq!(
            chain.colors,
            vec![
                ColorFilter::Grayscale(1.0),
                ColorFilter::Invert(1.0),
                ColorFilter::Saturate(3.0)
            ]
        );
    }

    #[test]
    fn rejects_other_orders_and_unknown_functions() {
        assert!(parse_filter("brightness(2) blur(3px)", "filter").is_err());
        assert!(parse_filter("drop-shadow(0 0 4px red) sepia(1)", "filter").is_err());
        assert!(parse_filter("url(#f)", "filter").is_err());
        assert!(parse_filter("brightness(-1)", "filter").is_err());
        assert!(parse_filter("blur(300px)", "filter").is_err());
        assert!(parse_backdrop_filter("brightness(2)", "backdrop-filter").is_err());
        assert_eq!(
            parse_backdrop_filter("blur(8px)", "backdrop-filter").unwrap(),
            8.0
        );
    }
}
