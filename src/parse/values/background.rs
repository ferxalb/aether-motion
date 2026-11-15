//! `background` parsing: solid colors and two-color linear gradients.
use crate::model::Background;
use crate::parse::values::color::parse_color;
use crate::parse::values::tokens::split_top;
use anyhow::{bail, Result};

pub(in crate::parse) fn parse_background(raw: &str) -> Result<Background> {
    if let Some(body) = raw
        .strip_prefix("linear-gradient(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts = split_top(body, ',');
        let (horizontal, from, to) = match parts.as_slice() {
            ["to bottom", from, to] | [from, to] => (false, from, to),
            ["to top", from, to] => (false, to, from),
            ["to right", from, to] => (true, from, to),
            ["to left", from, to] => (true, to, from),
            _ => bail!(
                "linear-gradient: use a horizontal or vertical direction and two opaque colors"
            ),
        };
        let (from, to) = (parse_color(from)?, parse_color(to)?);
        if from.a != 255 || to.a != 255 {
            bail!("linear-gradient requires two opaque colors");
        }
        return Ok(Background::Linear {
            from,
            to,
            horizontal,
        });
    }
    Ok(Background::Solid(parse_color(raw)?))
}
