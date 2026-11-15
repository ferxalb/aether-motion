//! Uniform solid borders drawn inside the box, skipping guaranteed empty interior spans.

use crate::model::Color;
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::shape::rounded_distance;
use crate::render::paint::surface::Surface;

impl Surface {
    /// Solid ring between the rounded box and the same box inset by `width`.
    pub(in crate::render) fn draw_border(
        &mut self,
        rect: Rect,
        radius: f32,
        width: f32,
        color: Color,
        alpha: f32,
    ) {
        let k = self.scale;
        let outer = [
            rect.x * k,
            rect.y * k,
            (rect.x + rect.w) * k,
            (rect.y + rect.h) * k,
        ];
        let width = (width * k).min((outer[2] - outer[0]).min(outer[3] - outer[1]) / 2.0);
        let inner = [
            outer[0] + width,
            outer[1] + width,
            outer[2] - width,
            outer[3] - width,
        ];
        let (outer_radius, inner_radius) = (radius * k, (radius * k - width).max(0.0));
        let Some(bounds) = clip_pixels(
            (self.width, self.height),
            outer[0].floor() as i32,
            outer[1].floor() as i32,
            outer[2].ceil() as i32,
            outer[3].ceil() as i32,
        ) else {
            return;
        };
        self.mark_painted(bounds);
        let strength = alpha * f32::from(color.a) / 255.0;
        let blend_color = Color { a: 255, ..color };
        let (center_y, half_y) = ((inner[1] + inner[3]) / 2.0, (inner[3] - inner[1]) / 2.0);
        // On straight rows everything well inside the inner box is untouched.
        let inside_from = (inner[0] + 0.5).ceil() as usize;
        let inside_to = (inner[2] - 1.5).floor().max(0.0) as usize;
        let stride = self.width as usize;
        for y in bounds.y0..bounds.y1 {
            let py = y as f32 + 0.5;
            let straight = (py - center_y).abs() <= half_y - inner_radius - 1.0;
            // The middle rectangle remains fully inside the rounded hole even on curved
            // rows. Skip it conservatively, retaining the original distance calculation
            // at every potentially visible edge pixel.
            let (skip_from, skip_to) = if straight {
                (inside_from, inside_to)
            } else if py >= inner[1] + 1.0 && py <= inner[3] - 1.0 {
                (
                    (inner[0] + inner_radius + 1.0).ceil().max(0.0) as usize,
                    (inner[2] - inner_radius - 2.0).floor().max(0.0) as usize,
                )
            } else {
                (1, 0)
            };
            let mut x = bounds.x0;
            while x < bounds.x1 {
                if x >= skip_from && x <= skip_to {
                    x = skip_to + 1;
                    continue;
                }
                let px = x as f32 + 0.5;
                let outside = (0.5 - rounded_distance(px, py, outer, outer_radius)).clamp(0.0, 1.0);
                let hole = (0.5 - rounded_distance(px, py, inner, inner_radius)).clamp(0.0, 1.0);
                let a = strength * outside * (1.0 - hole);
                if a > 0.002 {
                    let offset = (y * stride + x) * 4;
                    Self::blend_pixel(&mut self.pixels[offset..offset + 4], blend_color, a);
                }
                x += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_hole_matches_full_distance_scan() {
        for radius in [0.0, 4.0, 30.0, 80.0] {
            for width in [0.5, 3.0, 18.0] {
                let rect = Rect {
                    x: -8.25,
                    y: 2.75,
                    w: 144.0,
                    h: 96.0,
                };
                let color = Color {
                    r: 30,
                    g: 180,
                    b: 231,
                    a: 179,
                };
                let mut actual = Surface::filled(128, 104, 1.0, Color::BLACK);
                actual.draw_border(rect, radius, width, color, 0.73);
                let mut expected = Surface::filled(128, 104, 1.0, Color::BLACK);
                let outer = [rect.x, rect.y, rect.x + rect.w, rect.y + rect.h];
                let inner = [
                    outer[0] + width,
                    outer[1] + width,
                    outer[2] - width,
                    outer[3] - width,
                ];
                for y in 2..99 {
                    for x in 0..128 {
                        let px = x as f32 + 0.5;
                        let py = y as f32 + 0.5;
                        let outside =
                            (0.5 - rounded_distance(px, py, outer, radius)).clamp(0.0, 1.0);
                        let hole = (0.5
                            - rounded_distance(px, py, inner, (radius - width).max(0.0)))
                        .clamp(0.0, 1.0);
                        let a = (0.73 * f32::from(color.a) / 255.0) * outside * (1.0 - hole);
                        if a > 0.002 {
                            let offset = (y * 128 + x) * 4;
                            Surface::blend_pixel(
                                &mut expected.pixels[offset..offset + 4],
                                Color { a: 255, ..color },
                                a,
                            );
                        }
                    }
                }
                assert_eq!(
                    actual.pixels, expected.pixels,
                    "radius {radius}, width {width}"
                );
            }
        }
    }
}
