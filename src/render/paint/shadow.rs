//! Soft outer box shadows that follow the rounded box.
//!
//! Soft shadows (sigma of 2 px or more) are rasterized once per size into a coverage mask and
//! blended on later frames. The mask is anchored to whole pixels, so a moving card's shadow
//! snaps by at most half a pixel, which is invisible under a blur this wide. Sharp shadows are
//! always drawn directly.

use crate::model::{Color, Shadow};
use crate::render::geometry::{clip_pixels, PixelRect, Rect};
use crate::render::paint::shape::{rounded_distance, GaussianEdge};
use crate::render::paint::surface::Surface;
use crate::render::paint::tile_cache::TileCache;
use anyhow::Result;

/// Pixels at or above this blur radius (sigma) take the cached-mask path.
const MIN_CACHED_SIGMA: f32 = 2.0;

/// Shadow coverage (x65535, element interior already removed) over a rectangle of pixels
/// relative to the element's top-left pixel, plus the painted column ranges of each row.
pub(in crate::render) struct ShadowTile {
    x0: i32,
    y0: i32,
    width: usize,
    mask: Vec<u16>,
    /// Per row: first and last painted column (exclusive) split around an empty middle gap.
    spans: Vec<[u32; 4]>,
}

/// Masks keyed by element size, radius, spread, blur, and offset (all in output pixels).
pub(in crate::render) type ShadowCache = TileCache<[u32; 7], ShadowTile>;

impl ShadowTile {
    fn build(
        size: (i32, i32),
        element_radius: f32,
        cast_radius: f32,
        cast: [f32; 4],
        edge: &GaussianEdge,
    ) -> Self {
        let reach = edge.tail.ceil() + 1.0;
        let (x0, y0) = (
            (cast[0] - reach).floor() as i32,
            (cast[1] - reach).floor() as i32,
        );
        let (x1, y1) = (
            (cast[2] + reach).ceil() as i32,
            (cast[3] + reach).ceil() as i32,
        );
        let (width, height) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let element = [0.0, 0.0, size.0 as f32, size.1 as f32];
        let mut mask = vec![0u16; width * height];
        let mut spans = Vec::with_capacity(height);
        for j in 0..height {
            let py = (y0 + j as i32) as f32 + 0.5;
            let row = &mut mask[j * width..(j + 1) * width];
            for (i, texel) in row.iter_mut().enumerate() {
                let px = (x0 + i as i32) as f32 + 0.5;
                let covered = edge.coverage(rounded_distance(px, py, cast, cast_radius));
                if covered > 0.0 {
                    let outside =
                        (rounded_distance(px, py, element, element_radius) + 0.5).clamp(0.0, 1.0);
                    *texel = (covered * outside * 65535.0).round() as u16;
                }
            }
            spans.push(Self::row_spans(row));
        }
        Self {
            x0,
            y0,
            width,
            mask,
            spans,
        }
    }

    /// Painted range of a row, split around its longest empty run (the hidden card interior).
    fn row_spans(row: &[u16]) -> [u32; 4] {
        let Some(first) = row.iter().position(|&m| m > 0) else {
            return [0; 4];
        };
        let last = row.iter().rposition(|&m| m > 0).unwrap() + 1;
        let (mut best, mut run_start) = ((0, 0), None);
        for (i, &m) in row[first..last].iter().enumerate() {
            match (m == 0, run_start) {
                (true, None) => run_start = Some(i),
                (false, Some(s)) => {
                    if i - s > best.1 - best.0 {
                        best = (s, i);
                    }
                    run_start = None;
                }
                _ => {}
            }
        }
        if best.1 - best.0 < 16 {
            [first as u32, last as u32, 0, 0]
        } else {
            [
                first as u32,
                (first + best.0) as u32,
                (first + best.1) as u32,
                last as u32,
            ]
        }
    }
}

