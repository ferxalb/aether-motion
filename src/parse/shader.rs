//! `data-hf-shader` procedural layers: reads the program, `data-hf-shader-uniforms` (a JSON
//! object of numbers, 2 to 4 number arrays, and CSS colors, `var(--name)` of the element's custom
//! properties included), `data-hf-shader-keyframes` (uniform
//! values over clip time, with easing), `data-hf-shader-resolution`, and
//! `data-hf-shader-dither`, and compiles them into a `crate::shader::Shader`.

use crate::model::{Track, TrackPoint};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::css::vars::substitute;
use crate::parse::css::Vars;
use crate::parse::html::attr;
use crate::parse::values::color::parse_color;
use crate::shader::{Shader, Uniform};
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use serde_json::Value;

const OPTIONS: [&str; 4] = [
    "data-hf-shader-uniforms",
    "data-hf-shader-keyframes",
    "data-hf-shader-resolution",
    "data-hf-shader-dither",
];
/// Most keyframes a shader can have.
const MAX_KEYFRAMES: usize = 1000;

pub(in crate::parse) fn is_host(node: &NodeRef) -> bool {
    attr(node, "data-hf-shader").is_some()
}

/// Reads and compiles the shader of `node`, which has `data-hf-shader`.
pub(in crate::parse) fn parse_shader(node: &NodeRef, tag: &str, vars: &Vars) -> Result<Shader> {
    let source = attr(node, "data-hf-shader").unwrap_or_default();
    if source.trim().is_empty() {
        bail!(
            "data-hf-shader needs a program, for example data-hf-shader=\"rgb(uv.x, uv.y, 0.5)\""
        );
    }
    if !matches!(
        tag,
        "div" | "section" | "main" | "article" | "aside" | "header" | "footer"
    ) {
        bail!("data-hf-shader belongs on a <div> (or another block container), not <{tag}>");
    }
    for conflict in [
        "data-hf-3d",
        "data-hf-space",
        "data-hf-split",
        "data-hf-counter",
        "data-hf-captions",
        "data-hf-sequence",
    ] {
        if attr(node, conflict).is_some() {
            bail!("data-hf-shader cannot be combined with {conflict} on the same element; wrap it");
        }
    }
    if !super::html::direct_text(node).is_empty() {
        bail!("an element with data-hf-shader cannot have its own text; put the text in a child element");
    }
    let element = node.as_element().expect("element");
    for key in element.attributes.borrow().map.keys() {
        let key = &*key.local;
        if key.starts_with("data-hf-shader-") && !OPTIONS.contains(&key) {
            bail!(
                "unknown attribute {key}; data-hf-shader takes {}",
                OPTIONS.join(", ")
            );
        }
    }
    let resolution = match attr(node, "data-hf-shader-resolution")
        .as_deref()
        .map(str::trim)
    {
        None | Some("1") => 1.0,
        Some("0.5") => 0.5,
        Some("0.25") => 0.25,
        Some(other) => bail!("data-hf-shader-resolution must be 1, 0.5, or 0.25 (got {other})"),
    };
    let dither = match attr(node, "data-hf-shader-dither")
        .as_deref()
        .map(str::trim)
    {
        None | Some("on") => true,
        Some("off") => false,
        Some(other) => bail!("data-hf-shader-dither must be on or off (got {other})"),
    };
    let mut uniforms = match attr(node, "data-hf-shader-uniforms") {
        Some(raw) => parse_uniforms(&raw, Some(vars)).context("data-hf-shader-uniforms")?,
        None => Vec::new(),
    };
    if let Some(raw) = attr(node, "data-hf-shader-keyframes") {
        apply_keyframes(&raw, &mut uniforms, Some(vars)).context("data-hf-shader-keyframes")?;
    }
    Shader::compile(&source, uniforms, resolution, dither).context("data-hf-shader")
}

/// A uniform value: a number, an array of 2 to 4 numbers, or a CSS color (3 components, or an
/// array for alpha).
fn uniform_value(name: &str, value: &Value, vars: Option<&Vars>) -> Result<Vec<f32>> {
    let finite = |n: f64| -> Result<f32> {
        if !n.is_finite() || n.abs() > 1e9 {
            bail!("uniform {name} must be a finite number of at most 1e9 in magnitude");
        }
        Ok(n as f32)
    };
    match value {
        Value::Number(n) => Ok(vec![finite(n.as_f64().unwrap_or(f64::NAN))?]),
        Value::Array(items) if (2..=4).contains(&items.len()) => items
            .iter()
            .map(|item| {
                item.as_f64()
                    .ok_or_else(|| anyhow!("uniform {name} must be an array of numbers"))
                    .and_then(finite)
            })
            .collect(),
        Value::String(raw) => {
            let resolved = match vars {
                Some(vars) if raw.contains("var(") => substitute(raw, vars, 0)?,
                _ => raw.clone(),
            };
            let color = parse_color(resolved.trim()).with_context(|| format!("uniform {name}"))?;
            if color.a != 255 {
                bail!("uniform {name}: colors are rgb; give a [r, g, b, a] array (0 to 1) for transparency");
            }
            Ok([color.r, color.g, color.b]
                .iter()
                .map(|&c| f32::from(c) / 255.0)
                .collect())
        }
        _ => bail!(
            "uniform {name} must be a number, an array of 2 to 4 numbers, or a CSS color string"
        ),
    }
}

