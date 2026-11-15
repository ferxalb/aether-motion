//! Explicit and intrinsic sizing of a single element inside a layout.
//!
//! Sizes are in layout units (composition px times the accumulated parent scale `ps`). Text sizing
//! follows CSS shrink-to-fit: the content width capped by the room offered and by `max-width`.

use crate::model::{Composition, Content};
use crate::render::Renderer;
use anyhow::Result;

/// Horizontal room a parent offers an item.
#[derive(Clone, Copy, Default)]
pub(in crate::render) struct Room {
    /// Width the item may use, if the parent's width is definite.
    pub(in crate::render) width: Option<f32>,
    /// The item stretches to `width` (a flex column with `align-items: stretch`).
    pub(in crate::render) fill: bool,
}

impl<'a> Renderer<'a> {
    /// Padding plus border on the horizontal and vertical axes, in layout units.
    pub(in crate::render) fn chrome(&self, idx: usize, ps: f32) -> (f32, f32) {
        let style = &self.composition.elements[idx].style;
        let [top, right, bottom, left] = style.padding.map(|padding| padding * ps);
        let border = style.border_width * ps * 2.0;
        (left + right + border, top + bottom + border)
    }

    /// Explicit or intrinsic size of an element in layout units; `None` means unknown.
    pub(in crate::render) fn item_size(
        &mut self,
        idx: usize,
        ps: f32,
        avail: (f32, f32),
        room: Room,
    ) -> Result<(Option<f32>, Option<f32>)> {
        let composition: &'a Composition = self.composition;
        let element = &composition.elements[idx];
        let style = &element.style;
        let max_width = style.max_width.map(|n| n.resolve_scaled(avail.0, ps));
        let cap = |width: f32| max_width.map_or(width, |max| width.min(max));
        let width = style.width.map(|n| cap(n.resolve_scaled(avail.0, ps)));
        let height = style.height.map(|n| n.resolve_scaled(avail.1, ps));
        if width.is_some() && height.is_some() {
            return Ok((width, height));
        }
        // Outer width the content may grow to: the room offered, capped by `max-width`.
        let ceiling = match (room.width, max_width) {
            (Some(room), Some(max)) => Some(room.min(max)),
            (room, max) => room.or(max),
        };
        let (chrome_x, chrome_y) = self.chrome(idx, ps);
        match &element.content {
            Content::Text(_) => {
                let fixed = width.or_else(|| room.fill.then_some(ceiling).flatten());
                let limit = fixed
                    .or(ceiling)
                    .map(|outer| ((outer - chrome_x) / ps).max(0.0));
                let block = self.text_block(idx, limit)?;
                Ok((
                    Some(fixed.unwrap_or(block.fit_width(limit) * ps + chrome_x)),
                    Some(height.unwrap_or(block.height() * ps + chrome_y)),
                ))
            }
            Content::Container if style.display.as_deref() == Some("flex") => {
                let inner = width.or(ceiling).map(|outer| (outer - chrome_x).max(0.0));
                let Some((w, h)) = self.flex_intrinsic(idx, ps, inner)? else {
                    return Ok((width, height));
                };
                Ok((width.or(Some(cap(w))), height.or(Some(h))))
            }
            _ => Ok((width, height)),
        }
    }

    /// Height of a text element's border box when it is `outer_width` wide.
    pub(in crate::render) fn text_height_at(
        &mut self,
        idx: usize,
        ps: f32,
        outer_width: f32,
    ) -> Result<f32> {
        let (chrome_x, chrome_y) = self.chrome(idx, ps);
        let limit = ((outer_width - chrome_x) / ps).max(0.0);
        let block = self.text_block(idx, Some(limit))?;
        Ok(block.height() * ps + chrome_y)
    }
}
