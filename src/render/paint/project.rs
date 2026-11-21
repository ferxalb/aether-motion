//! Projected planes of a 2.5D space: a plane (its box, content, effects, and flat descendants) is
//! drawn unrotated on an offscreen layer ([`PlaneLayer`]), then warped onto the target through the
//! projective map of its 3D transform and camera (`crate::space::math::View::homography`).
//!
//! Tilted planes, planes seen by a turned camera, and pieces of planes split by the depth order
//! (`crate::space::bsp`) take this path. The layer covers the part of the plane's extent (its box,
//! the boxes of its flat descendants, and their shadow reach) that can reach the target: the
//! extent is clipped in plane coordinates against the near distance and the four target edges,
//! which are straight lines there. It is drawn at the largest on-screen magnification of that part,
//! within 1.5 times that part's on-screen area (clamped to between 1 Mpx and the larger of 1 Mpx
//! and the target's pixel count); a plane holding images, video, vector art, or text is also
//! limited to 16 Mpx for its whole extent, so fitted images stay within the
//! fitted-image limit, while boxes and gradients are drawn at full magnification. One layer serves
//! every fragment of its plane in a frame.
//!
//! Each target pixel is mapped back by the inverse homography and sampled from premultiplied
//! texels: bilinearly where the layer is not minified, and trilinearly from a box-filtered mipmap
//! chain (built on demand) where it is, using the exact footprint of the pixel. A fragment keeps
//! the pixels whose center lies on its side of each cut: the two pieces of a split plane partition
//! the pixels exactly, so nothing behind them shows through the cut (the plane's own outline stays
//! antialiased). Pixels whose plane point is closer than the near distance are left untouched.
//! While a layout probe runs, text drawn on the layer is reported through the same projection, cut
//! at the same near distance. Blend modes cannot be drawn on a projected plane; the renderer says
//! so (see `paint/effects`).

use crate::model::Color;
use crate::render::geometry::{clip_pixels, PixelRect, Rect};
use crate::render::paint::Surface;
use crate::render::Renderer;
use crate::space::math::{clip_half_plane, clip_near, invert, Affine, View};
use crate::space::Projection;
use anyhow::Result;

/// Most pixels a whole plane extent may have at the layer scale when it holds sized content.
const BOX_PIXELS: f64 = 16_777_216.0;
/// Least pixel budget of a layer, whatever the target size.
const MIN_LAYER_PIXELS: f64 = 1_048_576.0;

/// A plane drawn flat on an offscreen layer, ready to be warped onto the target.
pub(in crate::render) struct PlaneLayer {
    /// Premultiplied texels of each mip level (level 0 first); levels are added on demand.
    levels: Vec<(Vec<[u8; 4]>, [usize; 2])>,
    /// Layer texels per local px.
    scale: f64,
    /// Local px of the layer's texel origin.
    origin: [f64; 2],
    /// Target pixels to plane-local px (third coordinate: one over the camera distance).
    inverse: [[f64; 3]; 3],
    /// Target pixels to homogeneous plane-local px, for screen bounds of fragments.
    forward: [[f64; 3]; 3],
    /// The plane's extent after near and target clipping (plane-local px).
    visible: Vec<[f64; 2]>,
    near: f64,
}

/// Inputs that place a plane in its space for one frame.
#[derive(Clone, Copy)]
pub(in crate::render) struct PlaneSetup<'v> {
    pub(in crate::render) plane: Affine,
    /// Resting border-box size (local px).
    pub(in crate::render) size: [f32; 2],
    /// What the plane and its flat descendants may paint, `[left, top, right, bottom]` in local
    /// px from the plane's center.
    pub(in crate::render) extent: [f64; 4],
    /// Whether the plane holds images, video, vector art, or text (see [`BOX_PIXELS`]).
    pub(in crate::render) sized_content: bool,
    pub(in crate::render) view: &'v View,
}

