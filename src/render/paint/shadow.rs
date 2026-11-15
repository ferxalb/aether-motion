//! Soft outer box shadows that follow the rounded box.

use crate::model::{Color, Shadow};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::shape::{erfc, rounded_distance};
use crate::render::paint::surface::Surface;

impl Surface {
    /// Soft outer shadow: Gaussian falloff from the distance to the shadow's rounded rectangle.
    pub(in crate::render) fn draw_shadow(
        &mut self,
        rect: Rect,
        radius: f32,
        shadow: Shadow,
        alpha: f32,
        zoom: f32,
    ) {
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
        let tail = sigma * 3.0;
        let reach = tail.ceil() + 1.0;
        let Some(bounds) = clip_pixels(
            (self.width, self.height),
            (cast[0] - reach).floor() as i32,
            (cast[1] - reach).floor() as i32,
            (cast[2] + reach).ceil() as i32,
            (cast[3] + reach).ceil() as i32,
        ) else {
            return;
        };
        self.mark_painted(bounds);
        let cast_radius = (radius * k + spread).max(0.0);
        let element_radius = radius * k;
        let strength = alpha * f32::from(shadow.color.a) / 255.0;
        // Gaussian edge profile sampled every 1/16 px; beyond +-3 sigma it is flat.
        const STEPS_PER_PX: f32 = 16.0;
        let table: Vec<f32> = if sigma < 0.01 {
            Vec::new()
        } else {
            let n = (tail * 2.0 * STEPS_PER_PX).ceil() as usize + 1;
            (0..=n)
                .map(|i| {
                    let d = -tail + i as f32 / STEPS_PER_PX;
                    0.5 * erfc(d / (sigma * std::f32::consts::SQRT_2))
                })
                .collect()
        };
        let coverage = |distance: f32| -> f32 {
            if table.is_empty() {
                (0.5 - distance).clamp(0.0, 1.0)
            } else if distance >= tail {
                0.0
            } else if distance <= -tail {
                1.0
            } else {
                let position = (distance + tail) * STEPS_PER_PX;
                let i = (position as usize).min(table.len() - 2);
                let fraction = position - i as f32;
                table[i] + (table[i + 1] - table[i]) * fraction
            }
        };
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
                let hidden = coverage(distance) * strength;
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
    }
}
