//! Lengths and spacing values: `px`/`%`, single spacing values, and 1-to-4 value boxes.
use crate::model::Length;
use crate::parse::values::tokens::split_top;
use anyhow::{bail, Context, Result};

pub(in crate::parse) fn parse_length(raw: &str, property: &str) -> Result<Length> {
    let raw = raw.trim();
    if let Some(s) = raw.strip_suffix('%') {
        return Ok(Length::Percent(
            s.trim()
                .parse()
                .with_context(|| format!("{property}: invalid percentage"))?,
        ));
    }
    let s = raw.strip_suffix("px").unwrap_or(raw).trim();
    let n: f32 = s
        .parse()
        .with_context(|| format!("{property}: only px and % are supported: {raw}"))?;
    if !n.is_finite() {
        bail!("{property}: non-finite value");
    }
    Ok(Length::Px(n))
}

/// Non-negative px length (`0` allowed) for spacing properties.
pub(in crate::parse) fn spacing_px(raw: &str, property: &str) -> Result<f32> {
    match parse_length(raw, property)? {
        Length::Px(n) if n >= 0.0 => Ok(n),
        _ => bail!("{property}: only non-negative px values are supported: {raw}"),
    }
}

/// CSS 1-4 value box shorthand into top, right, bottom, left.
pub(in crate::parse) fn parse_box(raw: &str, property: &str) -> Result<[f32; 4]> {
    let v: Vec<f32> = split_top(raw, ' ')
        .into_iter()
        .map(|part| spacing_px(part, property))
        .collect::<Result<_>>()?;
    Ok(match v[..] {
        [a] => [a, a, a, a],
        [v1, h] => [v1, h, v1, h],
        [t, h, b] => [t, h, b, h],
        [t, r, b, l] => [t, r, b, l],
        _ => bail!("{property}: expected 1 to 4 values: {raw}"),
    })
}
