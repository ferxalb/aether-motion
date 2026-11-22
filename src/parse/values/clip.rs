//! `clip-path` shapes (`inset()`, `circle()`, `ellipse()`, `polygon()`, `path()`) with an optional
//! `<geometry-box>` (`margin-box`, `border-box`, `padding-box`, `content-box`), a box keyword
//! alone, and `mask-image` gradients.
//!
//! Basic shapes with px or % values can be interpolated by `data-hf-keyframes`; `path()`, a box
//! alone, and `closest-side`/`farthest-side` radii are static. `url()` is unsupported.
use crate::model::{ClipBox, ClipKind, ClipPath, FillRule, Gradient, Length, RadiusKeyword};
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

/// Most flattened points accepted in one `path()`.
const MAX_PATH_POINTS: usize = 20_000;

/// A `circle()` or `ellipse()` radius: a length, or a keyword resolved when drawn (`0px` stands
/// in for it in the params).
fn radius(word: &str) -> Result<(Length, Option<RadiusKeyword>)> {
    match word {
        "closest-side" => Ok((Length::Px(0.0), Some(RadiusKeyword::ClosestSide))),
        "farthest-side" => Ok((Length::Px(0.0), Some(RadiusKeyword::FarthestSide))),
        _ => Ok((non_negative(length(word)?, "a radius")?, None)),
    }
}

/// A `<geometry-box>` keyword.
fn reference_box(word: &str) -> Option<Result<ClipBox>> {
    Some(Ok(match word {
        "margin-box" => ClipBox::Margin,
        "border-box" => ClipBox::Border,
        "padding-box" => ClipBox::Padding,
        "content-box" => ClipBox::Content,
        "fill-box" | "stroke-box" | "view-box" => {
            return Some(Err(anyhow!(
                "clip-path: {word} applies to SVG elements only; use border-box, padding-box, content-box, or margin-box"
            )))
        }
        _ => return None,
    }))
}

/// An optional leading fill rule of `polygon()` or `path()`.
fn fill_rule(first: Option<&str>) -> Option<FillRule> {
    match first.map(str::trim) {
        Some("nonzero") => Some(FillRule::NonZero),
        Some("evenodd") => Some(FillRule::EvenOdd),
        _ => None,
    }
}

/// Parses a `clip-path` value (`<basic-shape> || <geometry-box>`); `None` for `none`.
pub(in crate::parse) fn parse_clip_path(raw: &str) -> Result<Option<ClipPath>> {
    let raw = raw.trim();
    if raw == "none" {
        return Ok(None);
    }
    if raw.starts_with("url(") {
        bail!("clip-path: url() references are not supported; use a basic shape or path()");
    }
    let mut reference = None;
    let mut shape_text = None;
    for token in split_top(raw, ' ') {
        if let Some(found) = reference_box(token) {
            if reference.replace(found?).is_some() {
                bail!("clip-path: give at most one box keyword: {raw}");
            }
        } else if shape_text.replace(token).is_some() {
            bail!("clip-path: give one shape and at most one box keyword: {raw}");
        }
    }
    let reference = reference.unwrap_or_default();
    let Some(text) = shape_text else {
        // A box alone clips to that box, rounded like the element.
        let mut shape = ClipPath::basic(ClipKind::Inset, vec![Length::Px(0.0); 5]);
        shape.reference = reference;
        shape.box_only = true;
        return Ok(Some(shape));
    };
    let mut shape = parse_shape(text, raw)?;
    shape.reference = reference;
    Ok(Some(shape))
}

