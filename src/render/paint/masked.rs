//! `clip-path` and `mask-image`: the element (box, shadows, effects, content, and children) renders on
//! an offscreen layer the size of the visible region, then composites through the shape's antialiased
//! coverage (`coverage.rs`) multiplied by the mask gradient's alpha.
//!
//! As in CSS, the element's `filter` runs first (inside the layer) and the clip and mask apply to its
//! result, so a blurred or drop-shadowed element is cut by its clip path. Shapes are evaluated in
//! the element's border box after scale; rotation is rejected at parse time.

use super::coverage::{ellipse_points, polygon_coverage, rounded_rect_points};
use super::gradient::alpha_plane;
use crate::model::{ClipKind, ClipPath, Color, Length};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::Result;

/// Outline of `shape` in output pixels, for a border box `rect` (composition px) at `k` output px
/// per composition px; `cumulative` scales px values like the rest of the element.
fn outline(shape: &ClipPath, rect: Rect, k: f32, cumulative: f32) -> Vec<(f32, f32)> {
    let (x0, y0, w, h) = (rect.x * k, rect.y * k, rect.w * k, rect.h * k);
    let resolve = |length: Length, basis: f32| match length {
        Length::Px(n) => n * cumulative * k,
        Length::Percent(n) => basis * n / 100.0,
    };
    let p = &shape.params;
    match shape.kind {
        ClipKind::Inset => {
            let (top, right, bottom, left) = (
                resolve(p[0], h),
                resolve(p[1], w),
                resolve(p[2], h),
                resolve(p[3], w),
            );
            let edges = [x0 + left, y0 + top, x0 + w - right, y0 + h - bottom];
            if edges[2] <= edges[0] || edges[3] <= edges[1] {
                return Vec::new();
            }
            rounded_rect_points(edges, resolve(p[4], 0.0))
        }
        ClipKind::Circle => {
            let r = resolve(p[0], w.hypot(h) / std::f32::consts::SQRT_2);
            ellipse_points(x0 + resolve(p[1], w), y0 + resolve(p[2], h), r, r)
        }
        ClipKind::Ellipse => ellipse_points(
            x0 + resolve(p[2], w),
            y0 + resolve(p[3], h),
            resolve(p[0], w),
            resolve(p[1], h),
        ),
        ClipKind::Polygon => p
            .chunks_exact(2)
            .map(|xy| (x0 + resolve(xy[0], w), y0 + resolve(xy[1], h)))
            .collect(),
    }
}

impl<'a> Renderer<'a> {
    /// Whether element `idx` draws through [`Self::draw_masked`] (and is not already inside it).
    pub(in crate::render) fn needs_mask(&self, idx: usize) -> bool {
        let style = &self.composition.elements[idx].style;
        idx != 0
            && self.mask_guard != Some(idx)
            && (style.clip_path.is_some() || style.mask.is_some())
    }

