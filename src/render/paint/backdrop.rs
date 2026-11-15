//! `backdrop-filter: blur()`: blurs what is already painted behind an element, inside its rounded box.
//!
//! Three successive box blurs approximate the gaussian (as in CSS the radius is the standard deviation).
//! Only the pixels inside the box are sampled and edge samples are clamped, so the result is a pure
//! function of the pixels behind the element and stays deterministic.

use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::Surface;

/// Radius in pixels of each box blur so that three of them approximate a gaussian of `sigma`.
fn box_radius(sigma: f32) -> usize {
    let width = (4.0 * sigma * sigma + 1.0).sqrt();
    ((width - 1.0) / 2.0).round().max(0.0) as usize
}

/// One box blur of radius `radius` over `line`, clamping samples at both ends. `tmp` is scratch space.
fn box_line(line: &mut [f32], tmp: &mut Vec<f32>, radius: usize) {
    let n = line.len();
    if n == 0 || radius == 0 {
        return;
    }
    let r = radius as isize;
    let last = n as isize - 1;
    let window = (2 * radius + 1) as f32;
    let mut sum: f32 = (-r..=r).map(|k| line[k.clamp(0, last) as usize]).sum();
    tmp.clear();
    for i in 0..n as isize {
        tmp.push(sum / window);
        sum += line[(i + r + 1).min(last) as usize] - line[(i - r).max(0) as usize];
    }
    line.copy_from_slice(tmp);
}

/// One vertical box blur of radius `radius` from `src` into `dst` (both `width` x `height`).
///
/// A running sum per column slides down the rows, so memory is read row by row (cache friendly)
/// and the cost does not depend on the radius. `sums` is scratch space of `width` values.
fn blur_vertical(
    src: &[f32],
    dst: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    sums: &mut Vec<f32>,
) {
    if width == 0 || height == 0 || radius == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let r = radius as isize;
    let last = height as isize - 1;
    let window = (2 * radius + 1) as f32;
    let row = |y: isize| {
        let y = y.clamp(0, last) as usize;
        &src[y * width..(y + 1) * width]
    };
    sums.clear();
    sums.resize(width, 0.0);
    for k in -r..=r {
        for (sum, value) in sums.iter_mut().zip(row(k)) {
            *sum += value;
        }
    }
    for y in 0..height as isize {
        let out = &mut dst[y as usize * width..(y as usize + 1) * width];
        for (target, sum) in out.iter_mut().zip(sums.iter()) {
            *target = sum / window;
        }
        let (add, sub) = (row(y + r + 1), row(y - r));
        for ((sum, a), s) in sums.iter_mut().zip(add).zip(sub) {
            *sum += a - s;
        }
    }
}

/// Three horizontal then three vertical box blurs over a `width` x `height` plane.
fn blur_plane(plane: &mut [f32], width: usize, height: usize, radius: usize) {
    let mut tmp = Vec::with_capacity(width.max(height));
    for _ in 0..3 {
        for row in plane.chunks_exact_mut(width) {
            box_line(row, &mut tmp, radius);
        }
    }
    let mut scratch = vec![0.0_f32; plane.len()];
    let mut sums = Vec::new();
    for _ in 0..3 {
        blur_vertical(plane, &mut scratch, width, height, radius, &mut sums);
        plane.copy_from_slice(&scratch);
    }
}

