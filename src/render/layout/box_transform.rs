//! Resolves the parts of `transform` that depend on the element's own box once a composition is
//! parsed, from its resting border box (measured with the renderer's own placement):
//! - percent translations (`translate(-50%, -50%)`): a static one becomes a constant
//!   `Motion::offset`, or, under a CSS animation of `transform` (which replaces it, as in CSS),
//!   part of the resting pose; an animated one (`@keyframes`) is folded point by point into the
//!   `x` and `y` tracks, which share its point layout;
//! - `transform-origin`: becomes `Motion::origin`, the pivot's px offset from the box center,
//!   which `Motion::pose` turns into a translation.
//!
//! Rendering stays a pure function of time, and an animated size does not move either one. It
//! also rejects a skew or non-uniform scale (`Motion::shape`) where a 2.5D plane would be
//! projected without it.

use crate::model::Composition;
use crate::render::Renderer;
use anyhow::{bail, Result};

/// Fills `Motion::offset` (and folds animated percent tracks) for every element with a percent
/// translation, and `Motion::origin` for every element with a `transform-origin`.
pub fn resolve_box_transforms(composition: &mut Composition) -> Result<()> {
    check_shapes_outside_planes(composition)?;
    let pending: Vec<usize> = composition
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.style.translate_percent != [0.0; 4]
                || e.motion.percent.is_some()
                || e.style.transform_origin.is_some()
        })
        .map(|(idx, _)| idx)
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    let mut offsets = Vec::with_capacity(pending.len());
    {
        let snapshot: &Composition = composition;
        let mut renderer = Renderer::new(snapshot, 1.0)?;
        for &idx in &pending {
            let element = &snapshot.elements[idx];
            let label = || {
                element
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("<{}> with no id", element.tag))
            };
            let what = if element.style.transform_origin.is_some() {
                "transform-origin"
            } else {
                "% translation"
            };
            if element.parent.is_none() {
                bail!("transform: {what} is not supported on the root");
            }
            if element.motion.rig.is_some() || element.depth.stage.is_some() {
                bail!(
                    "transform on {}: {what} is not supported on a camera or data-hf-space stage{}",
                    label(),
                    if what == "transform-origin" {
                        "; a camera zooms and turns around its center"
                    } else {
                        "; use px"
                    }
                );
            }
            let rect = renderer.rest_rect(idx)?;
            offsets.push((idx, (rect.w, rect.h)));
        }
    }
    for (idx, (w, h)) in offsets {
        let element = &mut composition.elements[idx];
        let [xw, xh, yw, yh] = element.style.translate_percent;
        let origin = element.style.transform_origin;
        let motion = &mut element.motion;
        let resolved = (xw * w + xh * h, yw * w + yh * h);
        if motion.percent.is_some() {
            // A CSS animation of transform replaces the static one: inside it the percent tracks
            // carry the static part (from its implied end keyframes); outside it, the resting
            // pose does.
            motion.base.x += resolved.0;
            motion.base.y += resolved.1;
        } else {
            motion.offset = resolved;
        }
        if let Some([ox, oy]) = origin {
            motion.origin = (ox.resolve(w) - w / 2.0, oy.resolve(h) - h / 2.0);
        }
        if let Some(parts) = motion.percent.take() {
            let [pxw, pxh, pyw, pyh] = *parts;
            for (track, along_w, along_h) in [(&mut motion.x, pxw, pxh), (&mut motion.y, pyw, pyh)]
            {
                if track.0.len() != along_w.0.len() || track.0.len() != along_h.0.len() {
                    bail!("internal: percent translation tracks do not match the x/y tracks");
                }
                for ((point, a), b) in track.0.iter_mut().zip(&along_w.0).zip(&along_h.0) {
                    point.value += a.value * w + b.value * h;
                }
            }
        }
    }
    Ok(())
}

/// A `data-hf-space` plane is placed by its scale, roll, and 3D values only, so a skewed or
/// non-uniformly scaled plane (or stage) would lose its shape: reject it instead. Elements drawn
/// flat inside a plane keep it.
fn check_shapes_outside_planes(composition: &Composition) -> Result<()> {
    let elements = &composition.elements;
    for element in elements.iter().filter(|e| e.motion.shape.is_some()) {
        let plane = element.depth.plane.is_some()
            || element.depth.stage.is_some()
            || element
                .parent
                .is_some_and(|parent| elements[parent].depth.stage.is_some());
        if plane {
            let label = element
                .id
                .clone()
                .unwrap_or_else(|| format!("<{}> with no id", element.tag));
            bail!(
                "transform on {label}: skew() or non-uniform scale() cannot be on a data-hf-space stage or plane; put it on an element inside the plane"
            );
        }
    }
    Ok(())
}