    /// Draws element `idx` placed in `rect` through its clip path and mask; `sigma` is its filter
    /// blur in output px.
    pub(in crate::render) fn draw_masked(
        &mut self,
        idx: usize,
        rect: Rect,
        cumulative: f32,
        sigma: f32,
    ) -> Result<()> {
        let k = self.scale;
        let element = &self.composition.elements[idx];
        let elapsed = self.time - element.start;
        let shape = element
            .motion
            .clip_at(element.style.clip_path.as_ref(), elapsed);
        let mask = element.style.mask.as_ref();
        let points = shape
            .as_ref()
            .map(|shape| outline(shape, rect, k, cumulative));
        // The visible region: the shape's bounds, or the border box for a mask alone (the mask
        // covers the border box and hides everything outside it).
        let box_edges = [
            rect.x * k,
            rect.y * k,
            (rect.x + rect.w) * k,
            (rect.y + rect.h) * k,
        ];
        let mut region = match &points {
            Some(points) if points.is_empty() => return Ok(()),
            Some(points) => points.iter().fold(
                [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ],
                |[l, t, r, b], &(x, y)| [l.min(x), t.min(y), r.max(x), b.max(y)],
            ),
            None => box_edges,
        };
        if mask.is_some() {
            region = [
                region[0].max(box_edges[0]),
                region[1].max(box_edges[1]),
                region[2].min(box_edges[2]),
                region[3].min(box_edges[3]),
            ];
        }
        let edge = |v: f32| crate::render::allocation::pixel_edge(v, "clip-path");
        let Some(bounds) = clip_pixels(
            (self.surface.width, self.surface.height),
            edge(region[0].floor())?,
            edge(region[1].floor())?,
            edge(region[2].ceil())?,
            edge(region[3].ceil())?,
        ) else {
            return Ok(());
        };
        let (lw, lh) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        crate::render::allocation::rgba_bytes(lw, lh, "clip-path")?;
        let mut coverage = match &points {
            Some(points) => {
                let local: Vec<(f32, f32)> = points
                    .iter()
                    .map(|&(x, y)| (x - bounds.x0 as f32, y - bounds.y0 as f32))
                    .collect();
                polygon_coverage(&local, lw, lh)
            }
            None => vec![1.0; lw * lh],
        };
        if let Some(gradient) = mask {
            let alpha = alpha_plane(
                gradient,
                box_edges,
                bounds.x0..bounds.x1,
                bounds.y0..bounds.y1,
            );
            for (c, a) in coverage.iter_mut().zip(alpha) {
                *c *= a;
            }
        }
        if coverage.iter().all(|&c| c <= 0.0) {
            return Ok(());
        }
        let origin = (bounds.x0 as f32 / k, bounds.y0 as f32 / k);
        let shifted = Rect {
            x: rect.x - origin.0,
            y: rect.y - origin.1,
            ..rect
        };
        let window = [
            bounds.x0 as f32 / k,
            bounds.y0 as f32 / k,
            bounds.x1 as f32 / k,
            bounds.y1 as f32 / k,
        ];
        // Drawing on black and on white recovers color and alpha without alpha-aware blending.
        let outer_guard = self.mask_guard.replace(idx);
        let mut layers: Vec<Vec<u8>> = Vec::with_capacity(2);
        let mut outcome = Ok(());
        for (pass, backdrop) in [Color::BLACK, Color::WHITE].into_iter().enumerate() {
            let layer = Surface::filled(lw as u32, lh as u32, k, backdrop);
            let outer = std::mem::replace(&mut self.surface, layer);
            let saved = self.begin_layer_pass(pass, origin, Some(window));
            outcome = outcome.and_then(|()| {
                if self.has_effects(idx, sigma) {
                    self.draw_with_effects(idx, shifted, cumulative, sigma)
                } else {
                    self.draw_posed(idx, shifted, cumulative)
                }
            });
            self.end_layer_pass(saved);
            layers.push(std::mem::replace(&mut self.surface, outer).pixels);
        }
        self.mask_guard = outer_guard;
        outcome?;
        let premultiplied = super::clip::premultiply(&layers[0], &layers[1]);
        self.surface.mark_painted(bounds);
        let stride = self.surface.width as usize;
        for (row, y) in (bounds.y0..bounds.y1).enumerate() {
            for (column, x) in (bounds.x0..bounds.x1).enumerate() {
                let i = row * lw + column;
                let cover = coverage[i];
                let texel = premultiplied[i];
                let alpha = f32::from(texel[3]) / 255.0 * cover;
                if alpha <= 0.0 {
                    continue;
                }
                let offset = (y * stride + x) * 4;
                let dest = &mut self.surface.pixels[offset..offset + 4];
                for (channel, value) in dest.iter_mut().zip(texel).take(3) {
                    *channel = (f32::from(value) * cover + f32::from(*channel) * (1.0 - alpha))
                        .round()
                        .min(255.0) as u8;
                }
                dest[3] = 255;
            }
        }
        Ok(())
    }
}
