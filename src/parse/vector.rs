//! Vector motion attributes: `data-hf-motion-path` (travel along a path) on any element, and
//! `data-hf-draw` (self-drawing strokes) on an inline `<svg>`.
//!
//! `data-hf-motion-path="<SVG path data>"` with `data-hf-motion-path-at` (seconds from the clip start,
//! default 0), `-duration` (default 2), `-easing` (default `ease-in-out-cubic`), and `-rotate`
//! (`auto` or `auto <deg>` to turn with the path). `data-hf-draw` (empty or `true`) with
//! `data-hf-draw-at` (default 0), `-duration` (default 1.5 per shape), `-stagger` (seconds between
//! shapes, default 0), and `-easing` (default `ease-in-out-cubic`).
use crate::model::{Easing, MotionPath};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::html::{attr, number};
use crate::vector::draw_on::{token, DrawTiming};
use crate::vector::PathMeasure;
use anyhow::{bail, Context, Result};
use kuchiki::NodeRef;

/// Shape elements whose strokes `data-hf-draw` reveals.
const SHAPES: [&str; 7] = [
    "path", "line", "polyline", "polygon", "rect", "circle", "ellipse",
];
/// Most shapes one drawing animates.
const MAX_SHAPES: usize = 2_000;

/// A finite number attribute `{prefix}-{name}`, or `default`; `range` bounds it.
fn option(
    node: &NodeRef,
    prefix: &str,
    name: &str,
    default: f64,
    range: (f64, f64),
) -> Result<f64> {
    let key = format!("{prefix}-{name}");
    let value = attr(node, &key)
        .map(|raw| number(&raw, &key))
        .transpose()?
        .unwrap_or(default);
    if !value.is_finite() || value < range.0 || value > range.1 {
        bail!("{key} must be between {} and {}", range.0, range.1);
    }
    Ok(value)
}

fn easing(node: &NodeRef, key: &str) -> Result<Easing> {
    parse_keyframe_easing(attr(node, key).as_deref().unwrap_or("ease-in-out-cubic"))
}

/// Errors when an option is present without its main attribute.
fn orphans(node: &NodeRef, prefix: &str, names: &[&str]) -> Result<()> {
    for name in names {
        let key = format!("{prefix}-{name}");
        if attr(node, &key).is_some() {
            bail!("{key} requires {prefix}");
        }
    }
    Ok(())
}

/// Reads `data-hf-motion-path`; `None` when absent.
pub(in crate::parse) fn parse_motion_path(node: &NodeRef) -> Result<Option<MotionPath>> {
    const PREFIX: &str = "data-hf-motion-path";
    let Some(data) = attr(node, PREFIX) else {
        orphans(node, PREFIX, &["at", "duration", "easing", "rotate"])?;
        return Ok(None);
    };
    let measure = PathMeasure::from_svg_data(data.trim()).context(PREFIX)?;
    let rotate = match attr(node, "data-hf-motion-path-rotate")
        .as_deref()
        .map(str::trim)
    {
        None | Some("none") => None,
        Some(raw) => {
            let rest = raw.strip_prefix("auto").ok_or_else(|| {
                anyhow::anyhow!(
                    "data-hf-motion-path-rotate must be none, auto, or auto <deg>: {raw}"
                )
            })?;
            let rest = rest.trim();
            Some(if rest.is_empty() {
                0.0
            } else {
                let degrees = rest.strip_suffix("deg").unwrap_or(rest).trim();
                degrees
                    .parse::<f32>()
                    .ok()
                    .filter(|d| d.is_finite())
                    .with_context(|| format!("data-hf-motion-path-rotate: invalid angle {rest}"))?
            })
        }
    };
    Ok(Some(MotionPath {
        measure,
        at: option(node, PREFIX, "at", 0.0, (0.0, 3600.0))?,
        duration: option(node, PREFIX, "duration", 2.0, (0.0, 3600.0))?,
        easing: easing(node, "data-hf-motion-path-easing")?,
        rotate,
    }))
}

/// Reads `data-hf-draw` on an inline `<svg>` and, when present, marks every visible shape: each
/// gets an `id` (if it has none) and a placeholder at the end of its `style`. Returns the ids in
/// document order and the timing.
pub(in crate::parse) fn prepare_draw(
    node: &NodeRef,
    tag: &str,
) -> Result<Option<(Vec<String>, DrawTiming)>> {
    const PREFIX: &str = "data-hf-draw";
    let Some(raw) = attr(node, PREFIX) else {
        orphans(node, PREFIX, &["at", "duration", "stagger", "easing"])?;
        return Ok(None);
    };
    match raw.trim() {
        "" | "true" | "stroke" => {}
        "false" => return Ok(None),
        other => bail!("data-hf-draw must be true (or empty): {other}"),
    }
    if tag != "svg" {
        bail!("data-hf-draw works on an inline <svg> only (inline the SVG file to animate its strokes)");
    }
    let timing = DrawTiming {
        at: option(node, PREFIX, "at", 0.0, (0.0, 3600.0))?,
        duration: option(node, PREFIX, "duration", 1.5, (0.0, 3600.0))?,
        stagger: option(node, PREFIX, "stagger", 0.0, (0.0, 3600.0))?,
        easing: easing(node, "data-hf-draw-easing")?,
    };
    let mut ids = Vec::new();
    for descendant in node.descendants() {
        let Some(element) = descendant.as_element() else {
            continue;
        };
        let name = element.name.local.to_string();
        if !SHAPES.contains(&name.as_str()) || in_resource(&descendant) {
            continue;
        }
        if ids.len() == MAX_SHAPES {
            bail!("data-hf-draw animates at most {MAX_SHAPES} shapes; split the drawing");
        }
        let index = ids.len();
        let mut attributes = element.attributes.borrow_mut();
        let id = match attributes.get("id") {
            Some(id) if !id.trim().is_empty() => id.to_string(),
            _ => {
                let id = format!("aether-draw-{index}");
                attributes.insert("id", id.clone());
                id
            }
        };
        let style = attributes.get("style").unwrap_or("").trim().to_string();
        let separator = if style.is_empty() || style.ends_with(';') {
            ""
        } else {
            ";"
        };
        attributes.insert("style", format!("{style}{separator}{}", token(index)));
        ids.push(id);
    }
    if ids.is_empty() {
        bail!("data-hf-draw found no shapes (path, line, polyline, polygon, rect, circle, ellipse) to draw");
    }
    Ok(Some((ids, timing)))
}

/// Whether a shape only defines a resource (inside `<defs>`, `<clipPath>`, `<mask>`, `<symbol>`,
/// `<marker>`, or `<pattern>`) rather than being drawn where it stands.
fn in_resource(node: &NodeRef) -> bool {
    node.ancestors().any(|ancestor| {
        ancestor.as_element().is_some_and(|el| {
            matches!(
                &*el.name.local,
                "defs" | "clipPath" | "clippath" | "mask" | "symbol" | "marker" | "pattern"
            )
        })
    })
}
