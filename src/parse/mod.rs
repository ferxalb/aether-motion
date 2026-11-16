//! HTML/CSS parser: turns a composition file into the `model` types, rejecting unsupported features explicitly.
//!
//! `parse_file` is the facade. Submodules: `html` (node helpers), `css` (stylesheet, selectors, cascade,
//! variables), `values` (value parsers), `style` (property groups into `Style`), `animation`
//! (CSS animation, `data-hf-keyframes`, and `data-hf-camera` into motion tracks), `split` (`data-hf-split` kinetic text), `audio` (`<audio>` clips for the mixer), `counter` (`data-hf-counter` animated numbers; `data-hf-reveal` lives in `animation/reveal.rs`), `transition` (`data-hf-transition` scene transitions), `tree` (DOM walk), `timeline` (`data-start`).

mod animation;
mod audio;
mod counter;
mod css;
mod html;
mod split;
mod style;
mod timeline;
mod transition;
mod tree;
mod values;

pub use values::color::parse_color;

use crate::model::{Background, Color, Composition, Content};
use animation::camera::motion_from_camera;
use animation::motion::{build_motion, motion_from_keyframes};
use anyhow::{anyhow, bail, Context, Result};
use css::cascade::cascade;
use css::stylesheet::parse_css;
use css::vars::inherit_vars;
use css::{Stylesheet, Vars};
use html::{attr, number};
use kuchiki::traits::*;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use timeline::{resolve_audio_start, resolve_start};
use transition::apply_transitions;
use tree::walk;

