//! `clip-path` basic shapes (`inset()`, `circle()`, `ellipse()`, `polygon()`) and `mask-image`
//! gradients.
//!
//! Shapes use px or % (no keywords such as `closest-side`, no `<geometry-box>`, `path()`, or
//! `url()`), so every parameter is a number that can be interpolated by `data-hf-keyframes`.
use crate::model::{
    Background, ClipKind, ClipPath, Gradient, GradientKind, Length, LinearDirection, Stop,
};
use crate::parse::values::background::parse_background;
use crate::parse::values::length::parse_length;
use crate::parse::values::tokens::split_top;
use anyhow::{anyhow, bail, Context, Result};

/// Most vertices accepted in one `polygon()`.
const MAX_POINTS: usize = 256;

fn body<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    raw.strip_prefix(name)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')
}

fn length(raw: &str) -> Result<Length> {
    parse_length(raw, "clip-path")
}

/// `at <x> <y>` (keywords, px, or %); the center when absent.
fn position(words: &[&str]) -> Result<[Length; 2]> {
    let keyword = |word: &str| -> Option<Length> {
        match word {
            "left" | "top" => Some(Length::Percent(0.0)),
            "center" => Some(Length::Percent(50.0)),
            "right" | "bottom" => Some(Length::Percent(100.0)),
            _ => None,
        }
    };
    let axis = |word: &str| keyword(word).map_or_else(|| length(word), Ok);
    Ok(match words {
        [] => [Length::Percent(50.0), Length::Percent(50.0)],
        [one] => match *one {
            "top" | "bottom" => [Length::Percent(50.0), axis(one)?],
            _ => [axis(one)?, Length::Percent(50.0)],
        },
        [y, x] if matches!(*y, "top" | "bottom") && matches!(*x, "left" | "right" | "center") => {
            [axis(x)?, axis(y)?]
        }
        [x, y] => [axis(x)?, axis(y)?],
        _ => bail!(
            "clip-path: a position takes one or two values: at {}",
            words.join(" ")
        ),
    })
}

/// Splits `<sizes> [at <position>]`.
fn sizes_and_position(text: &str) -> Result<(Vec<&str>, [Length; 2])> {
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.iter().position(|w| *w == "at") {
        Some(i) => Ok((words[..i].to_vec(), position(&words[i + 1..])?)),
        None => Ok((words, position(&[])?)),
    }
}

fn non_negative(value: Length, what: &str) -> Result<Length> {
    match value {
        Length::Px(n) | Length::Percent(n) if n < 0.0 => {
            bail!("clip-path: {what} cannot be negative")
        }
        _ => Ok(value),
    }
}

/// Parses a `clip-path` value; `None` for `none`.
pub(in crate::parse) fn parse_clip_path(raw: &str) -> Result<Option<ClipPath>> {
    let raw = raw.trim();
    if raw == "none" {
        return Ok(None);
    }
    let shape = if let Some(text) = body(raw, "inset") {
        let (edges, round) = match text.split_once(" round ") {
            Some((edges, round)) => (edges, Some(round.trim())),
            None => (text, None),
        };
        let edges: Vec<Length> = edges
            .split_whitespace()
            .map(length)
            .collect::<Result<_>>()?;
        let [top, right, bottom, left] = match edges.as_slice() {
            [a] => [*a; 4],
            [a, b] => [*a, *b, *a, *b],
            [a, b, c] => [*a, *b, *c, *b],
            [a, b, c, d] => [*a, *b, *c, *d],
            _ => bail!("clip-path: inset() takes one to four lengths"),
        };
        let radius = match round {
            None => Length::Px(0.0),
            Some(r) => match length(r)? {
                Length::Px(n) if n >= 0.0 => Length::Px(n),
                _ => bail!("clip-path: inset() round takes one non-negative px radius"),
            },
        };
        ClipPath {
            kind: ClipKind::Inset,
            params: vec![top, right, bottom, left, radius],
        }
    } else if let Some(text) = body(raw, "circle") {
        let (sizes, [x, y]) = sizes_and_position(text)?;
        let [r] = sizes.as_slice() else {
            bail!("clip-path: circle() needs one radius in px or % (keywords such as closest-side are not supported)");
        };
        ClipPath {
            kind: ClipKind::Circle,
            params: vec![non_negative(length(r)?, "a radius")?, x, y],
        }
    } else if let Some(text) = body(raw, "ellipse") {
        let (sizes, [x, y]) = sizes_and_position(text)?;
        let [rx, ry] = sizes.as_slice() else {
            bail!("clip-path: ellipse() needs two radii in px or % (keywords such as closest-side are not supported)");
        };
        ClipPath {
            kind: ClipKind::Ellipse,
            params: vec![
                non_negative(length(rx)?, "a radius")?,
                non_negative(length(ry)?, "a radius")?,
                x,
                y,
            ],
        }
    } else if let Some(text) = body(raw, "polygon") {
        let mut points = split_top(text, ',');
        if points.first().is_some_and(|p| *p == "nonzero") {
            points.remove(0);
        } else if points.first().is_some_and(|p| *p == "evenodd") {
            bail!("clip-path: polygon() supports the nonzero fill rule only");
        }
        if points.len() < 3 || points.len() > MAX_POINTS {
            bail!("clip-path: polygon() needs 3 to {MAX_POINTS} points");
        }
        let mut params = Vec::with_capacity(points.len() * 2);
        for point in points {
            let xy: Vec<&str> = point.split_whitespace().collect();
            let [x, y] = xy.as_slice() else {
                bail!("clip-path: each polygon() point is `x y`: {point}");
            };
            params.push(length(x)?);
            params.push(length(y)?);
        }
        ClipPath {
            kind: ClipKind::Polygon,
            params,
        }
    } else {
        bail!("unsupported clip-path: {raw}; use inset(), circle(), ellipse(), polygon(), or none (path(), url(), and box keywords are not supported)");
    };
    Ok(Some(shape))
}

