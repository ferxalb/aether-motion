//! Reveals and wipes: the element (box, content, and children) renders on an offscreen layer, then
//! composites through a mask that uncovers a growing part of its box from one edge.
//!
//! The mask edge is antialiased per pixel, so a reveal moves smoothly even at one pixel per frame.

use crate::model::{Color, RevealFrom};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::{anyhow, Result};

/// Share (0..=1) of pixel `p` that lies inside `[lo, hi]` along one axis.
fn span_coverage(p: usize, lo: f32, hi: f32) -> f32 {
    (hi.min(p as f32 + 1.0) - lo.max(p as f32)).clamp(0.0, 1.0)
}

/// Revealed `(x range, y range)` of a box with edges `[x0, y0, x1, y1]` when `amount` of it shows.
fn revealed_ranges(
    [x0, y0, x1, y1]: [f32; 4],
    amount: f32,
    from: RevealFrom,
) -> ((f32, f32), (f32, f32)) {
    let (w, h) = (x1 - x0, y1 - y0);
    match from {
        RevealFrom::Left => ((x0, x0 + amount * w), (y0, y1)),
        RevealFrom::Right => ((x1 - amount * w, x1), (y0, y1)),
        RevealFrom::Top => ((x0, x1), (y0, y0 + amount * h)),
        RevealFrom::Bottom => ((x0, x1), (y1 - amount * h, y1)),
    }
}

impl<'a> Renderer<'a> {
    /// Draws element `idx` showing only `amount` (0..=1) of its box, growing from the `from` edge.
    pub(in crate::render) fn draw_revealed(
        &mut self,
        idx: usize,
        rect: Rect,
        cumulative: f32,
        amount: f32,
        from: RevealFrom,
    ) -> Result<()> {
        if amount <= 0.0 {
            return Ok(());
        }
        let amount = amount.min(1.0);
        let k = self.scale;
        let (edges, _) = self.surface.fill_edges(rect, 0.0);
        let [x0, y0, x1, y1] = edges;
        let Some(bounds) = clip_pixels((self.surface.width, self.surface.height), x0, y0, x1, y1)
        else {
            return Ok(());
        };
        let (lw, lh) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        if lw.saturating_mul(lh).saturating_mul(4) > 128 * 1024 * 1024 {
            return Err(anyhow!("a reveal needs a layer over 128 MiB; use --scale"));
        }
        // The layer's origin is the clipped box corner, so layout coordinates shift by that corner.
        let shifted = Rect {
            x: rect.x - bounds.x0 as f32 / k,
            y: rect.y - bounds.y0 as f32 / k,
            ..rect
        };
        // Drawing on black and on white recovers color and alpha without alpha-aware blending.
        let mut layers: Vec<Vec<u8>> = Vec::with_capacity(2);
        let mut outcome = Ok(());
        for backdrop in [Color::BLACK, Color::WHITE] {
            let layer = Surface::filled(lw as u32, lh as u32, k, backdrop);
            let outer = std::mem::replace(&mut self.surface, layer);
            outcome = outcome.and_then(|()| self.draw_body(idx, shifted, cumulative));
            layers.push(std::mem::replace(&mut self.surface, outer).pixels);
        }
        outcome?;
        let premultiplied = super::clip::premultiply(&layers[0], &layers[1]);
        let ((rx0, rx1), (ry0, ry1)) = revealed_ranges(edges.map(|edge| edge as f32), amount, from);
        self.surface.mark_painted(bounds);
        let stride = self.surface.width as usize;
        for y in bounds.y0..bounds.y1 {
            let row = span_coverage(y, ry0, ry1);
            if row <= 0.0 {
                continue;
            }
            for x in bounds.x0..bounds.x1 {
                let coverage = row * span_coverage(x, rx0, rx1);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_is_fractional_at_the_edge() {
        assert_eq!(span_coverage(3, 0.0, 10.0), 1.0);
        assert_eq!(span_coverage(3, 3.25, 10.0), 0.75);
        assert_eq!(span_coverage(3, 0.0, 3.5), 0.5);
        assert_eq!(span_coverage(3, 5.0, 10.0), 0.0);
    }

    #[test]
    fn ranges_grow_from_the_named_edge() {
        let edges = [10.0, 20.0, 110.0, 70.0];
        assert_eq!(
            revealed_ranges(edges, 0.5, RevealFrom::Left).0,
            (10.0, 60.0)
        );
        assert_eq!(
            revealed_ranges(edges, 0.5, RevealFrom::Right).0,
            (60.0, 110.0)
        );
        assert_eq!(revealed_ranges(edges, 0.5, RevealFrom::Top).1, (20.0, 45.0));
        assert_eq!(
            revealed_ranges(edges, 0.5, RevealFrom::Bottom).1,
            (45.0, 70.0)
        );
        assert_eq!(
            revealed_ranges(edges, 1.0, RevealFrom::Left).0,
            (10.0, 110.0)
        );
    }
}
