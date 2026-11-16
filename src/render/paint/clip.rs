//! `overflow: hidden`: the element's content renders on an offscreen layer the size of its padding
//! box, then composites onto the frame through the box mask, so rounded corners clip too.
//!
//! The element's own background, border, and shadow are painted before this, unclipped, as in CSS.

use crate::model::Color;
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::Result;

/// Recovers premultiplied RGBA from one drawing made on black and again on white.
///
/// Color comes from the black pass; alpha from how much of the white backdrop shows through.
pub(super) fn premultiply(black: &[u8], white: &[u8]) -> Vec<[u8; 4]> {
    black
        .as_chunks::<4>()
        .0
        .iter()
        .zip(white.as_chunks::<4>().0)
        .map(|(b, w)| premultiplied_pixel(b, w))
        .collect()
}

/// Recovers one texel, shared by clipping and direct blur-plane preparation.
pub(super) fn premultiplied_pixel(b: &[u8; 4], w: &[u8; 4]) -> [u8; 4] {
    let gap = (0..3)
        .map(|c| i32::from(w[c]) - i32::from(b[c]))
        .sum::<i32>()
        / 3;
    [b[0], b[1], b[2], (255 - gap).clamp(0, 255) as u8]
}

impl<'a> Renderer<'a> {
    /// Draws the content of element `idx` clipped to its padding box (rounded like the border).
    pub(in crate::render) fn draw_clipped(
        &mut self,
        idx: usize,
        rect: Rect,
        content: Rect,
        cumulative: f32,
    ) -> Result<()> {
        let k = self.scale;
        let style = &self.composition.elements[idx].style;
        let border = style.border_width * cumulative;
        let radius = (style.border_radius * cumulative - border).max(0.0);
        let inner = Rect {
            x: rect.x + border,
            y: rect.y + border,
            w: (rect.w - 2.0 * border).max(0.0),
            h: (rect.h - 2.0 * border).max(0.0),
        };
        let (edges, radius_px) = self.surface.fill_edges(inner, radius);
        let [x0, y0, x1, y1] = edges;
        let Some(bounds) = clip_pixels((self.surface.width, self.surface.height), x0, y0, x1, y1)
        else {
            return Ok(());
        };
        let (lw, lh) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        crate::render::allocation::rgba_bytes(lw, lh, "overflow: hidden")?;
        // The layer's origin is the clipped box corner, so layout coordinates shift by that corner.
        let (ox, oy) = (bounds.x0 as f32 / k, bounds.y0 as f32 / k);
        let shift = |r: Rect| Rect {
            x: r.x - ox,
            y: r.y - oy,
            ..r
        };
        let (rect_layer, content_layer) = (shift(rect), shift(content));
        let mut layers: Vec<Vec<u8>> = Vec::with_capacity(2);
        let mut outcome = Ok(());
        for backdrop in [Color::BLACK, Color::WHITE] {
            let layer = Surface::filled(lw as u32, lh as u32, k, backdrop);
            let outer = std::mem::replace(&mut self.surface, layer);
            outcome = outcome
                .and_then(|()| self.draw_contents(idx, rect_layer, content_layer, cumulative));
            layers.push(std::mem::replace(&mut self.surface, outer).pixels);
        }
        outcome?;
        let premultiplied = premultiply(&layers[0], &layers[1]);
        self.surface.mark_painted(bounds);
        let float_edges = edges.map(|edge| edge as f32);
        let stride = self.surface.width as usize;
        for y in bounds.y0..bounds.y1 {
            let (left, right) = Surface::rounded_span(float_edges, radius_px, y);
            let first = left.floor().max(bounds.x0 as f32) as usize;
            let last = right.ceil().min(bounds.x1 as f32) as usize;
            for x in first..last {
                let coverage = (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0);
                let texel = premultiplied[(y - bounds.y0) * lw + (x - bounds.x0)];
                let alpha = f32::from(texel[3]) / 255.0 * coverage;
                if alpha <= 0.0 {
                    continue;
                }
                let offset = (y * stride + x) * 4;
                let dest = &mut self.surface.pixels[offset..offset + 4];
                for (channel, value) in dest.iter_mut().zip(texel).take(3) {
                    *channel = (f32::from(value) * coverage + f32::from(*channel) * (1.0 - alpha))
                        .round()
                        .min(255.0) as u8;
                }
                dest[3] = 255;
            }
        }
        Ok(())
    }
}