pub fn parse_file(path: &Path) -> Result<Composition> {
    let html =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let document = kuchiki::parse_html().one(html);
    if let Some(unsupported) = document
        .select("script, video, canvas, svg, iframe")
        .map_err(|_| anyhow!("error selecting HTML elements"))?
        .next()
    {
        bail!(
            "<{}> requires a browser/JavaScript and is not supported by the native backend",
            unsupported.name.local
        );
    }
    let metadata = document
        .select_first("[data-composition-id][data-width][data-height]")
        .map_err(|_| anyhow!("missing data-composition-id, data-width, or data-height"))?;
    let width: u32 = attr(metadata.as_node(), "data-width")
        .unwrap()
        .parse()
        .context("invalid data-width")?;
    let height: u32 = attr(metadata.as_node(), "data-height")
        .unwrap()
        .parse()
        .context("invalid data-height")?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        bail!("dimensions are outside 1..8192");
    }
    let mut sheet = Stylesheet::default();
    for node in document
        .select("style")
        .map_err(|_| anyhow!("error selecting CSS"))?
    {
        parse_css(&node.text_contents(), &mut sheet)?;
    }
    // Custom properties declared on `:root` (the <html> element) inherit into the composition.
    let mut root_vars = Vars::new();
    if let Ok(html) = document.select_first("html") {
        let properties = cascade(html.as_node(), &sheet.rules)?;
        root_vars = inherit_vars(&properties, &root_vars);
    }
    let root = if metadata
        .as_node()
        .as_element()
        .is_some_and(|e| e.name.local.as_ref() == "meta")
    {
        document
            .select_first("body")
            .map_err(|_| anyhow!("missing <body>"))?
            .as_node()
            .clone()
    } else {
        metadata.as_node().clone()
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut pending = Vec::new();
    let mut pending_audio = Vec::new();
    walk(
        &root,
        None,
        base,
        &sheet,
        &root_vars,
        &mut pending,
        &mut pending_audio,
    )?;
    if pending.is_empty() {
        bail!("empty composition");
    }
    let mut ids = HashMap::new();
    for (idx, item) in pending.iter().enumerate() {
        if let Some(id) = &item.element.id {
            if ids.insert(id.clone(), idx).is_some() {
                bail!("duplicate id: {id}");
            }
        }
    }
    let mut memo = vec![None; pending.len()];
    let mut visiting = vec![false; pending.len()];
    for idx in 0..pending.len() {
        resolve_start(idx, &pending, &ids, &mut memo, &mut visiting)?;
    }
    let mut audio = Vec::with_capacity(pending_audio.len());
    for item in pending_audio {
        let mut clip = item.clip;
        clip.start = resolve_audio_start(
            item.start_expr.as_deref(),
            item.parent,
            &pending,
            &ids,
            &mut memo,
            &mut visiting,
        )?;
        audio.push(clip);
    }
    let explicit_duration = attr(metadata.as_node(), "data-duration")
        .map(|s| number(&s, "data-duration"))
        .transpose()?;
    let effect = |name: &str| -> Result<f32> {
        let value = attr(metadata.as_node(), name)
            .map(|raw| number(&raw, name))
            .transpose()?
            .unwrap_or(0.0);
        if !(0.0..=1.0).contains(&value) {
            bail!("{name} must be between 0 and 1");
        }
        Ok(value as f32)
    };
    let vignette = effect("data-hf-vignette")?;
    let grain = effect("data-hf-grain")?;
    let inferred_duration = pending
        .iter()
        .enumerate()
        .filter_map(|(idx, p)| {
            p.duration
                .or_else(|| {
                    if p.start_expr.is_some() && matches!(p.element.content, Content::Image(_)) {
                        Some(3.0)
                    } else {
                        None
                    }
                })
                .map(|d| memo[idx].unwrap() + d)
        })
        .chain(
            audio
                .iter()
                .filter_map(|clip| clip.duration.map(|d| clip.start + d)),
        )
        .fold(0.0_f64, f64::max);
    let duration = explicit_duration.unwrap_or(inferred_duration);
    if duration <= 0.0 {
        bail!("unknown duration: add data-duration to the root or clips");
    }
    for clip in &audio {
        let label = clip.id.as_deref().unwrap_or("no id");
        if clip.start >= duration {
            bail!("audio clip {label} starts after the composition ends");
        }
        if clip
            .duration
            .is_some_and(|d| clip.start + d > duration + 1e-9)
        {
            bail!("audio clip {label} exceeds composition duration");
        }
    }
    let mut elements = Vec::with_capacity(pending.len());
    let mut transitions = Vec::new();
    for (idx, mut p) in pending.into_iter().enumerate() {
        p.element.start = memo[idx].unwrap();
        p.element.end = if let Some(d) = p.duration {
            p.element.start + d
        } else if p.start_expr.is_some() && matches!(p.element.content, Content::Image(_)) {
            p.element.start + 3.0
        } else {
            duration
        };
        if p.element.end > duration + 1e-9 {
            bail!(
                "clip {} exceeds composition duration",
                p.element.id.as_deref().unwrap_or("no id")
            );
        }
        if p.element.fade_in + p.element.fade_out > p.element.end - p.element.start + 1e-9 {
            bail!("fade-in + fade-out exceeds the clip duration");
        }
        let clip_len = p.element.end - p.element.start;
        let label = p.element.id.as_deref().unwrap_or("element with no id");
        let rest = p.element.motion.base;
        if let Some(spec) = &p.animation {
            p.element.motion = build_motion(spec, rest, clip_len)
                .with_context(|| format!("CSS animation on {label}"))?;
        } else if !p.keyframes.is_empty() {
            if p.keyframes
                .last()
                .is_some_and(|frame| frame.at > clip_len + 1e-9)
            {
                bail!("data-hf-keyframes exceeds the duration of {label}");
            }
            p.element.motion = motion_from_keyframes(rest, &p.keyframes);
        } else if !p.camera.is_empty() {
            if idx == 0 {
                bail!("data-hf-camera cannot be on the root; add a container inside it");
            }
            if p.camera
                .last()
                .is_some_and(|frame| frame.at > clip_len + 1e-9)
            {
                bail!("data-hf-camera exceeds the duration of {label}");
            }
            p.element.motion = motion_from_camera(rest, &p.camera);
        }
        if idx == 0 && p.element.style.filter_blur > 0.0 {
            bail!("filter: blur cannot be on the root; add a container inside it");
        }
        if let Some(spec) = p.reveal.take() {
            if idx == 0 {
                bail!("data-hf-reveal cannot be on the root; add a container inside it");
            }
            if spec.end() > clip_len + 1e-9 {
                bail!("data-hf-reveal exceeds the duration of {label}");
            }
            if p.element.style.rotate != 0.0 || !p.element.motion.rotate.0.is_empty() {
                bail!("data-hf-reveal cannot be combined with rotate on {label}");
            }
            spec.apply(&mut p.element.motion);
        }
        if let Some(spec) = p.transition.take() {
            transitions.push((idx, spec));
        }
        elements.push(p.element);
    }
    apply_transitions(&mut elements, &transitions, &ids, (width, height))?;
    let background = match elements[0].style.background {
        Some(Background::Solid(color)) => color,
        _ => Color::BLACK,
    };
    Ok(Composition {
        width,
        height,
        duration,
        background,
        vignette,
        grain,
        elements,
        ids,
        audio,
    })
}