impl Surface {
    /// Blends a cached shadow mask whose element sits at pixel `(ox, oy)`.
    fn blend_shadow_tile(
        pixels: &mut [u8],
        (w, h): (i32, i32),
        tile: &ShadowTile,
        (ox, oy): (i32, i32),
        color: Color,
        strength: f32,
    ) {
        let k = strength / 65535.0;
        let stride = w as usize;
        let rgb = [color.r, color.g, color.b].map(f32::from);
        for (j, spans) in tile.spans.iter().enumerate() {
            let y = i64::from(oy) + i64::from(tile.y0) + j as i64;
            if y < 0 || y >= i64::from(h) {
                continue;
            }
            let base = i64::from(ox) + i64::from(tile.x0);
            let mask = &tile.mask[j * tile.width..(j + 1) * tile.width];
            for pair in [[spans[0], spans[1]], [spans[2], spans[3]]] {
                let lo = (base + pair[0] as i64).max(0);
                let hi = (base + pair[1] as i64).min(w as i64);
                if lo >= hi {
                    continue;
                }
                let first = (lo - base) as usize;
                let dest = &mut pixels[(y as usize * stride + lo as usize) * 4
                    ..(y as usize * stride + hi as usize) * 4];
                for (pixel, &m) in dest.as_chunks_mut::<4>().0.iter_mut().zip(&mask[first..]) {
                    let a = f32::from(m) * k;
                    if a > 0.002 {
                        for c in 0..3 {
                            pixel[c] = (rgb[c] * a + f32::from(pixel[c]) * (1.0 - a) + 0.5) as u8;
                        }
                        pixel[3] = 255;
                    }
                }
            }
        }
    }

