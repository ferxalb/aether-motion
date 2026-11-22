//! `box-shadow` (a list of outer and `inset` shadows) and `text-shadow` (a list of outer
//! shadows without spread) parsing.
use crate::model::{Color, Length, Shadow};
use crate::parse::values::color::parse_color;
use crate::parse::values::length::parse_length;
use crate::parse::values::tokens::split_top;
use anyhow::{bail, Result};

/// Parses a `box-shadow` list (comma separated outer or inset shadows, at most 8) in CSS order.
pub(in crate::parse) fn parse_box_shadows(raw: &str) -> Result<Vec<Shadow>> {
    let raw = raw.trim();
    if raw == "none" {
        return Ok(Vec::new());
    }
    let layers = split_top(raw, ',');
    if layers.len() > 8 {
        bail!("box-shadow: at most 8 shadows are supported");
    }
    layers
        .into_iter()
        .map(|layer| {
            parse_shadow(layer.trim(), "box-shadow")?
                .ok_or_else(|| anyhow::anyhow!("box-shadow: none cannot be part of a list: {raw}"))
        })
        .collect()
}

/// `text-shadow` layers, first on top; `none` is an empty list.
pub(in crate::parse) fn parse_text_shadows(raw: &str) -> Result<Vec<Shadow>> {
    let raw = raw.trim();
    if raw == "none" {
        return Ok(Vec::new());
    }
    let layers = split_top(raw, ',');
    if layers.len() > MAX_TEXT_SHADOWS {
        bail!("text-shadow: at most {MAX_TEXT_SHADOWS} shadows are supported");
    }
    layers
        .into_iter()
        .map(|layer| {
            parse_shadow(layer.trim(), "text-shadow")?
                .ok_or_else(|| anyhow::anyhow!("text-shadow: none cannot be part of a list: {raw}"))
        })
        .collect()
}

/// Most `text-shadow` layers; each one is a blurred mask of the whole text run.
const MAX_TEXT_SHADOWS: usize = 8;

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
    let mut inset = false;
    for token in split_top(raw, ' ') {
        if token == "inset" {
            if !allow_spread {
                bail!("{property}: inset is unsupported");
            }
            if std::mem::replace(&mut inset, true) {
                bail!("{property}: inset given twice: {raw}");
            }
            continue;
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
        inset,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lists_in_css_order() {
        let list =
            parse_box_shadows("0 2px 4px rgba(0, 0, 0, 0.5), 0 20px 60px 4px #00000066").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].y, list[0].blur), (2.0, 4.0));
        assert_eq!((list[1].y, list[1].spread), (20.0, 4.0));
        assert!(parse_box_shadows("none").unwrap().is_empty());
    }

    #[test]
    fn parses_inset_anywhere_in_a_layer() {
        let list = parse_box_shadows("inset 0 2px 4px #000, 0 1px 2px #fff inset").unwrap();
        assert!(list[0].inset && list[1].inset);
        assert!(!parse_box_shadows("0 2px 4px #000").unwrap()[0].inset);
        assert!(parse_box_shadows("inset inset 0 2px #000").is_err());
        assert!(parse_shadow("inset 0 2px 4px #000", "text-shadow").is_err());
    }

    #[test]
    fn rejects_long_lists_and_none_in_a_list() {
        assert!(parse_box_shadows(&["0 1px 2px #000"; 9].join(", ")).is_err());
        assert!(parse_box_shadows("0 2px 4px #000, none").is_err());
    }
}
