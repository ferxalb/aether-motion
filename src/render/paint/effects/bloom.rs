//! `data-hf-bloom`: a soft halo of an element's bright pixels, added over the frame after the
//! element itself is composited, so the element stays exactly as sharp as without bloom.
//!
//! The halo is computed inside the element's effect layer only (its box plus the halo reach):
//! 1. Bright pass on the layer's final premultiplied texels (after color functions and pixel
//!    effects): a texel of luminance `L` keeps `max(0, L - threshold) / ((1 - threshold) L)` of its
//!    color, so pixels at or under the threshold cast nothing and an opaque white pixel casts its
//!    whole color. The bright texels are box-averaged into a grid reduced by 1, 2, or 4 (see
//!    [`reduction`]).
//! 2. Gaussian blur of the reduced grid (three box passes, `backdrop::blur_plane`) with the radius
//!    divided by the reduction. The reduced grid is padded by the blur reach, so it fades to zero
//!    before its edges.
//! 3. Bilinear upsampling, multiplied by the intensity and the tint, and added to the frame with a
//!    4x4 ordered dither before rounding, so faint halos on dark backgrounds do not band.
//!
//! The work is bounded by the layer, and the reduced planes live in a [`BloomScratch`] that the
//! renderer keeps across frames. Every step is a pure function of the layer, so output does not
//! depend on threads or segments.

use crate::model::{Bloom, Color};
use crate::render::geometry::PixelRect;
use crate::render::paint::backdrop::{blur_plane, box_radius};
use crate::render::paint::effects::blend::Layer;

/// Reduced planes (red, green, blue) and a row buffer, reused across frames.
#[derive(Default)]
pub(in crate::render) struct BloomScratch {
    planes: [Vec<f32>; 3],
    row: Vec<[f32; 4]>,
}

/// Grid reduction for a halo of standard deviation `sigma` output px: blurs narrower than 6 px stay
/// full size; wider ones keep at least 3 px of deviation on the reduced grid, where bilinear
/// upsampling of the (already very smooth) halo is visually exact.
pub(super) fn reduction(sigma: f32) -> usize {
    if sigma < 6.0 {
        1
    } else if sigma < 16.0 {
        2
    } else {
        4
    }
}

/// One bloom draw in output pixels.
#[derive(Clone, Copy)]
pub(super) struct BloomPass {
    pub(super) intensity: f32,
    pub(super) threshold: f32,
    /// Gaussian standard deviation of the halo, in output px.
    pub(super) sigma: f32,
    pub(super) tint: Option<Color>,
}

impl BloomPass {
    pub(super) fn new(bloom: Bloom, intensity: f32, px_per_unit: f32) -> Option<Self> {
        (intensity > 0.0).then_some(Self {
            intensity,
            threshold: bloom.threshold,
            sigma: bloom.radius / 2.0 * px_per_unit,
            tint: bloom.tint,
        })
    }

    /// Output px the halo reaches past the pixels that cast it (three box passes, plus the
    /// reduction's rounding).
    pub(super) fn reach(self) -> i32 {
        let factor = reduction(self.sigma);
        let box_r = box_radius(self.sigma / factor as f32);
        ((3 * box_r + 2) * factor) as i32
    }