fn parse_uniforms(raw: &str, vars: Option<&Vars>) -> Result<Vec<Uniform>> {
    let object: serde_json::Map<String, Value> =
        serde_json::from_str(raw).context("expected a JSON object such as {\"speed\": 0.5}")?;
    object
        .iter()
        .map(|(name, value)| {
            Ok(Uniform {
                name: name.clone(),
                value: uniform_value(name, value, vars)?,
                tracks: Vec::new(),
            })
        })
        .collect()
}

/// Adds keyframe tracks to declared uniforms. Each keyframe has `at` (clip seconds), optional
/// `easing` (for the segment that ends at it), and values for any declared uniforms.
fn apply_keyframes(raw: &str, uniforms: &mut [Uniform], vars: Option<&Vars>) -> Result<()> {
    let frames: Vec<serde_json::Map<String, Value>> =
        serde_json::from_str(raw).context("expected a JSON array of objects")?;
    if frames.is_empty() || frames.len() > MAX_KEYFRAMES {
        bail!("give 1 to {MAX_KEYFRAMES} keyframes");
    }
    for uniform in uniforms.iter_mut() {
        uniform.tracks = vec![Track::default(); uniform.value.len()];
    }
    let mut previous = -1.0;
    for (index, frame) in frames.iter().enumerate() {
        (|| -> Result<()> {
            let at = frame
                .get("at")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("at is required and must be a number"))?;
            if !at.is_finite() || at < 0.0 || at <= previous {
                bail!("times must be non-negative and increasing");
            }
            previous = at;
            let easing = match frame.get("easing") {
                None => Default::default(),
                Some(value) => parse_keyframe_easing(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("easing must be a string"))?,
                )?,
            };
            for (key, value) in frame {
                if key == "at" || key == "easing" {
                    continue;
                }
                let uniform = uniforms
                    .iter_mut()
                    .find(|u| &u.name == key)
                    .ok_or_else(|| {
                        anyhow!("{key} is not a declared uniform; declare it in data-hf-shader-uniforms")
                    })?;
                let values = uniform_value(key, value, vars)?;
                if values.len() != uniform.value.len() {
                    bail!(
                        "{key} has {} components, but its declaration has {}",
                        values.len(),
                        uniform.value.len()
                    );
                }
                for (track, value) in uniform.tracks.iter_mut().zip(values) {
                    track.0.push(TrackPoint { at, value, easing });
                }
            }
            Ok(())
        })()
        .with_context(|| format!("keyframe {}", index + 1))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_uniforms_of_every_kind() {
        let uniforms = parse_uniforms(
            r##"{"speed": 0.5, "center": [0.25, 0.75], "tint": "#ff8000"}"##,
            None,
        )
        .unwrap();
        assert_eq!(uniforms.len(), 3);
        let tint = uniforms.iter().find(|u| u.name == "tint").unwrap();
        assert_eq!(tint.value, vec![1.0, 128.0 / 255.0, 0.0]);
        assert!(parse_uniforms(r#"{"bad": true}"#, None).is_err());
        let vars = Vars::from([("--accent".to_string(), "#00ff00".to_string())]);
        let themed = parse_uniforms(r#"{"tint": "var(--accent, #ff0000)"}"#, Some(&vars)).unwrap();
        assert_eq!(themed[0].value, vec![0.0, 1.0, 0.0]);
        assert!(parse_uniforms(r#"{"bad": [1, 2, 3, 4, 5]}"#, None).is_err());
    }

    #[test]
    fn keyframes_build_tracks_for_declared_uniforms() {
        let mut uniforms = parse_uniforms(r#"{"glow": 0, "center": [0.5, 0.5]}"#, None).unwrap();
        apply_keyframes(
            r#"[{"at": 0, "glow": 0}, {"at": 1.5, "glow": 1, "center": [0.2, 0.8], "easing": "ease-out"}]"#,
            &mut uniforms,
            None,
        )
        .unwrap();
        let glow = uniforms.iter().find(|u| u.name == "glow").unwrap();
        assert_eq!(glow.tracks[0].0.len(), 2);
        let mut uniforms = parse_uniforms(r#"{"glow": 0}"#, None).unwrap();
        for bad in [
            r#"[{"at": 1, "other": 2}]"#,
            r#"[{"at": 1, "glow": [1, 2]}]"#,
            r#"[{"at": 1, "glow": 1}, {"at": 0.5, "glow": 0}]"#,
        ] {
            assert!(apply_keyframes(bad, &mut uniforms, None).is_err(), "{bad}");
        }
    }
}
