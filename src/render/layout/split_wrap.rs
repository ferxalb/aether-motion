//! Line breaks of `data-hf-split-wrap` kinetic text, measured with the renderer's own fonts and
//! layout between the two parses of a composition (see `parse::split::SplitPlan`).
//!
//! On the first parse each wrapping element is a flex column with one row per explicit line, and
//! each row holds that line's word pieces (or per-word letter groups). This module places the
//! element at rest and at scale 1, the unscaled size line breaks are always decided at, measures
//! every word with the flex sizing used for drawing, and breaks each row greedily where the next
//! word (plus the gap) would pass the element's content width. A word wider than the box keeps a
//! line of its own. The breaks depend on nothing but the composition, so every renderer, thread,
//! and part of a render sees the same lines.

use crate::model::{Composition, Content};
use crate::render::geometry::Rect;
use crate::render::layout::in_flow;
use crate::render::layout::sizing::Room;
use crate::render::Renderer;
use anyhow::{bail, Result};
use std::collections::HashMap;

/// Slack for rounding, in px, when a line exactly fills the box.
const FIT_EPSILON: f32 = 0.01;

/// Word indices (across the element's whole text) that start a wrapped line, by wrap ordinal.
pub fn split_wrap_breaks(composition: &Composition) -> Result<HashMap<u32, Vec<usize>>> {
    let mut renderer = Renderer::new(composition, 1.0)?;
    let mut breaks = HashMap::new();
    for (idx, element) in composition.elements.iter().enumerate() {
        if let Some(ordinal) = element.layout.split_wrap {
            breaks.insert(ordinal, renderer.wrap_breaks(idx)?);
        }
    }
    Ok(breaks)
}

impl<'a> Renderer<'a> {
    /// Border box of `idx` at rest and scale 1, following the static layout from the root.
    pub(in crate::render) fn rest_rect(&mut self, idx: usize) -> Result<Rect> {
        let composition: &'a Composition = self.composition;
        let mut path = vec![idx];
        while let Some(parent) = composition.elements[*path.last().unwrap()].parent {
            path.push(parent);
        }
        path.reverse();
        let root = Rect {
            x: 0.0,
            y: 0.0,
            w: composition.width as f32,
            h: composition.height as f32,
        };
        let mut rect = self.place(0, root, 1.0, None);
        for pair in path.windows(2) {
            let (parent, child) = (pair[0], pair[1]);
            let slot = if in_flow(composition, child) {
                let content = self.content_box(parent, rect, 1.0);
                self.flex_layout(parent, content, 1.0)?
                    .into_iter()
                    .find(|(kid, _)| *kid == child)
                    .map(|(_, slot)| slot)
            } else {
                None
            };
            rect = self.place(child, rect, 1.0, slot);
        }
        Ok(rect)
    }

    /// Width the pieces of wrapping element `idx` may fill: its content width at rest. A flex item
    /// without an explicit `width` is sized by its unwrapped content, so it is capped by the
    /// content width its parent offers, as wrapped text is.
    fn wrap_width(&mut self, idx: usize) -> Result<f32> {
        let composition: &'a Composition = self.composition;
        let rect = self.rest_rect(idx)?;
        let mut width = self.content_box(idx, rect, 1.0).w;
        let element = &composition.elements[idx];
        if in_flow(composition, idx) && element.style.width.is_none() {
            let parent = element.parent.expect("flex items have a parent");
            let parent_rect = self.rest_rect(parent)?;
            let [_, right, _, left] = self.margins(idx, 1.0);
            let (chrome_x, _) = self.chrome(idx, 1.0);
            let room = self.content_box(parent, parent_rect, 1.0).w - left - right - chrome_x;
            width = width.min(room.max(0.0));
        }
        Ok(width)
    }

    /// Breaks of one wrapping element (see [`split_wrap_breaks`]).
    fn wrap_breaks(&mut self, idx: usize) -> Result<Vec<usize>> {
        let composition: &'a Composition = self.composition;
        let element = &composition.elements[idx];
        if !element.style.column {
            bail!("internal: data-hf-split-wrap expected line rows");
        }
        let width = self.wrap_width(idx)?;
        if width <= 0.0 {
            bail!(
                "{} has no width to wrap into; give it a width or max-width",
                element
                    .id
                    .as_ref()
                    .map_or_else(|| format!("<{}>", element.tag), |id| format!("#{id}"))
            );
        }
        let mut breaks = Vec::new();
        let mut word = 0;
        for &row in &element.children {
            let gap = composition.elements[row].style.gap;
            let mut used: Option<f32> = None;
            for &piece in &composition.elements[row].children {
                // The placeholder that keeps an empty line's height is not a word.
                if matches!(&composition.elements[piece].content, Content::Text(t) if t.is_empty())
                {
                    continue;
                }
                let [_, right, _, left] = self.margins(piece, 1.0);
                let room = Room {
                    width: None,
                    fill: false,
                };
                let (w, _) = self.item_size(piece, 1.0, (0.0, 0.0), room)?;
                let w = w.unwrap_or(0.0) + left + right;
                used = Some(match used {
                    Some(line) if line + gap + w > width + FIT_EPSILON => {
                        breaks.push(word);
                        w
                    }
                    Some(line) => line + gap + w,
                    None => w,
                });
                word += 1;
            }
        }
        Ok(breaks)
    }
}