impl<'a> Renderer<'a> {
    /// Draws plane `idx` on a layer for the current frame, or `None` when none of it can show.
    pub(in crate::render) fn plane_layer(
        &mut self,
        idx: usize,
        setup: PlaneSetup<'_>,
    ) -> Result<Option<PlaneLayer>> {
        let k = f64::from(self.scale);
        let (plane, view, extent) = (&setup.plane, setup.view, setup.extent);
        let h = view.homography(plane, k);
        let Some(inverse) = invert(&h) else {
            return Ok(None);
        };
        let mut visible = vec![
            [extent[0], extent[1]],
            [extent[2], extent[1]],
            [extent[2], extent[3]],
            [extent[0], extent[3]],
        ];
        if let Projection::Perspective { .. } = view.projection {
            visible = clip_near(&visible, [h[2][2], h[2][0], h[2][1]], view.near);
        }
        let target = [
            f64::from(self.surface.width) + 1.0,
            f64::from(self.surface.height) + 1.0,
        ];
        for (row, high) in [(0, target[0]), (1, target[1])] {
            let (r, w) = (h[row], h[2]);
            visible = clip_half_plane(&visible, [r[2] + w[2], r[0] + w[0], r[1] + w[1]]);
            visible = clip_half_plane(
                &visible,
                [high * w[2] - r[2], high * w[0] - r[0], high * w[1] - r[1]],
            );
        }
        if visible.len() < 3 {
            return Ok(None);
        }
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        let mut magnification = 0.0_f64;
        for &[u, v] in &visible {
            low = [low[0].min(u), low[1].min(v)];
            high = [high[0].max(u), high[1].max(v)];
            magnification = magnification.max(view.magnification(plane.apply([u, v, 0.0])));
        }
        // Layer scale: composition px per local px, at the largest magnification, within the caps.
        let unit = |a: [f64; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        let mut q = magnification * unit(plane.axis(0)).max(unit(plane.axis(1)));
        // Texels beyond about 1.5 times the plane's on-screen area only feed minified pixels.
        let projected: Vec<[f64; 2]> = visible
            .iter()
            .map(|&[u, v]| view.project(plane.apply([u, v, 0.0])).map(|c| c * k))
            .collect();
        let area = projected
            .iter()
            .zip(projected.iter().cycle().skip(1))
            .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
            .sum::<f64>()
            .abs()
            / 2.0;
        let frame = f64::from(self.surface.width) * f64::from(self.surface.height);
        let budget = (1.5 * area).clamp(MIN_LAYER_PIXELS, frame.max(MIN_LAYER_PIXELS));
        let span = [high[0] - low[0], high[1] - low[1]];
        let layer_pixels = span[0] * span[1] * (q * k).powi(2);
        if layer_pixels > budget {
            q *= (budget / layer_pixels).sqrt();
        }
        if setup.sized_content {
            let whole = (extent[2] - extent[0]) * (extent[3] - extent[1]) * (q * k).powi(2);
            if whole > BOX_PIXELS {
                q *= (BOX_PIXELS / whole).sqrt();
            }
        }
        if !q.is_finite() || q <= 0.0 {
            return Ok(None);
        }
        // Two layer pixels of slack around the visible part, so bilinear taps stay inside.
        let pad = 2.0 / (q * k);
        let origin = [low[0] - pad, low[1] - pad];
        let (lw, lh) = (
            (((span[0] + 2.0 * pad) * q * k).ceil() as usize).max(1),
            (((span[1] + 2.0 * pad) * q * k).ceil() as usize).max(1),
        );
        crate::render::allocation::rgba_bytes(lw, lh, "projected plane")?;
        let size = setup.size;
        // The plane's border box in layer composition px: local u sits at (u - origin) * q.
        let layer_rect = Rect {
            x: ((-f64::from(size[0]) / 2.0 - origin[0]) * q) as f32,
            y: ((-f64::from(size[1]) / 2.0 - origin[1]) * q) as f32,
            w: (f64::from(size[0]) * q) as f32,
            h: (f64::from(size[1]) * q) as f32,
        };
        // Layer composition px to target composition px, for text the probe reports.
        let to_local = [
            [1.0 / q, 0.0, origin[0]],
            [0.0, 1.0 / q, origin[1]],
            [0.0, 0.0, 1.0],
        ];
        let to_target = multiply(&scale_rows(&h, 1.0 / k), &to_local);
        // Drawing on black and on white recovers color and alpha without alpha-aware blending.
        let started = crate::profile::enabled().then(std::time::Instant::now);
        let mut layers: Vec<Vec<u8>> = Vec::with_capacity(2);
        let mut outcome = Ok(());
        for (pass, backdrop) in [Color::BLACK, Color::WHITE].into_iter().enumerate() {
            let layer = Surface::filled(lw as u32, lh as u32, self.scale, backdrop);
            let outer = std::mem::replace(&mut self.surface, layer);
            let probed = pass == 0 && self.enter_probe_map(&to_target, view.near);
            self.layer_depth += 1;
            self.isolated += 1;
            self.projecting += 1;
            outcome = outcome.and_then(|()| self.draw_plane(idx, layer_rect, q as f32, 0.0));
            self.projecting -= 1;
            self.isolated -= 1;
            self.layer_depth -= 1;
            if probed {
                self.leave_probe_map();
            }
            layers.push(std::mem::replace(&mut self.surface, outer).pixels);
        }
        outcome?;
        let texels = super::clip::premultiply(&layers[0], &layers[1]);
        if let Some(started) = started {
            crate::profile::add(crate::profile::Phase::PlaneLayer, started.elapsed());
        }
        Ok(Some(PlaneLayer {
            levels: vec![(texels, [lw, lh])],
            scale: q * k,
            origin,
            inverse,
            forward: h,
            visible,
            near: view.near,
        }))
    }

    /// Composites `layer` onto the target, cut by `clips` (`c0 + cu * u + cv * v >= 0` in
    /// plane-local px; none for the whole plane).
    pub(in crate::render) fn composite_plane(
        &mut self,
        layer: &mut PlaneLayer,
        clips: &[[f64; 3]],
    ) {
        // Screen bounds of the fragment, with a little slack around its cuts.
        let slack = 2.0 / layer.scale.max(1e-9);
        let mut outline = layer.visible.clone();
        for clip in clips {
            let reach = clip[1].hypot(clip[2]) * slack;
            outline = clip_half_plane(&outline, [clip[0] + reach, clip[1], clip[2]]);
        }
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        let mut screen = Vec::with_capacity(outline.len());
        for &[u, v] in &outline {
            let h = &layer.forward;
            let w = h[2][0] * u + h[2][1] * v + h[2][2];
            if w <= 0.0 {
                continue;
            }
            let point: [f64; 2] =
                std::array::from_fn(|axis| (h[axis][0] * u + h[axis][1] * v + h[axis][2]) / w);
            for axis in 0..2 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
            screen.push(point);
        }
        if outline.len() < 3 || !low.iter().chain(&high).all(|v| v.is_finite()) {
            return;
        }
        let edge = |value: f64| value.clamp(-1e9, 1e9) as i32;
        let Some(bounds) = clip_pixels(
            (self.surface.width, self.surface.height),
            edge(low[0].floor()) - 1,
            edge(low[1].floor()) - 1,
            edge(high[0].ceil()) + 1,
            edge(high[1].ceil()) + 1,
        ) else {
            return;
        };
        self.surface.mark_painted(bounds);
        let surface = &mut self.surface;
        crate::profile::time(crate::profile::Phase::PlaneWarp, || {
            warp(surface, bounds, &screen, layer, clips)
        });
    }
}

fn multiply(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|i| a[r][i] * b[i][c]).sum()))
}

