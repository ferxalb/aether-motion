//! DOM walk: validates elements, resolves styles, and collects clips in document order.
use crate::model::{Content, Element, Keyframe, LayoutHints, Motion, Pose};
use crate::parse::animation::camera::{parse_camera, CameraFrame};
use crate::parse::animation::keyframes::parse_keyframes;
use crate::parse::animation::reveal::{parse_reveal, RevealSpec};
use crate::parse::animation::AnimationSpec;
use crate::parse::audio::{parse_audio, PendingAudio};
use crate::parse::captions::parse_captions;
use crate::parse::counter::parse_counter;
use crate::parse::css::{Stylesheet, Vars};
use crate::parse::html::{attr, direct_text, number};
use crate::parse::rich_text::{collect as collect_rich_text, has_inline_runs};
use crate::parse::split::parse_split;
use crate::parse::style::{style_for, Resolved};
use crate::parse::transition::{parse_transition, TransitionSpec};
use crate::parse::vector::{parse_motion_path, prepare_draw};
use crate::parse::video::parse_video;
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::path::Path;

#[derive(Debug)]
pub(in crate::parse) struct Pending {
    pub(in crate::parse) element: Element,
    pub(in crate::parse) start_expr: Option<String>,
    pub(in crate::parse) duration: Option<f64>,
    pub(in crate::parse) keyframes: Vec<Keyframe>,
    pub(in crate::parse) camera: Vec<CameraFrame>,
    pub(in crate::parse) transition: Option<TransitionSpec>,
    pub(in crate::parse) reveal: Option<RevealSpec>,
    pub(in crate::parse) animation: Option<AnimationSpec>,
    pub(in crate::parse) motion_path: Option<crate::model::MotionPath>,
}

