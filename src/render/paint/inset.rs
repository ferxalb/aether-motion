//! Inset `box-shadow`: a soft shadow cast inside the padding box, following its rounded corners.
//!
//! The shadow is the complement of a blurred "hole" (the box shifted by the offset and shrunk by the
//! spread), clipped to the box itself. It paints above the background and below the border and content.
//! Guaranteed zero-shadow spans on curved rows are skipped without approximating edge coverage.

use crate::model::{Color, Shadow};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::shape::{rounded_distance, GaussianEdge};
use crate::render::paint::surface::Surface;

/// A conservative central strip at least the blur reach inside the rounded hole.
/// The extra pixel also covers the hard-edge profile and floating-point boundaries.
fn curved_zero_span(hole: [f32; 4], radius: f32, tail: f32, py: f32) -> Option<(usize, usize)> {
    if hole[2] <= hole[0]
        || hole[3] <= hole[1]
        || py < hole[1] + tail + 1.0
        || py > hole[3] - tail - 1.0
    {
        return None;
    }
    let first = (hole[0] + radius + tail + 1.0).ceil().max(0.0) as usize;
    let last = (hole[2] - radius - tail - 1.0).floor();
    (last >= first as f32).then_some((first, last.max(0.0) as usize))
}

impl Surface {
    /// Draws one inset shadow inside `rect` (the padding box, rounded by `radius`).
    pub(in crate::render) fn draw_inset_shadow(
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
        let edge = GaussianEdge::new(shadow.blur * unit / 2.0);
        let element = [
            rect.x * k,
            rect.y * k,
            (rect.x + rect.w) * k,
            (rect.y + rect.h) * k,
        ];
        let Some(bounds) = clip_pixels(
            (self.width, self.height),
            element[0].floor() as i32,
            element[1].floor() as i32,
            element[2].ceil() as i32,
            element[3].ceil() as i32,
        ) else {
            return;
        };
        // The hole is the box moved by the offset and shrunk by the spread (grown if negative).
        let hole = [
            element[0] + shadow.x * unit + spread,
            element[1] + shadow.y * unit + spread,
            element[2] + shadow.x * unit - spread,
            element[3] + shadow.y * unit - spread,
        ];
        let empty = hole[2] <= hole[0] || hole[3] <= hole[1];
        let element_radius = radius * k;
        let hole_radius = (element_radius - spread).max(0.0);
        let strength = alpha * f32::from(shadow.color.a) / 255.0;
        if strength <= 0.0 {
            return;
        }
        self.mark_painted(bounds);
        let blend_color = Color {
            a: 255,
            ..shadow.color
        };
        let stride = self.width as usize;
        // On rows where the hole has straight sides, pixels deep inside it receive no shadow.
        let (hole_center_y, hole_half_y) = ((hole[1] + hole[3]) / 2.0, (hole[3] - hole[1]) / 2.0);
        let deep_from = (hole[0] + edge.tail + 1.0).ceil().max(0.0) as usize;
        let deep_to = (hole[2] - edge.tail - 1.0).floor().max(0.0) as usize;
        for y in bounds.y0..bounds.y1 {
            let py = y as f32 + 0.5;
            let straight = !empty && (py - hole_center_y).abs() <= hole_half_y - hole_radius - 1.0;
            let (skip_from, skip_to) = if straight {
                (deep_from, deep_to)
            } else {
                curved_zero_span(hole, hole_radius, edge.tail, py).unwrap_or((1, 0))
            };
            let mut x = bounds.x0;
            while x < bounds.x1 {
                if x >= skip_from && x <= skip_to {
                    x = skip_to + 1;
                    continue;
                }
                let px = x as f32 + 0.5;
                let inside =
                    (0.5 - rounded_distance(px, py, element, element_radius)).clamp(0.0, 1.0);
                if inside > 0.0 {
                    let shade = if empty {
                        1.0
                    } else {
                        1.0 - edge.coverage(rounded_distance(px, py, hole, hole_radius))
                    };
                    let a = shade * strength * inside;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(surface: &Surface, x: u32, y: u32) -> u8 {
        surface.pixels[((y * surface.width + x) * 4) as usize]
    }

    fn shadow(x: f32, y: f32, blur: f32, spread: f32) -> Shadow {
        Shadow {
            x,
            y,
            blur,
            spread,
            color: Color::BLACK,
            inset: true,
        }
    }

    fn white_surface() -> Surface {
        Surface::filled(60, 60, 1.0, Color::WHITE)
    }

    const BOX: Rect = Rect {
        x: 10.0,
        y: 10.0,
        w: 40.0,
        h: 40.0,
    };

    #[test]
    fn curved_spans_have_exactly_zero_shadow_coverage() {
        for origin in [-240.25, -30.75, 0.0, 15.25] {
            for radius in [0.0, 4.0, 30.0, 90.0, 200.0] {
                for sigma in [0.0, 0.005, 0.01, 2.0, 12.0, 40.0] {
                    let hole = [origin, 3.25, origin + 240.0, 183.75];
                    let edge = GaussianEdge::new(sigma);
                    for y in 0..190 {
                        let py = y as f32 + 0.5;
                        if let Some((first, last)) = curved_zero_span(hole, radius, edge.tail, py) {
                            for x in first..=last {
                                let distance = rounded_distance(x as f32 + 0.5, py, hole, radius);
                                assert_eq!(edge.coverage(distance), 1.0,
                                    "origin {origin}, radius {radius}, sigma {sigma}, pixel {x},{y}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_hard_inset_shadow_darkens_the_side_opposite_to_its_offset() {
        let mut surface = white_surface();
        surface.draw_inset_shadow(BOX, 0.0, shadow(6.0, 0.0, 0.0, 0.0), 1.0, 1.0);
        // A +6px x offset exposes a 6px band on the left edge only.
        assert_eq!(gray(&surface, 12, 30), 0);
        assert_eq!(gray(&surface, 15, 30), 0);
        assert_eq!(gray(&surface, 17, 30), 255);
        assert_eq!(gray(&surface, 45, 30), 255);
        // Nothing leaks outside the box.
        assert_eq!(gray(&surface, 5, 30), 255);
    }

    #[test]
    fn a_blurred_zero_offset_shadow_fades_from_every_edge() {
        let mut surface = white_surface();
        surface.draw_inset_shadow(BOX, 0.0, shadow(0.0, 0.0, 10.0, 0.0), 1.0, 1.0);
        let (edge, mid, center) = (
            gray(&surface, 10, 30),
            gray(&surface, 14, 30),
            gray(&surface, 30, 30),
        );
        assert!(edge < mid && mid < center, "{edge} {mid} {center}");
        assert_eq!(center, 255, "the middle is far beyond the blur reach");
        assert_eq!(gray(&surface, 30, 10), gray(&surface, 10, 30), "symmetric");
    }

    #[test]
    fn spread_thickens_the_band_and_alpha_scales_it() {
        let mut thin = white_surface();
        thin.draw_inset_shadow(BOX, 0.0, shadow(0.0, 0.0, 0.0, 2.0), 1.0, 1.0);
        let mut thick = white_surface();
        thick.draw_inset_shadow(BOX, 0.0, shadow(0.0, 0.0, 0.0, 6.0), 1.0, 1.0);
        assert_eq!(gray(&thin, 13, 30), 255);
        assert_eq!(gray(&thick, 13, 30), 0);
        let mut faint = white_surface();
        faint.draw_inset_shadow(BOX, 0.0, shadow(0.0, 0.0, 0.0, 6.0), 0.5, 1.0);
        assert!((120..136).contains(&gray(&faint, 13, 30)));
    }

    #[test]
    fn an_oversized_spread_fills_the_whole_box() {
        let mut surface = white_surface();
        surface.draw_inset_shadow(BOX, 0.0, shadow(0.0, 0.0, 0.0, 25.0), 1.0, 1.0);
        assert_eq!(gray(&surface, 30, 30), 0);
        assert_eq!(gray(&surface, 5, 30), 255);
    }

    #[test]
    fn rounded_corners_clip_the_shadow() {
        let mut surface = white_surface();
        surface.draw_inset_shadow(BOX, 20.0, shadow(0.0, 0.0, 0.0, 3.0), 1.0, 1.0);
        // The box is a circle, so its corner pixel is outside the shape and stays untouched.
        assert_eq!(gray(&surface, 10, 10), 255);
        assert_eq!(gray(&surface, 11, 30), 0);
    }

    #[test]
    fn the_shadow_is_deterministic_and_follows_zoom() {
        let draw = |zoom: f32| {
            let mut s = white_surface();
            s.draw_inset_shadow(BOX, 4.0, shadow(2.0, 3.0, 8.0, 1.0), 0.8, zoom);
            s.pixels
        };
        assert_eq!(draw(1.0), draw(1.0));
        assert_ne!(draw(1.0), draw(2.0));
    }
}
