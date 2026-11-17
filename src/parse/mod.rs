//! HTML/CSS parser: turns a composition file into the `model` types, rejecting unsupported features explicitly.
//!
//! `parse_file` (and `parse_file_with`, which takes variable values) is the facade. Submodules: `html` (node helpers), `css` (stylesheet, selectors, cascade,
//! variables), `values` (value parsers), `style` (property groups into `Style`), `animation`
//! (CSS animation, `data-hf-keyframes`, and `data-hf-camera` into motion tracks), `split` (`data-hf-split` kinetic text), `audio` (`<audio>` clips for the mixer), `speed` (`data-hf-speed` and `data-hf-speed-ramp` on `<video>` and `<audio>`), `captions` (`data-hf-captions` timed captions from a transcript), `counter` (`data-hf-counter` animated numbers; `data-hf-reveal` lives in `animation/reveal.rs`), `transition` (`data-hf-transition` scene transitions), `variables` (`data-composition-variables` declarations and their `data-var-*` bindings, applied to the DOM first), `tree` (DOM walk), `timeline` (`data-start`).

mod animation;
mod audio;
mod audio_effects;
mod captions;
mod counter;
mod css;
mod html;
mod speed;
mod split;
mod style;
mod timeline;
mod transition;
mod tree;
mod values;
mod variables;
mod video;

pub use values::color::parse_color;
pub use variables::{read_values_file, values_from_json, VariableValues};

use crate::model::{Background, BlendMode, Color, Composition, Content, MotionBlur};
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

/// Parses a composition with every variable at its declared default.
pub fn parse_file(path: &Path) -> Result<Composition> {
    parse_file_with(path, &VariableValues::new())
}

