//! Solid, rounded, and gradient background fills.

use crate::model::Color;
use crate::render::geometry::{clip_pixels, PixelRect, Rect};
use crate::render::paint::surface::Surface;

impl Surface {
    pub(in crate::render) fn fill(&mut self, rect: Rect, color: Color, alpha: f32, radius: f32) {
        let (edges, radius) = self.fill_edges(rect, radius);
        let [x0, y0, x1, y1] = edges;
        let a = (alpha * color.a as f32 / 255.0).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let Some(bounds) = clip_pixels((self.width, self.height), x0, y0, x1, y1) else {
            return;
        };
        let uniform_background = self.painted.is_none();
        self.mark_painted(bounds);
        if radius > 0.0 {
            self.fill_rounded(bounds, edges, radius, color, alpha);
            return;
        }
        let base = self.backdrop;
        let rows = self.rows(bounds);
        if a >= 1.0 || uniform_background {
            let pixel = if a >= 1.0 {
                [color.r, color.g, color.b, 255]
            } else {
                Self::over_background(base, color, a)
            };
            for row in rows {
                row.as_chunks_mut::<4>().0.fill(pixel);
            }
        } else if (bounds.x1 - bounds.x0) * (bounds.y1 - bounds.y0) >= 1024 {
            // Uniform color and opacity make each channel a lookup on the destination byte.
            let table = [color.r, color.g, color.b].map(|source| {
                std::array::from_fn::<_, 256, _>(|dest| {
                    (source as f32 * a + dest as f32 * (1.0 - a)).round() as u8
                })
            });
            for row in rows {
                for pixel in row.as_chunks_mut::<4>().0 {
                    pixel[0] = table[0][pixel[0] as usize];
                    pixel[1] = table[1][pixel[1] as usize];
                    pixel[2] = table[2][pixel[2] as usize];
                }
            }
        } else {
            for row in rows {
                for pixel in row.as_chunks_mut::<4>().0 {
                    pixel[0] = (color.r as f32 * a + pixel[0] as f32 * (1.0 - a)).round() as u8;
                    pixel[1] = (color.g as f32 * a + pixel[1] as f32 * (1.0 - a)).round() as u8;
                    pixel[2] = (color.b as f32 * a + pixel[2] as f32 * (1.0 - a)).round() as u8;
                }
            }
        }
    }

    pub(in crate::render) fn fill_edges(&self, rect: Rect, radius: f32) -> ([i32; 4], f32) {
        let x0 = (rect.x * self.scale).floor() as i32;
        let y0 = (rect.y * self.scale).floor() as i32;
        let x1 = ((rect.x + rect.w) * self.scale).ceil() as i32;
        let y1 = ((rect.y + rect.h) * self.scale).ceil() as i32;
        let radius = (radius * self.scale)
            .min(x1.saturating_sub(x0).min(y1.saturating_sub(y0)) as f32 * 0.5);
        ([x0, y0, x1, y1], radius)
    }

    pub(in crate::render) fn fill_gradient(
        &mut self,
        rect: Rect,
        [from, to]: [Color; 2],
        horizontal: bool,
        alpha: f32,
        radius: f32,
    ) {
        let (edges, radius) = self.fill_edges(rect, radius);
        let [x0, y0, x1, y1] = edges;
        let Some(bounds) = clip_pixels((self.width, self.height), x0, y0, x1, y1) else {
            return;
        };
        let uniform_background = self.painted.is_none();
        self.mark_painted(bounds);
        let float_edges = edges.map(|edge| edge as f32);
        let [start, end] = if horizontal {
            [float_edges[0], float_edges[2]]
        } else {
            [float_edges[1], float_edges[3]]
        };
        let color_at = |position: usize| {
            let t = (position as f32 + 0.5 - start) / (end - start).max(1.0);
            let channel =
                |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
            Color {
                r: channel(from.r, to.r),
                g: channel(from.g, to.g),
                b: channel(from.b, to.b),
                a: 255,
            }
        };
        let columns: Vec<_> = if horizontal {
            (bounds.x0..bounds.x1)
                .map(|x| {
                    let c = color_at(x);
                    [c.r, c.g, c.b, 255]
                })
                .collect()
        } else {
            Vec::new()
        };
        let base = self.backdrop;
        // On the first layer every uncovered pixel starts with the same color.
        let blended_columns = (horizontal && uniform_background && alpha < 1.0).then(|| {
            columns
                .iter()
                .map(|&[r, g, b, a]| Self::over_background(base, Color { r, g, b, a }, alpha))
                .collect::<Vec<_>>()
        });
        let interior_columns = blended_columns.as_deref().unwrap_or(&columns);
        let copy_interior = alpha >= 1.0 || uniform_background;
        for (y, row) in (bounds.y0..bounds.y1).zip(self.rows(bounds)) {
            let pixels = row.as_chunks_mut::<4>().0;
            let solid = color_at(y);
            let solid_pixel = if !horizontal && uniform_background && alpha < 1.0 {
                Self::over_background(base, solid, alpha)
            } else {
                [solid.r, solid.g, solid.b, 255]
            };
            if radius == 0.0 && copy_interior {
                if horizontal {
                    pixels.copy_from_slice(interior_columns);
                } else {
                    pixels.fill(solid_pixel);
                }
                continue;
            }
            let (left, right) = Self::rounded_span(float_edges, radius, y);
            let left_pixel = (left.floor() as usize).clamp(bounds.x0, bounds.x1) - bounds.x0;
            let right_pixel = (right.ceil() as usize).clamp(bounds.x0, bounds.x1) - bounds.x0;
            let inner_left = (left.ceil() as usize).clamp(bounds.x0, bounds.x1) - bounds.x0;
            let inner_right = (right.floor() as usize).clamp(bounds.x0, bounds.x1) - bounds.x0;
            let paint = |pixel: &mut [u8; 4], i: usize, coverage: f32| {
                let color = if horizontal {
                    let [r, g, b, a] = columns[i];
                    Color { r, g, b, a }
                } else {
                    solid
                };
                Self::blend_pixel(pixel, color, alpha * coverage);
            };
            if copy_interior && inner_left < inner_right {
                if horizontal {
                    pixels[inner_left..inner_right]
                        .copy_from_slice(&interior_columns[inner_left..inner_right]);
                } else {
                    pixels[inner_left..inner_right].fill(solid_pixel);
                }
            } else if inner_left < inner_right {
                for (i, pixel) in pixels
                    .iter_mut()
                    .enumerate()
                    .take(inner_right)
                    .skip(inner_left)
                {
                    paint(pixel, i, 1.0);
                }
            }
            for i in [left_pixel, right_pixel.saturating_sub(1)] {
                if i >= left_pixel && i < right_pixel && !(inner_left..inner_right).contains(&i) {
                    let x = (bounds.x0 + i) as f32;
                    let coverage = (right.min(x + 1.0) - left.max(x)).clamp(0.0, 1.0);
                    paint(&mut pixels[i], i, coverage);
                    if right_pixel - left_pixel == 1 {
                        break;
                    }
                }
            }
        }
    }

