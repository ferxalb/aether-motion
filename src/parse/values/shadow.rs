//! `box-shadow` parsing: one outer shadow.
use crate::model::{Color, Length, Shadow};
use crate::parse::values::color::parse_color;
use crate::parse::values::length::parse_length;
use crate::parse::values::tokens::split_top;
use anyhow::{bail, Result};

pub(in crate::parse) fn parse_shadow(raw: &str) -> Result<Option<Shadow>> {
    if raw == "none" {
        return Ok(None);
    }
    if split_top(raw, ',').len() > 1 {
        bail!("box-shadow: several shadows are unsupported: {raw}");
    }
    let mut lengths = Vec::new();
    let mut color = None;
    for token in split_top(raw, ' ') {
        if token == "inset" {
            bail!("box-shadow: inset is unsupported");
        }
        let numeric =
            token.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'));
        if numeric && !token.contains('(') {
            let n = match parse_length(token, "box-shadow")? {
                Length::Px(n) => n,
                Length::Percent(_) => bail!("box-shadow: only px lengths are supported"),
            };
            lengths.push(n);
        } else if color.replace(parse_color(token)?).is_some() {
            bail!("box-shadow: more than one color: {raw}");
        }
    }
    let [x, y, blur, spread] = match lengths[..] {
        [x, y] => [x, y, 0.0, 0.0],
        [x, y, blur] => [x, y, blur, 0.0],
        [x, y, blur, spread] => [x, y, blur, spread],
        _ => bail!("box-shadow: expected offset-x offset-y [blur [spread]] color: {raw}"),
    };
    if blur < 0.0 {
        bail!("box-shadow: blur must not be negative");
    }
    Ok(Some(Shadow {
        x,
        y,
        blur,
        spread,
        color: color.unwrap_or(Color::BLACK),
    }))
}