/// `m` with its first two rows (the pixel coordinates) scaled by `s`.
fn scale_rows(m: &[[f64; 3]; 3], s: f64) -> [[f64; 3]; 3] {
    [m[0].map(|v| v * s), m[1].map(|v| v * s), m[2]]
}

impl PlaneLayer {
    /// Mip level `level` (0 = the layer), building the chain up to it on demand by averaging 2x2
    /// blocks (texels outside the layer count as transparent).
    /// Returns the level actually available (the chain ends at 1x1).
    fn level(&mut self, level: usize) -> (usize, &[[u8; 4]], [usize; 2]) {
        while self.levels.len() <= level {
            let (texels, [w, h]) = self.levels.last().expect("level 0 exists");
            if *w == 1 && *h == 1 {
                break;
            }
            let (nw, nh) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
            let at = |x: usize, y: usize| -> [u32; 4] {
                if x < *w && y < *h {
                    texels[y * w + x].map(u32::from)
                } else {
                    [0; 4]
                }
            };
            let mut next = Vec::with_capacity(nw * nh);
            for y in 0..nh {
                for x in 0..nw {
                    let taps = [
                        at(2 * x, 2 * y),
                        at(2 * x + 1, 2 * y),
                        at(2 * x, 2 * y + 1),
                        at(2 * x + 1, 2 * y + 1),
                    ];
                    next.push(std::array::from_fn(|c| {
                        ((taps.iter().map(|t| t[c]).sum::<u32>() + 2) / 4) as u8
                    }));
                }
            }
            self.levels.push((next, [nw, nh]));
        }
        let level = level.min(self.levels.len() - 1);
        let (texels, size) = &self.levels[level];
        (level, texels, *size)
    }