pub(in crate::parse) fn walk(
    node: &NodeRef,
    parent: Option<usize>,
    base: &Path,
    sheet: &Stylesheet,
    vars: &Vars,
    out: &mut Vec<Pending>,
    audio: &mut Vec<PendingAudio>,
) -> Result<()> {
    let Some(el) = node.as_element() else {
        return Ok(());
    };
    let tag = el.name.local.to_string();
    // `<br>` is consumed by the parent's text (see `direct_text`); it is not an element of its own.
    if matches!(
        tag.as_str(),
        "head" | "meta" | "style" | "title" | "link" | "br"
    ) {
        return Ok(());
    }
    if tag == "audio" {
        if node.children().any(|c| c.as_element().is_some()) {
            bail!("<audio> cannot have child elements; set the file with src");
        }
        audio.push(parse_audio(node, parent, base)?);
        return Ok(());
    }
    if matches!(tag.as_str(), "script" | "canvas" | "iframe") {
        bail!("<{tag}> requires a browser/JavaScript and is not supported by the native backend");
    }
    if !matches!(
        tag.as_str(),
        "html"
            | "body"
            | "main"
            | "section"
            | "article"
            | "div"
            | "h1"
            | "h2"
            | "h3"
            | "p"
            | "span"
            | "small"
            | "strong"
            | "b"
            | "em"
            | "i"
            | "code"
            | "img"
            | "svg"
            | "video"
    ) {
        bail!("unsupported HTML element: <{tag}>");
    }
    if attr(node, "data-composition-src").is_some() {
        bail!("data-composition-src is not yet supported");
    }
    let Resolved {
        mut style,
        vars,
        animation,
    } = style_for(node, sheet, vars, parent.is_none())
        .with_context(|| format!("styles for <{tag}>"))?;
    if matches!(tag.as_str(), "em" | "i") {
        style.font_italic = style.font_italic.or(Some(true));
    }
    let start_expr = attr(node, "data-start");
    let duration = attr(node, "data-duration")
        .map(|s| number(&s, "data-duration"))
        .transpose()?;
    let fade_in = attr(node, "data-hf-fade-in")
        .map(|s| number(&s, "data-hf-fade-in"))
        .transpose()?
        .unwrap_or(0.0);
    let fade_out = attr(node, "data-hf-fade-out")
        .map(|s| number(&s, "data-hf-fade-out"))
        .transpose()?
        .unwrap_or(0.0);
    let keyframes = attr(node, "data-hf-keyframes")
        .map(|raw| parse_keyframes(&raw))
        .transpose()
        .with_context(|| {
            format!(
                "data-hf-keyframes on <{tag}> ({})",
                attr(node, "id").as_deref().unwrap_or("no id")
            )
        })?
        .unwrap_or_default();
    let camera = attr(node, "data-hf-camera")
        .map(|raw| parse_camera(&raw))
        .transpose()
        .with_context(|| {
            format!(
                "data-hf-camera on <{tag}> ({})",
                attr(node, "id").as_deref().unwrap_or("no id")
            )
        })?
        .unwrap_or_default();
    if animation.is_some() && !keyframes.is_empty() {
        bail!(
            "<{tag}> ({}) combines CSS animation and data-hf-keyframes; use only one",
            attr(node, "id").as_deref().unwrap_or("no id")
        );
    }
    if !camera.is_empty() && (animation.is_some() || !keyframes.is_empty()) {
        bail!(
            "<{tag}> ({}) combines data-hf-camera with another animation; wrap the element in a camera container",
            attr(node, "id").as_deref().unwrap_or("no id")
        );
    }
    let track = attr(node, "data-track-index")
        .map(|s| s.parse::<i32>().context("invalid data-track-index"))
        .transpose()?
        .unwrap_or(0);
    let mut spans = Vec::new();
    let mut inline_children: Vec<NodeRef> = Vec::new();
    let mut video_length = None;
    let mut video_audio = None;
    let motion_path = parse_motion_path(node).with_context(|| {
        format!(
            "data-hf-motion-path on <{tag}> ({})",
            attr(node, "id").as_deref().unwrap_or("no id")
        )
    })?;
    let draw = prepare_draw(node, &tag)?;
    let content = if tag == "svg" {
        let markup = inline_svg_markup(node)?;
        match draw {
            Some((ids, timing)) => Content::DrawnSvg(std::sync::Arc::new(
                crate::vector::DrawOnSvg::new(&markup, &ids, timing)?,
            )),
            None => Content::InlineSvg(markup.into()),
        }
    } else if tag == "img" {
        let src = attr(node, "src").ok_or_else(|| anyhow!("<img> requires src"))?;
        if src.starts_with("data:") || src.contains("://") {
            bail!("only local images are supported: {src}");
        }
        Content::Image(base.join(src))
    } else if tag == "video" {
        let parsed = parse_video(node, base, duration.is_some()).with_context(|| {
            format!(
                "<video> ({})",
                attr(node, "id").as_deref().unwrap_or("no id")
            )
        })?;
        video_length = Some(parsed.default_duration);
        video_audio = parsed.audio;
        Content::Video(parsed.source)
    } else if has_inline_runs(node) {
        let rich = collect_rich_text(node, sheet, &vars).with_context(|| {
            format!(
                "text of <{tag}> ({})",
                attr(node, "id").as_deref().unwrap_or("no id")
            )
        })?;
        spans = rich.spans;
        inline_children = rich.consumed;
        Content::Text(rich.text)
    } else {
        let text = direct_text(node);
        if text.is_empty() {
            Content::Container
        } else {
            Content::Text(text)
        }
    };
    let duration = duration.or(video_length);
    let reveal = parse_reveal(node).with_context(|| {
        format!(
            "data-hf-reveal on <{tag}> ({})",
            attr(node, "id").as_deref().unwrap_or("no id")
        )
    })?;
    let transition = parse_transition(node).with_context(|| {
        format!(
            "data-hf-transition on <{tag}> ({})",
            attr(node, "id").as_deref().unwrap_or("no id")
        )
    })?;
    let split = parse_split(node, &tag, &style).with_context(|| {
        format!(
            "data-hf-split on <{tag}> ({})",
            attr(node, "id").as_deref().unwrap_or("no id")
        )
    })?;
    let mut split_text = None;
    let content = if let Some(spec) = &split {
        let Content::Text(text) = &content else {
            bail!("data-hf-split requires an element with direct text");
        };
        if node
            .children()
            .any(|c| c.as_element().is_some_and(|e| &*e.name.local != "br"))
        {
            bail!("data-hf-split supports plain text with <br> line breaks (no child elements)");
        }
        split_text = Some(text.clone());
        spec.apply_container_style(&mut style, text);
        Content::Container
    } else {
        content
    };
    let counter = parse_counter(node, &tag).with_context(|| {
        format!(
            "data-hf-counter on <{tag}> ({})",
            attr(node, "id").as_deref().unwrap_or("no id")
        )
    })?;
    if let Some(spec) = &counter {
        if split.is_some() {
            bail!("data-hf-counter cannot be combined with data-hf-split");
        }
        if !matches!(content, Content::Container)
            || node.children().any(|c| c.as_element().is_some())
        {
            bail!("data-hf-counter requires an empty element (no text or child elements)");
        }
        spec.apply_container_style(&mut style);
    }
    let captions = parse_captions(node, &tag, &style, base).with_context(|| {
        format!(
            "data-hf-captions on <{tag}> ({})",
            attr(node, "id").as_deref().unwrap_or("no id")
        )
    })?;
    if captions.is_some() {
        if split.is_some() || counter.is_some() {
            bail!("data-hf-captions cannot be combined with data-hf-split or data-hf-counter");
        }
        if !matches!(content, Content::Container)
            || node.children().any(|c| c.as_element().is_some())
        {
            bail!("data-hf-captions requires an empty element (no text or child elements)");
        }
    }
    let host_style = captions.as_ref().map(|_| style.clone());
    let idx = out.len();
    let rest = Pose {
        x: style.translate.0,
        y: style.translate.1,
        scale: style.scale.unwrap_or(1.0),
        rotate: style.rotate,
        opacity: style.opacity.unwrap_or(1.0),
        reveal: 1.0,
    };
    out.push(Pending {
        element: Element {
            tag,
            id: attr(node, "id"),
            parent,
            children: Vec::new(),
            content,
            style,
            start: 0.0,
            end: f64::INFINITY,
            fade_in,
            fade_out,
            track,
            motion: Motion {
                base: rest,
                ..Motion::default()
            },
            spans,
            layout: LayoutHints {
                allow_safe_zone: flag(node, "data-layout-allow-safe-zone")?,
                allow_caption_zone: flag(node, "data-layout-allow-caption-zone")?,
                ..LayoutHints::default()
            },
        },
        start_expr,
        duration,
        keyframes,
        camera,
        transition,
        reveal,
        animation,
        motion_path,
    });
    if let Some(parent_idx) = parent {
        out[parent_idx].element.children.push(idx);
    }
    if let Some(mut clip) = video_audio {
        // The source's sound plays exactly as long as the clip and starts with it.
        clip.duration = duration;
        audio.push(PendingAudio {
            clip,
            start_expr: None,
            parent: Some(idx),
        });
    }
    if let Some(spec) = &counter {
        return spec.expand(idx, out);
    }
    if let (Some(spec), Some(host)) = (&captions, &host_style) {
        return spec.expand(idx, host, out);
    }
    if let (Some(spec), Some(text)) = (&split, &split_text) {
        return spec.expand(text, idx, out);
    }
    // SVG children belong to the vector graphic, and `<source>` children to the video.
    if matches!(
        out[idx].element.content,
        Content::InlineSvg(_) | Content::DrawnSvg(_) | Content::Video(_)
    ) {
        return Ok(());
    }
    for child in node.children() {
        // Inline runs were folded into this element's text.
        if inline_children.contains(&child) {
            continue;
        }
        walk(&child, Some(idx), base, sheet, &vars, out, audio)?;
    }
    Ok(())
}

