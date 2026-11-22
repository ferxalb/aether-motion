//! Resolves percent `transform` translations (`translate(-50%, -50%)`) once a composition is
//! parsed: each element's percent part is multiplied by its resting border box (measured with the
//! renderer's own placement). A static one becomes a constant `Motion::offset`; an animated one
//! (`@keyframes`) is folded point by point into the `x` and `y` tracks, which share its point
//! layout. Rendering stays a pure function of time, and an animated size does not move it.

use crate::model::Composition;
use crate::render::Renderer;
use anyhow::{bail, Result};

/// Fills `Motion::offset` for every element whose static `transform` has a percent translation.
pub fn resolve_percent_translations(composition: &mut Composition) -> Result<()> {
    let pending: Vec<usize> = composition
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| e.style.translate_percent != [0.0; 4] || e.motion.percent.is_some())
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
            if element.parent.is_none() {
                bail!("transform: % translation is not supported on the root");
            }
            if element.motion.rig.is_some() || element.depth.stage.is_some() {
                bail!(
                    "transform on {}: % translation is not supported on a camera or data-hf-space stage; use px",
                    label()
                );
            }
            let rect = renderer.rest_rect(idx)?;
            offsets.push((idx, (rect.w, rect.h)));
        }
    }
    for (idx, (w, h)) in offsets {
        let element = &mut composition.elements[idx];
        let [xw, xh, yw, yh] = element.style.translate_percent;
        let motion = &mut element.motion;
        motion.offset = (xw * w + xh * h, yw * w + yh * h);
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