    /// Adds the halo of `layer` (`lw` x `lh` texels) to `pixels` (an RGBA frame `stride` pixels
    /// wide) inside `bounds`.
    pub(super) fn apply(
        self,
        layer: &Layer<'_>,
        lh: usize,
        pixels: &mut [u8],
        stride: usize,
        bounds: PixelRect,
        scratch: &mut BloomScratch,
    ) {
        let lw = layer.width;
        let factor = reduction(self.sigma);
        let box_r = box_radius(self.sigma / factor as f32);
        // Zero texels around the grid let the blur fade out instead of clamping at the edge.
        let margin = 3 * box_r + 1;
        let gw = lw.div_ceil(factor) + 2 * margin;
        let gh = lh.div_ceil(factor) + 2 * margin;
        let BloomScratch { planes, row } = scratch;
        for plane in planes.iter_mut() {
            plane.clear();
            plane.resize(gw * gh, 0.0);
        }
        // 1. Bright pass, box-averaged into the reduced grid.
        let knee = 1.0 / (1.0 - self.threshold);
        let threshold = self.threshold * 255.0;
        let cell = 1.0 / (factor * factor) as f32;
        let mut lit = false;
        for y in 0..lh {
            layer.fill_row(y * lw, lw, row);
            let gy = y / factor + margin;
            for (x, texel) in row.iter().enumerate() {
                let [r, g, b, _] = *texel;
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                if luma <= threshold {
                    continue;
                }
                let keep = (luma - threshold) * knee / luma * cell;
                let i = gy * gw + x / factor + margin;
                planes[0][i] += r * keep;
                planes[1][i] += g * keep;
                planes[2][i] += b * keep;
                lit = true;
            }
        }
        if !lit {
            return;
        }
        // 2. Blur on the reduced grid.
        for plane in planes.iter_mut() {
            blur_plane(plane, gw, gh, box_r);
        }
        // 3. Bilinear upsampling, tint, intensity, and a dithered add.
        let gain = self.tint.map_or([self.intensity; 3], |tint| {
            [tint.r, tint.g, tint.b].map(|c| f32::from(c) / 255.0 * self.intensity)
        });
        let (ox, oy) = layer.origin;
        let inv = 1.0 / factor as f32;
        for y in bounds.y0..bounds.y1 {
            let ly = (y as i64 - i64::from(oy)) as f32;
            let gy = (ly + 0.5) * inv - 0.5 + margin as f32;
            let y0 = gy.floor();
            let fy = gy - y0;
            let y0 = (y0.max(0.0) as usize).min(gh - 1);
            let y1 = (y0 + 1).min(gh - 1);
            let dither = DITHER[y & 3];
            let line = &mut pixels[(y * stride + bounds.x0) * 4..(y * stride + bounds.x1) * 4];
            for (dx, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let x = bounds.x0 + dx;
                let lx = (x as i64 - i64::from(ox)) as f32;
                let gx = (lx + 0.5) * inv - 0.5 + margin as f32;
                let x0 = gx.floor();
                let fx = gx - x0;
                let x0 = (x0.max(0.0) as usize).min(gw - 1);
                let x1 = (x0 + 1).min(gw - 1);
                let d = dither[x & 3];
                for c in 0..3 {
                    let p = &planes[c];
                    let top = p[y0 * gw + x0] + (p[y0 * gw + x1] - p[y0 * gw + x0]) * fx;
                    let bottom = p[y1 * gw + x0] + (p[y1 * gw + x1] - p[y1 * gw + x0]) * fx;
                    let halo = (top + (bottom - top) * fy) * gain[c];
                    if halo > 0.0 {
                        let value = f32::from(pixel[c]) + halo + d;
                        pixel[c] = value.clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
}

/// 4x4 Bayer thresholds in `[0, 1)`, added before truncation (an ordered-dither rounding).
const DITHER: [[f32; 4]; 4] = {
    const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let mut out = [[0.0; 4]; 4];
    let mut y = 0;
    while y < 4 {
        let mut x = 0;
        while x < 4 {
            out[y][x] = (BAYER[y][x] as f32 + 0.5) / 16.0;
            x += 1;
        }
        y += 1;
    }
    out
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::paint::effects::blend::Texels;

    fn pass(sigma: f32) -> BloomPass {
        BloomPass {
            intensity: 1.0,
            threshold: 0.5,
            sigma,
            tint: None,
        }
    }

    /// A `size` x `size` layer of four planes with one opaque square of `value` in the middle.
    fn square(size: usize, value: f32) -> Vec<Vec<f32>> {
        let mut planes = vec![vec![0.0; size * size]; 4];
        for y in size / 2 - 4..size / 2 + 4 {
            for x in size / 2 - 4..size / 2 + 4 {
                for (c, plane) in planes.iter_mut().enumerate() {
                    plane[y * size + x] = if c == 3 { 255.0 } else { value };
                }
            }
        }
        planes
    }

    fn halo(planes: &[Vec<f32>], size: usize, sigma: f32) -> Vec<u8> {
        let layer = Layer {
            texels: Texels::Planes(planes),
            width: size,
            origin: (0, 0),
        };
        let mut pixels = vec![0_u8; size * size * 4];
        let bounds = PixelRect {
            x0: 0,
            y0: 0,
            x1: size,
            y1: size,
        };
        let mut scratch = BloomScratch::default();
        pass(sigma).apply(&layer, size, &mut pixels, size, bounds, &mut scratch);
        pixels
    }

    #[test]
    fn dim_pixels_cast_no_halo() {
        let planes = square(64, 100.0);
        assert!(halo(&planes, 64, 4.0).iter().all(|&v| v == 0));
    }

    #[test]
    fn bright_pixels_glow_around_themselves_symmetrically() {
        for sigma in [3.0, 8.0, 20.0] {
            let size = 128;
            let pixels = halo(&square(size, 255.0), size, sigma);
            let at = |x: usize, y: usize| pixels[(y * size + x) * 4];
            // Outside the square the halo is lit and falls off with distance.
            let near = at(size / 2 + 6, size / 2);
            let far = at(size / 2 + 20, size / 2);
            assert!(near > far && near > 0, "sigma {sigma}: {near} {far}");
            // Left and right agree within the dither.
            let left = at(size / 2 - 7, size / 2) as i32;
            let right = at(size / 2 + 6, size / 2) as i32;
            assert!((left - right).abs() <= 2, "sigma {sigma}: {left} {right}");
            // The reach bounds the halo.
            let reach = pass(sigma).reach() as usize;
            if size / 2 + 4 + reach < size {
                assert_eq!(at(size / 2 + 4 + reach, size / 2), 0, "sigma {sigma}");
            }
        }
    }

    #[test]
    fn the_halo_is_deterministic() {
        let planes = square(96, 240.0);
        assert_eq!(halo(&planes, 96, 18.0), halo(&planes, 96, 18.0));
    }
}
