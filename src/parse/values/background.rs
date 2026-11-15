//! `background` parsing: solid colors, `linear-gradient`, and `radial-gradient`.
//!
//! Two opaque stops along an axis keep the cheap `Background::Linear` fast path; every other
//! gradient (angles, corners, several stops, translucent stops, radial) becomes `Background::Gradient`.

use crate::model::{
    Background, Color, Gradient, GradientKind, LinearDirection, Stop,
};
use crate::parse::values::color::parse_color;
use crate::parse::values::tokens::split_top;
use anyhow::{bail, Context, Result};

pub(in crate::parse) fn parse_background(raw: &str) -> Result<Background> {
    if let Some(body) = function_body(raw, "linear-gradient") {
        return parse_linear(body);
    }
    if let Some(body) = function_body(raw, "radial-gradient") {
        return parse_radial(body);
    }
    if raw.contains("gradient(") {
        bail!("unsupported gradient: {raw}; use linear-gradient or radial-gradient");
    }
    Ok(Background::Solid(parse_color(raw)?))
}

fn function_body<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    raw.strip_prefix(name)?.strip_prefix('(')?.strip_suffix(')')
}

fn parse_linear(body: &str) -> Result<Background> {
    let parts = split_top(body, ',');
    let (direction, stops) = match parts.split_first() {
        Some((first, rest)) if is_direction(first) => (parse_direction(first)?, rest),
        _ => (LinearDirection::Angle(180.0), parts.as_slice()),
    };
    let stops = parse_stops(stops)?;
    if let ([a, b], LinearDirection::Angle(angle)) = (stops.as_slice(), direction) {
        let axis = [(0.0, false), (180.0, false), (90.0, true), (270.0, true)]
            .into_iter()
            .find(|&(deg, _)| deg == angle);
        if let Some((_, horizontal)) = axis {
            let opaque = a.color.a == 255 && b.color.a == 255;
            if opaque && a.pos == 0.0 && b.pos == 1.0 {
                // Angle 0 and 270 run toward the start, so the colors swap.
                let (from, to) = if matches!(angle as i32, 0 | 270) {
                    (b.color, a.color)
                } else {
                    (a.color, b.color)
                };
                return Ok(Background::Linear { from, to, horizontal });
            }
        }
    }
    Ok(Background::Gradient(Gradient {
        kind: GradientKind::Linear(direction),
        stops,
    }))
}

fn is_direction(part: &str) -> bool {
    part.starts_with("to ")
        || part.ends_with("deg")
        || part.ends_with("turn")
        || part.ends_with("rad")
        || part.ends_with("grad")
}

fn parse_direction(part: &str) -> Result<LinearDirection> {
    if let Some(sides) = part.strip_prefix("to ") {
        let words: Vec<_> = sides.split_whitespace().collect();
        let (mut x, mut y) = (0i8, 0i8);
        for word in &words {
            match *word {
                "left" => x -= 1,
                "right" => x += 1,
                "top" => y -= 1,
                "bottom" => y += 1,
                _ => bail!("linear-gradient: unsupported direction: {part}"),
            }
        }
        return Ok(match (x, y, words.len()) {
            (0, -1, 1) => LinearDirection::Angle(0.0),
            (1, 0, 1) => LinearDirection::Angle(90.0),
            (0, 1, 1) => LinearDirection::Angle(180.0),
            (-1, 0, 1) => LinearDirection::Angle(270.0),
            (-1 | 1, -1 | 1, 2) => LinearDirection::Corner(x, y),
            _ => bail!("linear-gradient: unsupported direction: {part}"),
        });
    }
    let split = part
        .find(|c: char| c.is_ascii_alphabetic())
        .context("linear-gradient: invalid angle")?;
    let (number, unit) = part.split_at(split);
    let n: f32 = number.parse().with_context(|| format!("linear-gradient: invalid angle: {part}"))?;
    let degrees = match unit {
        "deg" => n,
        "turn" => n * 360.0,
        "rad" => n.to_degrees(),
        "grad" => n * 0.9,
        _ => bail!("linear-gradient: invalid angle unit: {part}"),
    };
    if !degrees.is_finite() {
        bail!("linear-gradient: invalid angle: {part}");
    }
    Ok(LinearDirection::Angle(degrees.rem_euclid(360.0)))
}

fn parse_radial(body: &str) -> Result<Background> {
    let parts = split_top(body, ',');
    let mut circle = false;
    let mut farthest_side = false;
    let mut center = (0.5, 0.5);
    let mut stops = parts.as_slice();
    if let Some((first, rest)) = parts.split_first() {
        let looks_like_config = first.contains("circle")
            || first.contains("ellipse")
            || first.starts_with("at ")
            || first.contains("farthest")
            || first.contains("closest");
        if looks_like_config {
            let (shape, position) = match first.split_once("at ") {
                Some((shape, position)) => (shape, Some(position)),
                None => (*first, None),
            };
            for word in shape.split_whitespace() {
                match word {
                    "circle" => circle = true,
                    "ellipse" => circle = false,
                    "farthest-corner" => farthest_side = false,
                    "farthest-side" => farthest_side = true,
                    _ => bail!(
                        "radial-gradient: unsupported shape or size: {word}; use circle, ellipse, farthest-corner, or farthest-side"
                    ),
                }
            }
            if let Some(position) = position {
                center = parse_position(position)?;
            }
            stops = rest;
        }
    }
    Ok(Background::Gradient(Gradient {
        kind: GradientKind::Radial {
            circle,
            center,
            farthest_side,
        },
        stops: parse_stops(stops)?,
    }))
}