    /// Bilinear sample of mip level `level` at level-0 texel coordinates `(sx, sy)`.
    fn bilinear(&mut self, level: usize, sx: f64, sy: f64) -> [f32; 4] {
        let (level, texels, [lw, lh]) = self.level(level);
        let factor = f64::from(1u32 << level.min(30));
        let (sx, sy) = ((sx + 0.5) / factor - 0.5, (sy + 0.5) / factor - 0.5);
        let (ix, iy) = (sx.floor(), sy.floor());
        let (fx, fy) = ((sx - ix) as f32, (sy - iy) as f32);
        let (ix, iy) = (ix as i64, iy as i64);
        let texel = |x: i64, y: i64| -> [f32; 4] {
            if x < 0 || y < 0 || x >= lw as i64 || y >= lh as i64 {
                return [0.0; 4];
            }
            texels[y as usize * lw + x as usize].map(f32::from)
        };
        let taps = [
            (texel(ix, iy), (1.0 - fx) * (1.0 - fy)),
            (texel(ix + 1, iy), fx * (1.0 - fy)),
            (texel(ix, iy + 1), (1.0 - fx) * fy),
            (texel(ix + 1, iy + 1), fx * fy),
        ];
        let mut sample = [0.0_f32; 4];
        for (tap, weight) in taps {
            for (total, value) in sample.iter_mut().zip(tap) {
                *total += value * weight;
            }
        }
        sample
    }
}

/// Composites `layer` (cut by `clips`) over `bounds` of the target.
/// Columns `[first, last)` of row `y` that the convex screen polygon `outline` (target pixels)
/// can touch, with two pixels of slack, within `bounds`.
fn row_span(outline: &[[f64; 2]], y: usize, bounds: PixelRect) -> (usize, usize) {
    let (top, bottom) = (y as f64 - 1.0, y as f64 + 2.0);
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for (i, &a) in outline.iter().enumerate() {
        let b = outline[(i + 1) % outline.len()];
        // The part of edge a-b inside the band [top, bottom].
        let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
        if hi < top || lo > bottom {
            continue;
        }
        let at = |t: f64| a[0] + (b[0] - a[0]) * t;
        if (b[1] - a[1]).abs() < 1e-12 {
            low = low.min(a[0].min(b[0]));
            high = high.max(a[0].max(b[0]));
            continue;
        }
        for edge_y in [top.max(lo), bottom.min(hi)] {
            let x = at(((edge_y - a[1]) / (b[1] - a[1])).clamp(0.0, 1.0));
            low = low.min(x);
            high = high.max(x);
        }
    }
    if low.partial_cmp(&high).is_none_or(|order| order.is_gt()) {
        return (0, 0);
    }
    let first = ((low - 2.0).floor().max(bounds.x0 as f64) as usize).min(bounds.x1);
    let last = ((high + 2.0).ceil().min(bounds.x1 as f64).max(0.0) as usize).max(first);
    (first, last)
}

