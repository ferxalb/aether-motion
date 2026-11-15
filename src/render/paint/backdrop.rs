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

/// Three horizontal then three vertical box blurs over a `width` x `height` plane.
fn blur_plane(plane: &mut [f32], width: usize, height: usize, radius: usize) {
    let mut tmp = Vec::with_capacity(width.max(height));
    for _ in 0..3 {
        for row in plane.chunks_exact_mut(width) {
            box_line(row, &mut tmp, radius);
        }
    }
    let mut column = vec![0.0_f32; height];
    for x in 0..width {
        for (y, value) in column.iter_mut().enumerate() {
            *value = plane[y * width + x];
        }
        for _ in 0..3 {
            box_line(&mut column, &mut tmp, radius);
        }
        for (y, value) in column.iter().enumerate() {
            plane[y * width + x] = *value;
        }
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
}