/// `at` position: keywords or percentages (px would need the box size and is unsupported).
fn parse_position(text: &str) -> Result<(f32, f32)> {
    let words: Vec<_> = text.split_whitespace().collect();
    let axis = |word: &str| -> Result<f32> {
        Ok(match word {
            "left" | "top" => 0.0,
            "center" => 0.5,
            "right" | "bottom" => 1.0,
            _ => word
                .strip_suffix('%')
                .and_then(|n| n.parse::<f32>().ok())
                .map(|n| n / 100.0)
                .with_context(|| {
                    format!("radial-gradient: position must use keywords or %: {text}")
                })?,
        })
    };
    Ok(match words.as_slice() {
        [one] => match *one {
            "top" | "bottom" => (0.5, axis(one)?),
            _ => (axis(one)?, 0.5),
        },
        [x, y] if matches!(*x, "top" | "bottom") && matches!(*y, "left" | "right" | "center") => {
            (axis(y)?, axis(x)?)
        }
        [x, y] => (axis(x)?, axis(y)?),
        _ => bail!("radial-gradient: unsupported position: {text}"),
    })
}

/// Parses `color [N%]` stops and fills in CSS default positions.
fn parse_stops(parts: &[&str]) -> Result<Vec<Stop>> {
    if parts.len() < 2 {
        bail!("gradient needs at least two color stops");
    }
    let mut raw: Vec<(Color, Option<f32>)> = Vec::with_capacity(parts.len());
    for part in parts {
        let tokens = split_top(part, ' ');
        let (color, position) = match tokens.as_slice() {
            [color] => (*color, None),
            [color, position] => {
                let n = position
                    .strip_suffix('%')
                    .and_then(|n| n.parse::<f32>().ok())
                    .filter(|n| n.is_finite())
                    .with_context(|| {
                        format!("gradient stop position must be a percentage: {position}")
                    })?;
                (*color, Some(n / 100.0))
            }
            _ => bail!("unsupported gradient stop: {part}"),
        };
        raw.push((parse_color(color)?, position));
    }
    let last = raw.len() - 1;
    raw[0].1.get_or_insert(0.0);
    raw[last].1.get_or_insert(1.0);
    // Positions never go backwards.
    let mut floor = f32::MIN;
    for (_, position) in raw.iter_mut() {
        if let Some(p) = position {
            *p = p.max(floor);
            floor = *p;
        }
    }
    // Unset stops spread evenly between their positioned neighbours.
    let mut i = 0;
    while i < raw.len() {
        if raw[i].1.is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let end = (i..raw.len()).find(|&j| raw[j].1.is_some()).unwrap();
        let (a, b) = (raw[start].1.unwrap(), raw[end].1.unwrap());
        for j in start + 1..end {
            raw[j].1 = Some(a + (b - a) * (j - start) as f32 / (end - start) as f32);
        }
        i = end + 1;
    }
    Ok(raw
        .into_iter()
        .map(|(color, pos)| Stop {
            pos: pos.unwrap(),
            color,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_opaque_axis_stops_keep_the_fast_path() {
        assert!(matches!(
            parse_background("linear-gradient(to right, red, blue)").unwrap(),
            Background::Linear { horizontal: true, .. }
        ));
        assert!(matches!(
            parse_background("linear-gradient(#000, #fff)").unwrap(),
            Background::Linear { horizontal: false, .. }
        ));
    }

    #[test]
    fn other_gradients_use_the_general_path() {
        let Background::Gradient(g) =
            parse_background("linear-gradient(135deg, red, #00f 40%, rgba(0,0,0,0.5))").unwrap()
        else {
            panic!("expected a general gradient");
        };
        assert_eq!(g.kind, GradientKind::Linear(LinearDirection::Angle(135.0)));
        let positions: Vec<_> = g.stops.iter().map(|s| s.pos).collect();
        assert_eq!(positions, [0.0, 0.4, 1.0]);
    }

    #[test]
    fn unset_stops_spread_evenly() {
        let Background::Gradient(g) =
            parse_background("linear-gradient(to bottom right, red, green, blue, white)").unwrap()
        else {
            panic!();
        };
        assert_eq!(g.kind, GradientKind::Linear(LinearDirection::Corner(1, 1)));
        let positions: Vec<_> = g.stops.iter().map(|s| s.pos).collect();
        assert!(positions.iter().zip([0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn radial_parses_shape_and_position() {
        let Background::Gradient(g) =
            parse_background("radial-gradient(circle at 30% top, #fff, transparent)").unwrap()
        else {
            panic!();
        };
        assert_eq!(
            g.kind,
            GradientKind::Radial { circle: true, center: (0.3, 0.0), farthest_side: false }
        );
    }

    #[test]
    fn rejects_what_it_cannot_draw() {
        assert!(parse_background("linear-gradient(red)").is_err());
        assert!(parse_background("radial-gradient(closest-side, red, blue)").is_err());
        assert!(parse_background("conic-gradient(red, blue)").is_err());
        assert!(parse_background("linear-gradient(red 10px, blue)").is_err());
    }
}
