//! HTML/CSS parser: turns a composition file into the `model` types, rejecting unsupported features explicitly.
//!
//! `parse_file` (and `parse_file_with`, which takes variable values) is the facade. Submodules: `html` (node helpers), `css` (stylesheet, selectors, cascade,
//! variables), `values` (value parsers), `style` (property groups into `Style`), `animation`
//! (CSS animation, `data-hf-keyframes`, and `data-hf-camera` into motion tracks), `split/` (`data-hf-split` kinetic text: pieces, entrance motion, line rows, and the wrap plan of the two-pass parse), `effects` (`data-hf-bloom` and per-element `data-hf-motion-blur`), `audio` (`<audio>` clips for the mixer), `speed` (`data-hf-speed` and `data-hf-speed-ramp` on `<video>` and `<audio>`), `captions` (`data-hf-captions` timed captions from a transcript), `counter` (`data-hf-counter` animated numbers; `data-hf-reveal` lives in `animation/reveal.rs`), `transition` (`data-hf-transition` scene transitions), `scenes` (external scene instances), `variables` (`data-composition-variables` declarations and their `data-var-*` bindings, applied to the DOM first), `sequence` (image-sequence indexing), `filmic` (root tone controls), `tree` (DOM walk), `timeline` (`data-start`).

mod animation;
mod audio;
mod audio_effects;
mod captions;
mod counter;
mod css;
mod effects;
mod filmic;
mod html;
mod rich_text;
mod scenes;
mod sequence;
mod speed;
mod split;
mod style;
mod timeline;
mod transition;
mod tree;
mod values;
mod variables;
mod vector;
mod video;

pub use values::color::parse_color;
pub use variables::{read_values_file, values_from_json, VariableValues};

use crate::model::{Background, BlendMode, Color, Composition, Content, MotionBlur};
use animation::camera::motion_from_camera;
use animation::motion::{build_motion, motion_from_keyframes};
use animation::rig::{attach_rigs, keys_from_frames};
use animation::shots::compile_shots;
use anyhow::{anyhow, bail, Context, Result};
use css::cascade::cascade;
use css::stylesheet::parse_css;
use css::vars::inherit_vars;
use css::{Stylesheet, Vars};
use html::{attr, number};
use kuchiki::traits::*;
use split::SplitPlan;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use timeline::{resolve_audio_start, resolve_start};
use transition::apply_transitions;
use tree::{walk, WalkContext};

/// Parses a composition with every variable at its declared default.
pub fn parse_file(path: &Path) -> Result<Composition> {
    parse_file_with(path, &VariableValues::new())
}

/// Parses a composition with `values` for its declared variables (`data-composition-variables`).
///
/// A composition with `data-hf-split-wrap` text is parsed twice: the first parse lays its pieces
/// out on one row per explicit line, the renderer measures where those rows must wrap (with the
/// same fonts and layout it draws with), and the second parse builds the wrapped rows. Parsing is
/// deterministic, so both parses see the same document.
///
/// Camera rigs that follow a subject are bound last: the renderer measures the resting layout,
/// and each subject's path is baked once (see `crate::camera::tracking`).
pub fn parse_file_with(path: &Path, values: &VariableValues) -> Result<Composition> {
    let first = scenes::SceneStack::default().parse(path, values, &SplitPlan::default())?;
    let mut composition = if first.elements.iter().any(|e| e.layout.split_wrap.is_some()) {
        let breaks = crate::render::split_wrap_breaks(&first).context("data-hf-split-wrap")?;
        scenes::SceneStack::default().parse(path, values, &SplitPlan::resolved(breaks))?
    } else {
        first
    };
    crate::render::bind_camera_rigs(&mut composition).context("data-hf-camera-subject")?;
    Ok(composition)
}