/// Parses a composition with `values` for its declared variables (`data-composition-variables`).
pub fn parse_file_with(path: &Path, values: &VariableValues) -> Result<Composition> {
    let html =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let document = kuchiki::parse_html().one(html);
    if let Some(unsupported) = document
        .select("script, canvas, iframe")
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
    // Variables rewrite the document (text, sources, removed elements) before anything reads it.
    let applied = variables::apply(&document, metadata.as_node(), &root, values)?;
    let mut sheet = Stylesheet {
        pinned: applied.pinned,
        ..Stylesheet::default()
    };
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
    audio_effects::resolve_ducking(&mut audio)?;
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
    let motion_blur = parse_motion_blur(metadata.as_node())?;
    let mut grade = Vec::new();
    let inferred_duration = pending
        .iter()
        .enumerate()
        .filter_map(|(idx, p)| {
            p.duration
                .or_else(|| {
                    if p.start_expr.is_some()
                        && matches!(p.element.content, Content::Image(_) | Content::InlineSvg(_))
                    {
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
        } else if p.start_expr.is_some()
            && matches!(p.element.content, Content::Image(_) | Content::InlineSvg(_))
        {
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
        let label = p
            .element
            .id
            .clone()
            .unwrap_or_else(|| "element with no id".to_string());
        let label = label.as_str();
        let rest = p.element.motion.base;
        let filter_rest = filter_base(&p.element.style);
        if let Some(spec) = &p.animation {
            p.element.motion = build_motion(spec, rest, filter_rest, clip_len)
                .with_context(|| format!("CSS animation on {label}"))?;
        } else if !p.keyframes.is_empty() {
            if p.keyframes
                .last()
                .is_some_and(|frame| frame.at > clip_len + 1e-9)
            {
                bail!("data-hf-keyframes exceeds the duration of {label}");
            }
            p.element.motion = motion_from_keyframes(rest, filter_rest, &p.keyframes);
            clip_from_keyframes(&mut p.element, &p.keyframes)
                .with_context(|| format!("data-hf-keyframes clipPath on {label}"))?;
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
        order_animated_filters(&mut p.element, idx == 0)
            .with_context(|| format!("animated filter on {label}"))?;
        if idx == 0 {
            let style = &mut p.element.style;
            if style.filter_blur > 0.0 {
                bail!("filter: blur cannot be on the root; add a container inside it");
            }
            if style.drop_shadow.is_some() {
                bail!("filter: drop-shadow cannot be on the root; add a container inside it");
            }
            if style.blend != BlendMode::Normal {
                bail!("mix-blend-mode cannot be on the root; add a container inside it");
            }
            if style
                .filter_colors
                .iter()
                .any(|f| matches!(f, crate::model::ColorFilter::Opacity(_)))
            {
                bail!("filter: opacity() cannot be on the root; use it on a container inside it");
            }
            // Color functions on the root grade every finished frame instead of a layer.
            grade = std::mem::take(&mut style.filter_colors);
        }
        check_clip_and_mask(&p.element, idx == 0)
            .with_context(|| format!("clip-path or mask-image on {label}"))?;
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
    // Light and focus transitions add filter tracks to scenes that had none.
    for element in elements.iter_mut().skip(1) {
        let label = element
            .id
            .clone()
            .unwrap_or_else(|| "element with no id".into());
        order_animated_filters(element, false)
            .with_context(|| format!("animated filter on {label}"))?;
    }
    check_blend_isolation(&elements)?;
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
        grade,
        motion_blur,
        elements,
        ids,
        audio,
        variables: applied.report,
    })
}

/// `data-motion-blur="on"` with optional `data-motion-blur-shutter` (degrees, default 180) and
/// `data-motion-blur-samples` (2 to 64; automatic when omitted) on the composition root.
fn parse_motion_blur(node: &kuchiki::NodeRef) -> Result<Option<MotionBlur>> {
    let shutter = attr(node, "data-motion-blur-shutter")
        .map(|raw| number(&raw, "data-motion-blur-shutter"))
        .transpose()?;
    let samples = attr(node, "data-motion-blur-samples")
        .map(|raw| number(&raw, "data-motion-blur-samples"))
        .transpose()?;
    match attr(node, "data-motion-blur").as_deref().map(str::trim) {
        None | Some("off") => {
            if shutter.is_some() || samples.is_some() {
                bail!("data-motion-blur-shutter and -samples need data-motion-blur=\"on\"");
            }
            return Ok(None);
        }
        Some("on" | "") => {}
        Some(other) => bail!("data-motion-blur must be on or off, not {other}"),
    }
    let shutter = shutter.unwrap_or(180.0);
    if !(shutter > 0.0 && shutter <= 720.0) {
        bail!("data-motion-blur-shutter must be above 0 and at most 720 degrees");
    }
    if samples.is_some_and(|n| n.fract() != 0.0 || !(2.0..=64.0).contains(&n)) {
        bail!("data-motion-blur-samples must be a whole number from 2 to 64");
    }
    Ok(Some(MotionBlur {
        shutter: shutter as f32,
        samples: samples.map(|n| n as u32),
    }))
}

/// `data-hf-keyframes` `clipPath`: one track per shape value. The static `clip-path` (or the first
/// keyframe's shape when there is none) fixes the function, point count, and units; keyframes
/// without `clipPath` inherit the previous values.
fn clip_from_keyframes(
    element: &mut crate::model::Element,
    frames: &[crate::model::Keyframe],
) -> Result<()> {
    use crate::model::{Track, TrackPoint};
    let Some(first) = frames.iter().find_map(|k| k.clip.as_ref()) else {
        return Ok(());
    };
    let base = element
        .style
        .clip_path
        .get_or_insert_with(|| first.clone())
        .clone();
    for shape in frames.iter().filter_map(|k| k.clip.as_ref()) {
        crate::parse::values::clip::check_compatible(&base, shape)?;
    }
    let mut values = base.values();
    let mut tracks = vec![Track::default(); values.len()];
    for frame in frames {
        if let Some(shape) = &frame.clip {
            values = shape.values();
        }
        for (track, &value) in tracks.iter_mut().zip(&values) {
            track.0.push(TrackPoint {
                at: frame.at,
                value,
                easing: frame.easing,
            });
        }
    }
    element.motion.clip = tracks;
    Ok(())
}

/// `clip-path` and `mask-image` draw through an offscreen layer that does not rotate.
fn check_clip_and_mask(element: &crate::model::Element, root: bool) -> Result<()> {
    let style = &element.style;
    if style.clip_path.is_none() && style.mask.is_none() {
        return Ok(());
    }
    if root {
        bail!("not supported on the root; add a full-size container inside it");
    }
    if style.rotate != 0.0
        || element.motion.base.rotate != 0.0
        || !element.motion.rotate.0.is_empty()
    {
        bail!("cannot be combined with rotate on the same element; wrap the rotated element in a container");
    }
    if style.blend != BlendMode::Normal {
        bail!("cannot be combined with mix-blend-mode on the same element; wrap it in a container");
    }
    Ok(())
}

/// Blend modes need the real backdrop, which an ancestor drawn through an offscreen layer hides.
fn check_blend_isolation(elements: &[crate::model::Element]) -> Result<()> {
    for element in elements
        .iter()
        .filter(|e| e.style.blend != BlendMode::Normal)
    {
        let mut parent = element.parent;
        while let Some(idx) = parent {
            let ancestor = &elements[idx];
            let style = &ancestor.style;
            // The root is drawn without rotation, wipe, or filter layers (those are rejected or
            // ignored there), but `overflow: hidden` still renders its content on a layer.
            let layered = style.overflow_hidden
                || (idx != 0
                    && (!ancestor.motion.filter.is_static()
                        || style.rotate != 0.0
                        || ancestor.motion.base.rotate != 0.0
                        || !ancestor.motion.rotate.0.is_empty()
                        || ancestor.motion.wipe.is_some()
                        || style.filter_blur > 0.0
                        || !style.filter_colors.is_empty()
                        || style.drop_shadow.is_some()
                        || style.clip_path.is_some()
                        || style.mask.is_some()
                        || !ancestor.motion.fx.is_static()
                        || style.blend != BlendMode::Normal));
            if layered {
                let name = |e: &crate::model::Element| {
                    e.id.as_ref()
                        .map_or_else(|| format!("<{}>", e.tag), |id| format!("#{id}"))
                };
                bail!(
                    "mix-blend-mode on {} cannot be inside {} (overflow: hidden, rotation, a reveal, a wipe, a filter, a clip-path or mask, or a blend mode); move it out of that element",
                    name(element),
                    name(ancestor)
                );
            }
            parent = ancestor.parent;
        }
    }
    Ok(())
}

/// Resting amount of each animatable filter channel: the static `filter` value, else neutral.
pub(in crate::parse) fn filter_base(style: &crate::model::Style) -> [f32; 8] {
    use crate::model::FilterChannel;
    let mut base = FilterChannel::ALL.map(FilterChannel::neutral);
    base[FilterChannel::Blur.index()] = style.filter_blur;
    for &filter in &style.filter_colors {
        if let Some(channel) = FilterChannel::of(filter) {
            base[channel.index()] = FilterChannel::amount(filter);
        }
    }
    base
}

/// Checks an element's animated filter channels and lists those its static `filter` lacks (they
/// apply after the static functions, in `FilterChannel::ALL` order).
fn order_animated_filters(element: &mut crate::model::Element, root: bool) -> Result<()> {
    use crate::model::FilterChannel;
    let filter = &mut element.motion.filter;
    filter.appended.clear();
    if filter.is_static() {
        return Ok(());
    }
    if root {
        bail!("filters cannot be animated on the root; animate a full-size container inside it");
    }
    for channel in FilterChannel::ALL.into_iter().skip(1) {
        if !filter.is_animated(channel) {
            continue;
        }
        let listed = element
            .style
            .filter_colors
            .iter()
            .filter(|&&f| FilterChannel::of(f) == Some(channel))
            .count();
        match listed {
            0 => filter.appended.push(channel),
            1 => {}
            _ => bail!(
                "{} is animated but appears {listed} times in the static filter; list it once",
                channel.key()
            ),
        }
    }
    Ok(())
}
