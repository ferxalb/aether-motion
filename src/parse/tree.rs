//! DOM walk: validates elements, resolves styles, and collects clips in document order.
use crate::model::{Content, Element, Keyframe, Motion, Pose};
use crate::parse::animation::camera::{parse_camera, CameraFrame};
use crate::parse::animation::keyframes::parse_keyframes;
use crate::parse::animation::AnimationSpec;
use crate::parse::css::{Stylesheet, Vars};
use crate::parse::html::{attr, direct_text, number};
use crate::parse::split::parse_split;
use crate::parse::transition::{parse_transition, TransitionSpec};
use crate::parse::style::{style_for, Resolved};
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
    pub(in crate::parse) animation: Option<AnimationSpec>,
}

pub(in crate::parse) fn walk(
    node: &NodeRef,
    parent: Option<usize>,
    base: &Path,
    sheet: &Stylesheet,
    vars: &Vars,
    out: &mut Vec<Pending>,
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
    if matches!(
        tag.as_str(),
        "script" | "video" | "audio" | "canvas" | "svg" | "iframe"
    ) {
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
            | "img"
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
    } = style_for(node, sheet, vars).with_context(|| format!("styles for <{tag}>"))?;
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
    let content = if tag == "img" {
        let src = attr(node, "src").ok_or_else(|| anyhow!("<img> requires src"))?;
        if src.starts_with("data:") || src.contains("://") {
            bail!("only local images are supported: {src}");
        }
        Content::Image(base.join(src))
    } else {
        let text = direct_text(node);
        if text.is_empty() {
            Content::Container
        } else {
            Content::Text(text)
        }
    };
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
        if text.contains('\n')
            || node
                .children()
                .any(|c| c.as_element().is_some_and(|e| &*e.name.local != "br"))
        {
            bail!("data-hf-split supports one line of plain text (no <br> or child elements)");
        }
        split_text = Some(text.clone());
        spec.apply_container_style(&mut style);
        Content::Container
    } else {
        content
    };
    let idx = out.len();
    let rest = Pose {
        x: style.translate.0,
        y: style.translate.1,
        scale: style.scale.unwrap_or(1.0),
        rotate: style.rotate,
        opacity: style.opacity.unwrap_or(1.0),
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
        },
        start_expr,
        duration,
        keyframes,
        camera,
        transition,
        animation,
    });
    if let Some(parent_idx) = parent {
        out[parent_idx].element.children.push(idx);
    }
    if let (Some(spec), Some(text)) = (&split, &split_text) {
        return spec.expand(text, idx, out);
    }
    for child in node.children() {
        walk(&child, Some(idx), base, sheet, &vars, out)?;
    }
    Ok(())
}