    /// Soft outer shadow: Gaussian falloff from the distance to the shadow's rounded rectangle.
    pub(in crate::render) fn draw_shadow(
        &mut self,
        rect: Rect,
        radius: f32,
        shadow: Shadow,
        alpha: f32,
        zoom: f32,
    ) -> Result<()> {
        let k = self.scale;
        let unit = zoom * k;
        let spread = shadow.spread * unit;
        let sigma = shadow.blur * unit / 2.0;
        let element = [
            rect.x * k,
            rect.y * k,
            (rect.x + rect.w) * k,
            (rect.y + rect.h) * k,
        ];
        let cast = [
            element[0] + shadow.x * unit - spread,
            element[1] + shadow.y * unit - spread,
            element[2] + shadow.x * unit + spread,
            element[3] + shadow.y * unit + spread,
        ];
        let edge = GaussianEdge::new(sigma)?;
        if sigma >= MIN_CACHED_SIGMA {
            let strength = alpha * f32::from(shadow.color.a) / 255.0;
            if strength <= 0.0 {
                return Ok(());
            }
            let size = (
                (element[2] - element[0]).round() as i32,
                (element[3] - element[1]).round() as i32,
            );
            let origin = (element[0].round() as i32, element[1].round() as i32);
            let (sx, sy) = (shadow.x * unit, shadow.y * unit);
            let cast = [
                -spread + sx,
                -spread + sy,
                size.0 as f32 + spread + sx,
                size.1 as f32 + spread + sy,
            ];
            let (element_radius, cast_radius) = (radius * k, (radius * k + spread).max(0.0));
            let key = [
                size.0 as u32,
                size.1 as u32,
                element_radius.to_bits(),
                spread.to_bits(),
                sigma.to_bits(),
                sx.to_bits(),
                sy.to_bits(),
            ];
            let reach = edge.tail.ceil() + 1.0;
            let cost = ((cast[2] - cast[0] + 2.0 * reach + 2.0)
                * (cast[3] - cast[1] + 2.0 * reach + 2.0)) as usize;
            let cost = cost.checked_mul(2).unwrap_or(usize::MAX);
            let (width, height) = (self.width, self.height);
            // A strongly negative spread can collapse the mask rectangle. The direct
            // path remains valid, but constructing a tile would cast negative lengths
            // to usize. Such a tile must never reach the allocation builder.
            let valid_mask = (cast[2] + reach).ceil() >= (cast[0] - reach).floor()
                && (cast[3] + reach).ceil() >= (cast[1] - reach).floor();
            let tile = if valid_mask {
                self.shadow_cache.get_or_build(key, cost, || {
                    ShadowTile::build(size, element_radius, cast_radius, cast, &edge)
                })?
            } else {
                None
            };
            if let Some(tile) = tile {
                let tx = i64::from(origin.0) + i64::from(tile.x0);
                let ty = i64::from(origin.1) + i64::from(tile.y0);
                let touched = PixelRect {
                    x0: tx.clamp(0, i64::from(width)) as usize,
                    y0: ty.clamp(0, i64::from(height)) as usize,
                    x1: (tx + tile.width as i64).clamp(0, i64::from(width)) as usize,
                    y1: (ty + tile.spans.len() as i64).clamp(0, i64::from(height)) as usize,
                };
                Self::blend_shadow_tile(
                    &mut self.pixels,
                    (width as i32, height as i32),
                    tile,
                    origin,
                    Color {
                        a: 255,
                        ..shadow.color
                    },
                    strength,
                );
                if touched.x0 < touched.x1 && touched.y0 < touched.y1 {
                    self.mark_painted(touched);
                }
                return Ok(());
            }
        }
        let tail = edge.tail;
        let reach = tail.ceil() + 1.0;
        let Some(bounds) = clip_pixels(
            (self.width, self.height),
            (cast[0] - reach).floor() as i32,
            (cast[1] - reach).floor() as i32,
            (cast[2] + reach).ceil() as i32,
            (cast[3] + reach).ceil() as i32,
        ) else {
            return Ok(());
        };
        self.mark_painted(bounds);
        let cast_radius = (radius * k + spread).max(0.0);
        let element_radius = radius * k;
        let strength = alpha * f32::from(shadow.color.a) / 255.0;
        let (center_y, half_y) = (
            (element[1] + element[3]) / 2.0,
            (element[3] - element[1]) / 2.0,
        );
        // Pixels well inside the element are hidden by it; skip them on its straight rows.
        let inside_from = (element[0] + 0.5).ceil() as usize;
        let inside_to = (element[2] - 1.5).floor().max(0.0) as usize;
        let (blend_color, stride) = (
            Color {
                a: 255,
                ..shadow.color
            },
            self.width as usize,
        );
        for y in bounds.y0..bounds.y1 {
            let py = y as f32 + 0.5;
            let straight = (py - center_y).abs() <= half_y - element_radius - 1.0;
            let mut x = bounds.x0;
            while x < bounds.x1 {
                if straight && x >= inside_from && x <= inside_to {
                    x = inside_to + 1;
                    continue;
                }
                let px = x as f32 + 0.5;
                let distance = rounded_distance(px, py, cast, cast_radius);
                let hidden = edge.coverage(distance) * strength;
                if hidden > 0.002 {
                    // CSS clips the shadow under the element itself.
                    let outside =
                        (rounded_distance(px, py, element, element_radius) + 0.5).clamp(0.0, 1.0);
                    let a = hidden * outside;
                    if a > 0.002 {
                        let offset = (y * stride + x) * 4;
                        Self::blend_pixel(&mut self.pixels[offset..offset + 4], blend_color, a);
                    }
                }
                x += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_size_shadow_masks_remain_valid_empty_tiles() {
        let edge = GaussianEdge::new(2.0).unwrap();
        let tile = ShadowTile::build((16, 16), 0.0, 0.0, [15.0, 15.0, 1.0, 1.0], &edge);
        assert_eq!(tile.width, 0);
        assert!(tile.mask.is_empty());
        assert!(tile.spans.is_empty());
    }

    #[test]
    fn collapsed_negative_spread_never_builds_a_negative_size_mask() {
        let mut surface = Surface::filled(32, 32, 1.0, Color::BLACK);
        let shadow = Shadow {
            x: 0.0,
            y: 0.0,
            blur: 24.0,
            spread: -100.0,
            color: Color::WHITE,
            inset: false,
        };
        let rect = Rect {
            x: 8.0,
            y: 8.0,
            w: 16.0,
            h: 16.0,
        };
        for _ in 0..3 {
            surface.draw_shadow(rect, 0.0, shadow, 1.0, 1.0).unwrap();
        }
        assert!(surface
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 0, 0, 255]));
    }

    #[test]
    fn cached_mask_matches_direct_drawing_closely() {
        let shadow = Shadow {
            x: 0.0,
            y: 8.0,
            blur: 24.0,
            spread: 0.0,
            color: Color {
                r: 0,
                g: 0,
                b: 0,
                a: 200,
            },
            inset: false,
        };
        let rect = Rect {
            x: 20.0,
            y: 14.0,
            w: 60.0,
            h: 40.0,
        };
        let mut cached = Surface::filled(
            120,
            100,
            1.0,
            Color {
                r: 90,
                g: 120,
                b: 200,
                a: 255,
            },
        );
        for _ in 0..3 {
            cached.draw_shadow(rect, 10.0, shadow, 0.9, 1.0).unwrap();
        }
        let mut direct = Surface::filled(
            120,
            100,
            1.0,
            Color {
                r: 90,
                g: 120,
                b: 200,
                a: 255,
            },
        );
        for _ in 0..3 {
            direct.shadow_cache = Default::default();
            direct.draw_shadow(rect, 10.0, shadow, 0.9, 1.0).unwrap();
        }
        // Drawing three times on one surface also reaches the cached path from the second call on;
        // a cache reset before each call keeps `direct` on the uncached (first sighting) path.
        let worst = cached
            .pixels
            .iter()
            .zip(&direct.pixels)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 2, "cached shadow differs by {worst}");
    }
}