/// Parses one basic shape function.
fn parse_shape(raw: &str, whole: &str) -> Result<ClipPath> {
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
        ClipPath::basic(ClipKind::Inset, vec![top, right, bottom, left, radius])
    } else if let Some(text) = body(raw, "circle") {
        let (sizes, [x, y]) = sizes_and_position(text)?;
        let (r, keyword) = match sizes.as_slice() {
            [] => radius("closest-side")?,
            [r] => radius(r)?,
            _ => bail!(
                "clip-path: circle() takes one radius (px, %, closest-side, or farthest-side)"
            ),
        };
        let mut shape = ClipPath::basic(ClipKind::Circle, vec![r, x, y]);
        shape.radii = [keyword, None];
        shape
    } else if let Some(text) = body(raw, "ellipse") {
        let (sizes, [x, y]) = sizes_and_position(text)?;
        let ((rx, kx), (ry, ky)) = match sizes.as_slice() {
            [] => (radius("closest-side")?, radius("closest-side")?),
            [rx, ry] => (radius(rx)?, radius(ry)?),
            _ => bail!("clip-path: ellipse() takes two radii (px, %, closest-side, or farthest-side) or none"),
        };
        let mut shape = ClipPath::basic(ClipKind::Ellipse, vec![rx, ry, x, y]);
        shape.radii = [kx, ky];
        shape
    } else if let Some(text) = body(raw, "polygon") {
        let mut points = split_top(text, ',');
        let rule = fill_rule(points.first().copied());
        if rule.is_some() {
            points.remove(0);
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
        let mut shape = ClipPath::basic(ClipKind::Polygon, params);
        shape.rule = rule.unwrap_or_default();
        shape
    } else if let Some(text) = body(raw, "path") {
        let mut parts = split_top(text, ',');
        let rule = fill_rule(parts.first().copied());
        if rule.is_some() {
            parts.remove(0);
        }
        let [data] = parts.as_slice() else {
            bail!("clip-path: path() takes an optional fill rule and one quoted string of path data: {whole}");
        };
        let data = data.trim();
        let data = data
            .strip_prefix('"')
            .and_then(|d| d.strip_suffix('"'))
            .or_else(|| data.strip_prefix('\'').and_then(|d| d.strip_suffix('\'')))
            .ok_or_else(|| anyhow!("clip-path: path() data must be a quoted string: {whole}"))?;
        let measure = crate::vector::PathMeasure::from_svg_data(data)
            .with_context(|| format!("clip-path: path(): {data}"))?;
        let contours = measure.contours();
        let points: usize = contours.iter().map(Vec::len).sum();
        if contours.is_empty() {
            bail!("clip-path: path() encloses no area: {data}");
        }
        if points > MAX_PATH_POINTS {
            bail!("clip-path: path() is too detailed ({points} flattened points; at most {MAX_PATH_POINTS})");
        }
        let mut shape = ClipPath::basic(ClipKind::Path, Vec::new());
        shape.rule = rule.unwrap_or_default();
        shape.contours = Some(std::sync::Arc::new(contours));
        shape
    } else {
        bail!("unsupported clip-path: {whole}; use inset(), circle(), ellipse(), polygon(), path(), a box keyword, or none");
    };
    Ok(shape)
}

/// Interpolation needs the same shape, the same number of points, and the same unit per value.
pub(in crate::parse) fn check_compatible(base: &ClipPath, other: &ClipPath) -> Result<()> {
    if base.kind == ClipKind::Path
        || other.kind == ClipKind::Path
        || base.box_only
        || other.box_only
    {
        bail!("path() and box-only clip-paths cannot be animated; animate a basic shape instead");
    }
    if base.radii.iter().chain(&other.radii).any(Option::is_some) {
        bail!("clip-path radii given as closest-side or farthest-side cannot be animated; use px or %");
    }
    let same_units = base.params.len() == other.params.len()
        && base
            .params
            .iter()
            .zip(&other.params)
            .all(|(a, b)| std::mem::discriminant(a) == std::mem::discriminant(b));
    if base.kind != other.kind || !same_units {
        bail!("animated clip-path shapes must use the same function, the same number of points, and the same unit (px or %) for each value");
    }
    if base.rule != other.rule || base.reference != other.reference {
        bail!("animated clip-path shapes must use the same fill rule and box keyword");
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
    background
        .to_gradient()
        .map(Some)
        .ok_or_else(|| anyhow!("mask-image needs a gradient"))
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
            "path('M0 0')",
            "path(M0 0 L4 0 L0 4)",
            "url(#a)",
            "circle(4px) fill-box",
            "circle(4px) padding-box content-box",
            "circle(4px) ellipse(2px 2px)",
            "ellipse(4px)",
            "polygon(0 0, 1px 1px)",
            "inset(1px round 10%)",
            "circle(-5px)",
        ] {
            assert!(parse_clip_path(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn parses_paths_rules_boxes_and_radius_keywords() {
        let path =
            parse_clip_path("path(evenodd, 'M0 0 H10 V10 H0 Z M2 2 H8 V8 H2 Z') content-box")
                .unwrap()
                .unwrap();
        assert_eq!(
            (path.kind, path.rule, path.reference),
            (ClipKind::Path, FillRule::EvenOdd, ClipBox::Content)
        );
        assert_eq!(path.contours.as_ref().unwrap().len(), 2);
        let polygon = parse_clip_path("padding-box polygon(evenodd, 0 0, 4px 0, 0 4px)")
            .unwrap()
            .unwrap();
        assert_eq!(
            (polygon.rule, polygon.reference),
            (FillRule::EvenOdd, ClipBox::Padding)
        );
        let circle = parse_clip_path("circle(farthest-side at 0 0)")
            .unwrap()
            .unwrap();
        assert_eq!(circle.radii, [Some(RadiusKeyword::FarthestSide), None]);
        let default = parse_clip_path("circle()").unwrap().unwrap();
        assert_eq!(default.radii[0], Some(RadiusKeyword::ClosestSide));
        let ellipse = parse_clip_path("ellipse(closest-side 20%)")
            .unwrap()
            .unwrap();
        assert_eq!(ellipse.radii, [Some(RadiusKeyword::ClosestSide), None]);
        let alone = parse_clip_path("content-box").unwrap().unwrap();
        assert!(alone.box_only && alone.reference == ClipBox::Content);
        assert!(check_compatible(&path, &path).is_err());
        assert!(check_compatible(&circle, &circle).is_err());
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
