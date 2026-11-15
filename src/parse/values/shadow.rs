//! `box-shadow` and `text-shadow` parsing: one outer shadow.
use crate::model::{Color, Length, Shadow};
use crate::parse::values::color::parse_color;
use crate::parse::values::length::parse_length;
use crate::parse::values::tokens::split_top;
use anyhow::{bail, Result};

/// Parses one shadow for `property`. `text-shadow` has no spread, so it takes at most three lengths.
pub(in crate::parse) fn parse_shadow(raw: &str, property: &str) -> Result<Option<Shadow>> {
    let allow_spread = property == "box-shadow";
    if raw == "none" {
        return Ok(None);
    }
    if split_top(raw, ',').len() > 1 {
        bail!("{property}: several shadows are unsupported: {raw}");
    }
    let mut lengths = Vec::new();
    let mut color = None;
    for token in split_top(raw, ' ') {
        if token == "inset" {
            bail!("{property}: inset is unsupported");
        }
        let numeric =
            token.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'));
        if numeric && !token.contains('(') {
            let n = match parse_length(token, property)? {
                Length::Px(n) => n,
                Length::Percent(_) => bail!("{property}: only px lengths are supported"),
            };
            lengths.push(n);
        } else if color.replace(parse_color(token)?).is_some() {
            bail!("{property}: more than one color: {raw}");
        }
    }
    let [x, y, blur, spread] = match lengths[..] {
        [x, y] => [x, y, 0.0, 0.0],
        [x, y, blur] => [x, y, blur, 0.0],
        [x, y, blur, spread] if allow_spread => [x, y, blur, spread],
        _ if allow_spread => {
            bail!("{property}: expected offset-x offset-y [blur [spread]] color: {raw}")
        }
        _ => bail!("{property}: expected offset-x offset-y [blur] color (no spread): {raw}"),
    };
    if blur < 0.0 {
        bail!("{property}: blur must not be negative");
    }
    Ok(Some(Shadow {
        x,
        y,
        blur,
        spread,
        color: color.unwrap_or(Color::BLACK),
    }))
}