fn warp(
    surface: &mut Surface,
    bounds: PixelRect,
    outline: &[[f64; 2]],
    layer: &mut PlaneLayer,
    clips: &[[f64; 3]],
) {
    let m = layer.inverse;
    let far_limit = if layer.near > 0.0 {
        1.0 / layer.near
    } else {
        f64::INFINITY
    };
    let (scale, origin) = (layer.scale, layer.origin);
    let [lw, lh] = layer.levels[0].1;
    let stride = surface.width as usize;
    for y in bounds.y0..bounds.y1 {
        let py = y as f64 + 0.5;
        let (first, last) = row_span(outline, y, bounds);
        for x in first..last {
            let px = x as f64 + 0.5;
            let c = m[2][0] * px + m[2][1] * py + m[2][2];
            // `c` is one over the camera distance (perspective) or 1 (orthographic).
            if c <= 0.0 || c >= far_limit {
                continue;
            }
            let u = (m[0][0] * px + m[0][1] * py + m[0][2]) / c;
            let v = (m[1][0] * px + m[1][1] * py + m[1][2]) / c;
            let sx = (u - origin[0]) * scale - 0.5;
            let sy = (v - origin[1]) * scale - 0.5;
            if !(sx > -1.0 && sy > -1.0 && sx < lw as f64 && sy < lh as f64) {
                continue;
            }
            // A fragment keeps the pixels whose center is on its side of every cut (a center
            // exactly on a cut belongs to the front piece, whose cut is `>= 0`).
            if clips
                .iter()
                .any(|clip| clip[0] + clip[1] * u + clip[2] * v < 0.0)
            {
                continue;
            }
            // Derivatives of (u, v) per target pixel, for the mip level.
            let du = [(m[0][0] - u * m[2][0]) / c, (m[0][1] - u * m[2][1]) / c];
            let dv = [(m[1][0] - v * m[2][0]) / c, (m[1][1] - v * m[2][1]) / c];
            let footprint = du[0].hypot(dv[0]).max(du[1].hypot(dv[1])) * scale;
            let sample = if footprint > 1.0 {
                let level = footprint.log2();
                let base = level.floor() as usize;
                let f = (level - level.floor()) as f32;
                let a = layer.bilinear(base, sx, sy);
                let b = layer.bilinear(base + 1, sx, sy);
                std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f)
            } else {
                layer.bilinear(0, sx, sy)
            };
            if sample[3] < 0.5 {
                continue;
            }
            let keep = 1.0 - sample[3] / 255.0;
            let offset = (y * stride + x) * 4;
            let dest = &mut surface.pixels[offset..offset + 4];
            for (channel, value) in dest.iter_mut().zip(sample).take(3) {
                *channel = (value + f32::from(*channel) * keep).round().min(255.0) as u8;
            }
            dest[3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(texels: Vec<[u8; 4]>, size: [usize; 2]) -> PlaneLayer {
        PlaneLayer {
            levels: vec![(texels, size)],
            scale: 1.0,
            origin: [0.0; 2],
            inverse: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            forward: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            visible: Vec::new(),
            near: 0.0,
        }
    }

    #[test]
    fn mip_requests_past_the_chain_use_its_last_level() {
        let mut layer = layer(vec![[200, 100, 40, 255]; 4], [2, 2]);
        let (level, _, size) = layer.level(7);
        assert_eq!((level, size), (1, [1, 1]));
        // Asking again does not grow the chain.
        layer.level(9);
        assert_eq!(layer.levels.len(), 2);
        // A clamped request samples the last level at its own scale: the same as asking for it.
        let past = layer.bilinear(6, 0.5, 0.5);
        let last = layer.bilinear(1, 0.5, 0.5);
        assert_eq!(past, last);
        assert!(past[3] > 0.0);
    }
}