/// Boolean marker attribute: present or `true` is on, `false` is off.
fn flag(node: &NodeRef, name: &str) -> Result<bool> {
    match attr(node, name).as_deref() {
        None | Some("false") => Ok(false),
        Some("" | "true") => Ok(true),
        Some(other) => bail!("{name} must be true or false: {other}"),
    }
}

/// Serializes an inline `<svg>` to standalone XML that the SVG rasterizer accepts. Scripts,
/// event handlers, and external references are rejected so output stays deterministic.
fn inline_svg_markup(node: &NodeRef) -> Result<String> {
    for d in node.descendants() {
        let Some(el) = d.as_element() else { continue };
        let name = el.name.local.to_string();
        if matches!(
            name.as_str(),
            "script" | "foreignObject" | "animate" | "set" | "animateTransform" | "animateMotion"
        ) {
            bail!("<{name}> inside <svg> requires a browser/JavaScript and is not supported by the native backend");
        }
        for (key, value) in el.attributes.borrow().map.iter() {
            let key = &*key.local;
            if key.starts_with("on") {
                bail!("event handler {key} inside <svg> requires JavaScript and is not supported");
            }
            if matches!(key, "href") && !value.value.starts_with('#') {
                bail!(
                    "only local #fragment references are supported inside <svg>: {}",
                    value.value
                );
            }
        }
    }
    let mut out = Vec::new();
    node.serialize(&mut out)
        .context("could not serialize <svg>")?;
    let mut markup = String::from_utf8(out).context("<svg> is not valid UTF-8")?;
    if !markup.contains("xmlns=") {
        markup = markup.replacen("<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"", 1);
    }
    Ok(markup)
}