/// Interpolation needs the same shape, the same number of points, and the same unit per value.
pub(in crate::parse) fn check_compatible(base: &ClipPath, other: &ClipPath) -> Result<()> {
    let same_units = base.params.len() == other.params.len()
        && base
            .params
            .iter()
            .zip(&other.params)
            .all(|(a, b)| std::mem::discriminant(a) == std::mem::discriminant(b));
    if base.kind != other.kind || !same_units {
        bail!("animated clip-path shapes must use the same function, the same number of points, and the same unit (px or %) for each value");
    }
    Ok(())
}

/// Parses a `mask-image` value into a gradient; `None` for `none`.
pub(in crate::parse) fn parse_mask_image(raw: &str) -> Result<Option<Gradient>> {
    let raw = raw.trim();
    if raw == "none" {
        return Ok(None);
    }
    if !(raw.starts_with("linear-gradient") || raw.starts_with("radial-gradient")) {
        bail!("mask-image: only linear-gradient() and radial-gradient() are supported (no url() images)");
    }
    let background = parse_background(raw).context("mask-image")?;
    Ok(Some(match background {
        Background::Gradient(gradient) => gradient,
        Background::Linear {
            from,
            to,
            horizontal,
        } => Gradient {
            kind: GradientKind::Linear(LinearDirection::Angle(if horizontal {
                90.0
            } else {
                180.0
            })),
            stops: vec![
                Stop {
                    pos: 0.0,
                    color: from,
                },
                Stop {
                    pos: 1.0,
                    color: to,
                },
            ],
        },
        Background::Solid(_) => return Err(anyhow!("mask-image needs a gradient")),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_shape() {
        let inset = parse_clip_path("inset(10px 20% round 8px)")
            .unwrap()
            .unwrap();
        assert_eq!(inset.kind, ClipKind::Inset);
        assert_eq!(inset.values(), vec![10.0, 20.0, 10.0, 20.0, 8.0]);
        let circle = parse_clip_path("circle(40% at right top)")
            .unwrap()
            .unwrap();
        assert_eq!(circle.values(), vec![40.0, 100.0, 0.0]);
        let ellipse = parse_clip_path("ellipse(30px 20px)").unwrap().unwrap();
        assert_eq!(ellipse.values(), vec![30.0, 20.0, 50.0, 50.0]);
        let polygon = parse_clip_path("polygon(50% 0%, 100% 100%, 0% 100%)")
            .unwrap()
            .unwrap();
        assert_eq!(polygon.params.len(), 6);
        assert!(parse_clip_path("none").unwrap().is_none());
    }

    #[test]
    fn rejects_unsupported_forms() {
        for raw in [
            "circle(closest-side)",
            "path('M0 0')",
            "url(#a)",
            "polygon(evenodd, 0 0, 1px 0, 0 1px)",
            "polygon(0 0, 1px 1px)",
            "inset(1px round 10%)",
            "circle(-5px)",
        ] {
            assert!(parse_clip_path(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn compatibility_needs_matching_units() {
        let a = parse_clip_path("circle(0% at 50% 50%)").unwrap().unwrap();
        let b = parse_clip_path("circle(75% at 50% 50%)").unwrap().unwrap();
        let c = parse_clip_path("circle(75px at 50% 50%)").unwrap().unwrap();
        assert!(check_compatible(&a, &b).is_ok());
        assert!(check_compatible(&a, &c).is_err());
    }

    #[test]
    fn mask_images_become_gradients() {
        let g = parse_mask_image("linear-gradient(to bottom, #000, transparent)")
            .unwrap()
            .unwrap();
        assert_eq!(g.stops.len(), 2);
        assert!(parse_mask_image("url(a.png)").is_err());
    }
}