/// One parse of the composition, with the wrap breaks of `splits` (empty on the first parse).
fn parse_pass(
    path: &Path,
    values: &VariableValues,
    splits: &SplitPlan,
    stack: &mut scenes::SceneStack,
    imported: bool,
) -> Result<Composition> {
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
    let context = WalkContext {
        base,
        sheet: &sheet,
        splits,
    };
    walk(
        &root,
        None,
        &context,
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
    let host = scenes::Host {
        document: &document,
        root: &root,
        base,
        size: (width, height),
    };
    let instances = scenes::load(&host, &mut pending, &ids, splits, stack)?;
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
    let filmic = filmic::parse(metadata.as_node())?;
    let motion_blur = parse_motion_blur(metadata.as_node())?;
    if imported && (vignette != 0.0 || grain != 0.0 || filmic.is_some() || motion_blur.is_some()) {
        bail!("frame-wide finishing and shutter settings belong on the top-level composition");
    }
    let mut grade = Vec::new();
    let inferred_duration = pending
        .iter()
        .enumerate()
        .filter_map(|(idx, p)| {
            p.duration
                .or_else(|| {
                    if p.start_expr.is_some()
                        && matches!(
                            p.element.content,
                            Content::Image(_)
                                | Content::Sequence(_)
                                | Content::InlineSvg(_)
                                | Content::DrawnSvg(_)
                        )
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
    if imported && (memo[0] != Some(0.0) || pending[0].duration.is_some_and(|d| d != duration)) {
        bail!("an imported scene root must cover its entire local timeline starting at 0");
    }
    let mut elements = Vec::with_capacity(pending.len());
    let mut transitions = Vec::new();
    let mut rigs = Vec::new();
    for (idx, mut p) in pending.into_iter().enumerate() {
        p.element.start = memo[idx].unwrap();
        p.element.end = if let Some(d) = p.duration {
            p.element.start + d
        } else if p.start_expr.is_some()
            && matches!(
                p.element.content,
                Content::Image(_)
                    | Content::Sequence(_)
                    | Content::InlineSvg(_)
                    | Content::DrawnSvg(_)
            )
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
                .frames
                .last()
                .is_some_and(|frame| frame.at > clip_len + 1e-9)
            {
                bail!("data-hf-camera exceeds the duration of {label}");
            }
            if p.camera.is_rig() {
                let options = p.camera.options.clone().unwrap_or_default();
                let keys = if p.camera.shots.is_empty() {
                    keys_from_frames(&p.camera.frames, &options)
                } else {
                    compile_shots(&p.camera.shots, &options, clip_len)
                        .with_context(|| format!("data-hf-shots on {label}"))?
                };
                p.element.motion = crate::model::Motion {
                    base: rest,
                    hold_before: true,
                    hold_after: true,
                    ..Default::default()
                };
                rigs.push((idx, keys, options));
            } else {
                p.element.motion = motion_from_camera(rest, &p.camera.frames);
            }
        }
        if let Some(path) = p.motion_path.take() {
            if idx == 0 {
                bail!("data-hf-motion-path cannot be on the root; add a container inside it");
            }
            if path.at + path.duration > clip_len + 1e-9 {
                bail!("data-hf-motion-path exceeds the duration of {label}");
            }
            p.element.motion.path = Some(std::sync::Arc::new(path));
        }
        if let Content::DrawnSvg(drawing) = &p.element.content {
            if drawing.end() > clip_len + 1e-9 {
                bail!("data-hf-draw on {label} ends at {:.2}s, after the clip ({clip_len:.2}s); shorten -duration or -stagger", drawing.end());
            }
        }
        order_animated_filters(&mut p.element, idx == 0)
            .with_context(|| format!("animated filter on {label}"))?;
        effects::finish_effects(&mut p.element, &p.keyframes, idx == 0)
            .with_context(|| format!("effects on {label}"))?;
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
            if imported && !style.filter_colors.is_empty() {
                bail!("root CSS color filters belong on the top-level composition; grade a container inside the scene instead");
            }
            if imported
                && (!p.element.motion.is_static() || rest.scale != 1.0 || rest.rotate != 0.0)
            {
                bail!("animate, scale, or rotate a container inside the scene instead of its composition root");
            }
            grade = std::mem::take(&mut style.filter_colors);
        }
        if p.element.style.background_clip_text && !matches!(p.element.content, Content::Text(_)) {
            bail!("background-clip: text on {label} needs text of its own (it fills the element's glyphs; descendants are not clipped)");
        }
        if p.element.style.background_clip_text && p.element.style.background.is_none() {
            bail!("background-clip: text on {label} needs a background (for example a linear-gradient)");
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
            if p.element.style.rotate != 0.0 || p.element.motion.rotates() {
                bail!("data-hf-reveal cannot be combined with rotate on {label}");
            }
            spec.apply(&mut p.element.motion);
        }
        if let Some(spec) = p.transition.take() {
            transitions.push((idx, spec));
        }
        elements.push(p.element);
    }
    attach_rigs(&mut elements, rigs, &ids)?;
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
    scenes::append(instances, &mut elements, &mut ids, &mut audio)?;
    check_blend_isolation(&elements)?;
    effects::check_layer_blur(&elements, motion_blur)?;
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
        filmic,
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
    if style.rotate != 0.0 || element.motion.rotates() {
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
                        || ancestor.motion.rotates()
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