impl Surface {
    /// Replaces the pixels inside `rect` (rounded by `radius`) with their blurred version.
    ///
    /// `sigma` is the blur's standard deviation in output pixels; `alpha` mixes the blur with the original.
    pub(in crate::render) fn backdrop_blur(
        &mut self,
        rect: Rect,
        radius: f32,
        sigma: f32,
        alpha: f32,
    ) {
        let box_r = box_radius(sigma);
        if box_r == 0 || alpha <= 0.0 {
            return;
        }
        let (edges, radius_px) = self.fill_edges(rect, radius);
        let [x0, y0, x1, y1] = edges;
        let Some(bounds) = clip_pixels((self.width, self.height), x0, y0, x1, y1) else {
            return;
        };
        let (w, h) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        let stride = self.width as usize * 4;
        let mut planes = vec![vec![0.0_f32; w * h]; 3];
        for y in 0..h {
            let base = (bounds.y0 + y) * stride + bounds.x0 * 4;
            for x in 0..w {
                for (c, plane) in planes.iter_mut().enumerate() {
                    plane[y * w + x] = f32::from(self.pixels[base + x * 4 + c]);
                }
            }
        }
        for plane in &mut planes {
            blur_plane(plane, w, h, box_r);
        }
        self.mark_painted(bounds);
        let float_edges = edges.map(|edge| edge as f32);
        for y in bounds.y0..bounds.y1 {
            let (left, right) = Self::rounded_span(float_edges, radius_px, y);
            let first = left.floor().max(bounds.x0 as f32) as usize;
            let last = right.ceil().min(bounds.x1 as f32) as usize;
            for x in first..last {
                let coverage = (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0);
                let mix = coverage * alpha.min(1.0);
                if mix <= 0.0 {
                    continue;
                }
                let blurred = (y - bounds.y0) * w + (x - bounds.x0);
                let offset = y * stride + x * 4;
                for (c, plane) in planes.iter().enumerate() {
                    let old = f32::from(self.pixels[offset + c]);
                    self.pixels[offset + c] = (plane[blurred] * mix + old * (1.0 - mix))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
                self.pixels[offset + 3] = 255;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Color;

    #[test]
    fn radius_follows_sigma() {
        assert_eq!(box_radius(0.0), 0);
        assert_eq!(box_radius(10.0), 10);
        assert!(box_radius(30.0) > box_radius(10.0));
    }

    #[test]
    fn blur_keeps_flat_areas_and_spreads_edges() {
        let mut flat = vec![7.0_f32; 16];
        blur_plane(&mut flat, 4, 4, 2);
        assert!(flat.iter().all(|v| (v - 7.0).abs() < 1e-3));
        let mut line = vec![0.0, 0.0, 100.0, 0.0, 0.0];
        let mut tmp = Vec::new();
        box_line(&mut line, &mut tmp, 1);
        assert!((line[1] - 100.0 / 3.0).abs() < 1e-3);
        assert!((line[2] - 100.0 / 3.0).abs() < 1e-3);
        assert_eq!(line[0], 0.0);
    }

    fn naive_vertical(src: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
        let mut out = vec![0.0; src.len()];
        for y in 0..height {
            for x in 0..width {
                let total: f32 = (-(radius as isize)..=radius as isize)
                    .map(|k| {
                        let row = (y as isize + k).clamp(0, height as isize - 1) as usize;
                        src[row * width + x]
                    })
                    .sum();
                out[y * width + x] = total / (2 * radius + 1) as f32;
            }
        }
        out
    }

    #[test]
    fn the_sliding_vertical_pass_matches_a_naive_box_blur() {
        let (w, h) = (7, 11);
        let src: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 101) as f32).collect();
        for radius in [1, 2, 5, 20] {
            let mut dst = vec![0.0; src.len()];
            blur_vertical(&src, &mut dst, w, h, radius, &mut Vec::new());
            let expected = naive_vertical(&src, w, h, radius);
            for (got, want) in dst.iter().zip(&expected) {
                assert!(
                    (got - want).abs() < 1e-2,
                    "radius {radius}: {got} vs {want}"
                );
            }
        }
    }

    fn white_left_half() -> Surface {
        let mut surface = Surface::filled(40, 20, 1.0, Color::BLACK);
        let left = Rect {
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        surface.fill(left, Color::WHITE, 1.0, 0.0);
        surface
    }

    fn red(surface: &Surface, x: u32, y: u32) -> u8 {
        surface.pixels[((y * surface.width + x) * 4) as usize]
    }

    #[test]
    fn blur_softens_an_edge_inside_the_box_and_leaves_the_rest_alone() {
        let mut surface = white_left_half();
        let region = Rect {
            x: 10.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        surface.backdrop_blur(region, 0.0, 3.0, 1.0);
        // Either side of the edge at x = 20 moves towards the middle.
        assert!(
            (1..255).contains(&red(&surface, 19, 10)),
            "{}",
            red(&surface, 19, 10)
        );
        assert!(
            (1..255).contains(&red(&surface, 20, 10)),
            "{}",
            red(&surface, 20, 10)
        );
        assert!(red(&surface, 19, 10) > red(&surface, 20, 10));
        // Outside the box nothing changes.
        assert_eq!(red(&surface, 5, 10), 255);
        assert_eq!(red(&surface, 35, 10), 0);
        // Flat areas inside the box stay flat, and the alpha channel stays opaque.
        assert_eq!(red(&surface, 10, 10), 255);
        assert_eq!(
            surface.pixels[((10 * surface.width + 20) * 4 + 3) as usize],
            255
        );
    }

    #[test]
    fn blur_is_a_no_op_without_alpha_or_radius() {
        let original = white_left_half().pixels;
        let region = Rect {
            x: 10.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        let mut hidden = white_left_half();
        hidden.backdrop_blur(region, 0.0, 3.0, 0.0);
        assert_eq!(hidden.pixels, original);
        let mut sharp = white_left_half();
        sharp.backdrop_blur(region, 0.0, 0.0, 1.0);
        assert_eq!(sharp.pixels, original);
    }

    #[test]
    fn partial_alpha_mixes_the_blur_with_the_original() {
        let region = Rect {
            x: 10.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        let mut full = white_left_half();
        full.backdrop_blur(region, 0.0, 3.0, 1.0);
        let mut half = white_left_half();
        half.backdrop_blur(region, 0.0, 3.0, 0.5);
        let (f, h) = (i32::from(red(&full, 19, 10)), i32::from(red(&half, 19, 10)));
        assert!(h > f && h < 255, "{f} < {h} < 255");
    }

    #[test]
    fn rounded_corners_are_left_unblurred() {
        let mut surface = Surface::filled(40, 40, 1.0, Color::BLACK);
        let stripe = Rect {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 5.0,
        };
        surface.fill(stripe, Color::WHITE, 1.0, 0.0);
        let before = red(&surface, 10, 0);
        // A 30 px box with 15 px radius is a circle: its corner pixel is outside the shape.
        let region = Rect {
            x: 10.0,
            y: 0.0,
            w: 30.0,
            h: 30.0,
        };
        surface.backdrop_blur(region, 15.0, 4.0, 1.0);
        assert_eq!(red(&surface, 10, 0), before);
        assert!(red(&surface, 25, 4) < 255);
    }
}