    pub(in crate::render) fn rounded_span(
        [x0, y0, x1, y1]: [f32; 4],
        radius: f32,
        y: usize,
    ) -> (f32, f32) {
        let center = y as f32 + 0.5;
        let dy = (y0 + radius - center).max(center - (y1 - radius)).max(0.0);
        let inset = radius - (radius * radius - dy * dy).max(0.0).sqrt();
        (x0 + inset, x1 - inset)
    }

    pub(in crate::render) fn fill_rounded(
        &mut self,
        bounds: PixelRect,
        edges: [i32; 4],
        radius: f32,
        color: Color,
        alpha: f32,
    ) {
        let edges = edges.map(|edge| edge as f32);
        let width = self.width;
        let height = self.height;
        let a = alpha * color.a as f32 / 255.0;
        // Rounded interiors have constant source color and opacity, just like square fills.
        // Keep the antialiased edge on its original coverage-aware path.
        let table = (a > 0.0
            && a < 1.0
            && (bounds.x1 - bounds.x0) * (bounds.y1 - bounds.y0) >= 1024)
            .then(|| {
                [color.r, color.g, color.b].map(|source| {
                    std::array::from_fn::<_, 256, _>(|dest| {
                        (source as f32 * a + dest as f32 * (1.0 - a)).round() as u8
                    })
                })
            });
        for y in bounds.y0..bounds.y1 {
            let (left, right) = Self::rounded_span(edges, radius, y);
            let inner_left = (left.ceil() as usize).clamp(bounds.x0, bounds.x1);
            let inner_right = (right.floor() as usize).clamp(inner_left, bounds.x1);
            for x in [inner_left.saturating_sub(1), inner_right] {
                if x >= bounds.x0 && x < bounds.x1 {
                    let coverage = (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0);
                    Self::blend_at(
                        &mut self.pixels,
                        width,
                        height,
                        x as i32,
                        y as i32,
                        color,
                        alpha * coverage,
                    );
                }
            }
            if inner_left == inner_right {
                continue;
            }
            let row = PixelRect {
                x0: inner_left,
                y0: y,
                x1: inner_right,
                y1: y + 1,
            };
            for row in self.rows(row) {
                let pixels = row.as_chunks_mut::<4>().0;
                if a >= 1.0 {
                    pixels.fill([color.r, color.g, color.b, 255]);
                } else if let Some(table) = &table {
                    for pixel in pixels {
                        pixel[0] = table[0][pixel[0] as usize];
                        pixel[1] = table[1][pixel[1] as usize];
                        pixel[2] = table[2][pixel[2] as usize];
                        pixel[3] = 255;
                    }
                } else {
                    for pixel in pixels {
                        Self::blend_pixel(pixel, color, a);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_lookup_preserves_interior_blending_and_alpha() {
        let color = Color {
            r: 17,
            g: 139,
            b: 241,
            a: 173,
        };
        for alpha in [0.01, 0.37, 1.0] {
            let mut surface = Surface::filled(260, 8, 1.0, Color::BLACK);
            for (i, p) in surface.pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *p = [i as u8, (i / 3) as u8, (255 - i % 256) as u8, 31];
            }
            let before = surface.pixels.clone();
            surface.fill_rounded(
                PixelRect {
                    x0: 0,
                    y0: 0,
                    x1: 260,
                    y1: 8,
                },
                [0, 0, 260, 8],
                2.0,
                color,
                alpha,
            );
            for y in 0..8 {
                for x in 2..258 {
                    let offset = (y * 260 + x) * 4;
                    let mut expected = before[offset..offset + 4].to_vec();
                    Surface::blend_pixel(&mut expected, color, alpha * color.a as f32 / 255.0);
                    assert_eq!(&surface.pixels[offset..offset + 4], expected);
                }
            }
        }
    }
}
