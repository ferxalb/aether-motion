//! Frame-wide finishing applied after everything is drawn: vignette and film grain.
//!
//! Both are pure functions of pixel position (and, for grain, of the frame time), so output stays
//! deterministic and independent of render order. The vignette is a per-pixel brightness table built
//! once per surface size and strength, so a frame costs one integer multiply per channel; the grain is
//! an integer hash per pixel. All arithmetic is fixed point (1/256 steps), in branch-free row loops
//! that are also compiled for AVX2 and chosen at run time (same results on every CPU).

use crate::render::geometry::PixelRect;
use crate::render::paint::Surface;

/// Integer hash of a pixel position and seed; every bit depends on all inputs.
#[inline(always)]
fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

/// Brightness factor of the vignette at normalized distance `d` (0 center, 1 corner).
fn vignette_factor(strength: f32, d: f32) -> f32 {
    let t = ((d - 0.35) / 0.65).clamp(0.0, 1.0);
    1.0 - strength * t * t * (3.0 - 2.0 * t)
}

/// Per-pixel vignette brightness in 1/256 steps (256 = unchanged) for one surface size and strength.
pub(in crate::render) struct FinishLut {
    width: u32,
    height: u32,
    strength_bits: u32,
    factors: Vec<u16>,
}

impl FinishLut {
    fn build(width: u32, height: u32, strength: f32) -> Self {
        let (w, h) = (width as usize, height as usize);
        let mut factors = Vec::with_capacity(w * h);
        for y in 0..h {
            let ny = (y as f32 + 0.5) / h as f32 * 2.0 - 1.0;
            for x in 0..w {
                let nx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
                let d = (nx * nx + ny * ny).sqrt() / std::f32::consts::SQRT_2;
                factors.push((vignette_factor(strength, d) * 256.0).round() as u16);
            }
        }
        Self {
            width,
            height,
            strength_bits: strength.to_bits(),
            factors,
        }
    }

    fn matches(&self, width: u32, height: u32, strength: f32) -> bool {
        self.width == width && self.height == height && self.strength_bits == strength.to_bits()
    }
}

/// Applies the vignette `factors` (one per pixel, when present) and grain of `amplitude` to every
/// row of `pixels`, with one loop per combination so the per-pixel work has no branches. Compiled for
/// AVX2 when the CPU has it (integer arithmetic only, so the result is the same everywhere).
fn finish_rows(
    pixels: &mut [u8],
    width: usize,
    factors: Option<&[u16]>,
    seed: u32,
    amplitude: i32,
) {
    #[cfg(target_arch = "x86_64")]
    if crate::render::paint::kernels::use_avx2() {
        #[target_feature(enable = "avx2")]
        fn avx2(
            pixels: &mut [u8],
            width: usize,
            factors: Option<&[u16]>,
            seed: u32,
            amplitude: i32,
        ) {
            finish_rows_impl(pixels, width, factors, seed, amplitude);
        }
        // SAFETY: the CPU supports AVX2, checked just above.
        return unsafe { avx2(pixels, width, factors, seed, amplitude) };
    }
    finish_rows_impl(pixels, width, factors, seed, amplitude);
}

#[inline(always)]
fn finish_rows_impl(
    pixels: &mut [u8],
    width: usize,
    factors: Option<&[u16]>,
    seed: u32,
    amplitude: i32,
) {
    let rows = pixels.chunks_exact_mut(width * 4).enumerate();
    match (factors, amplitude > 0) {
        (Some(table), true) => rows.for_each(|(y, row)| {
            finish_row::<true>(row, &table[y * width..(y + 1) * width], y, seed, amplitude)
        }),
        (Some(table), false) => rows.for_each(|(y, row)| {
            finish_row::<false>(row, &table[y * width..(y + 1) * width], y, seed, 0)
        }),
        (None, _) => rows.for_each(|(y, row)| grain_row(row, y, seed, amplitude)),
    }
}

/// Noise of pixel `(x, y)` in 1/256 of a level, for `amplitude` > 0.
#[inline(always)]
fn noise(x: usize, y: usize, seed: u32, amplitude: i32) -> i32 {
    let bits = ((hash(x as u32, y as u32, seed) >> 8) & 0xFFFF) as i32;
    ((bits * 2 - 65535) * amplitude) / 65535
}

#[inline(always)]
fn shade(pixel: &mut [u8; 4], factor: i32, noise: i32) {
    for channel in &mut pixel[..3] {
        let value = (i32::from(*channel) * factor + noise + 128) >> 8;
        *channel = value.clamp(0, 255) as u8;
    }
}

/// Vignette (and, with `GRAIN`, grain) over one row of pixels with its row of factors.
#[inline(always)]
fn finish_row<const GRAIN: bool>(
    row: &mut [u8],
    factors: &[u16],
    y: usize,
    seed: u32,
    amplitude: i32,
) {
    for (x, (pixel, &factor)) in row
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(factors)
        .enumerate()
    {
        let noise = if GRAIN {
            noise(x, y, seed, amplitude)
        } else {
            0
        };
        shade(pixel, i32::from(factor), noise);
    }
}

/// Grain alone over one row of pixels.
#[inline(always)]
fn grain_row(row: &mut [u8], y: usize, seed: u32, amplitude: i32) {
    for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        shade(pixel, 256, noise(x, y, seed, amplitude));
    }
}

impl Surface {
    /// Darkens the corners by `vignette` (0..=1) and adds monochrome noise of amplitude `grain` (0..=1),
    /// reseeded by `seed` each frame. Marks the whole surface as painted so the next frame resets it.
    pub(in crate::render) fn apply_finish(&mut self, vignette: f32, grain: f32, seed: u32) {
        if vignette <= 0.0 && grain <= 0.0 {
            return;
        }
        let (width, height) = (self.width as usize, self.height as usize);
        if vignette > 0.0
            && !self
                .finish_lut
                .as_ref()
                .is_some_and(|lut| lut.matches(self.width, self.height, vignette))
        {
            self.finish_lut = Some(FinishLut::build(self.width, self.height, vignette));
        }
        let factors = if vignette > 0.0 {
            self.finish_lut.as_ref().map(|lut| lut.factors.as_slice())
        } else {
            None
        };
        // Noise amplitude in 1/256 of a level; the largest product below stays within i32.
        let amplitude = (grain * 64.0 * 256.0) as i32;
        finish_rows(&mut self.pixels, width, factors, seed, amplitude);
        self.mark_painted(PixelRect {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Color;

    fn gray(width: u32, height: u32, level: u8) -> Surface {
        Surface::filled(
            width,
            height,
            1.0,
            Color {
                r: level,
                g: level,
                b: level,
                a: 255,
            },
        )
    }

    fn pixel(surface: &Surface, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * surface.width + x) * 4) as usize;
        surface.pixels[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn vignette_only_darkens_towards_the_corners() {
        assert_eq!(vignette_factor(0.5, 0.0), 1.0);
        assert_eq!(vignette_factor(0.5, 0.35), 1.0);
        assert!((vignette_factor(0.5, 1.0) - 0.5).abs() < 1e-6);
        assert!(vignette_factor(0.5, 0.7) < 1.0);
    }

    #[test]
    fn noise_is_deterministic_and_varies_with_seed() {
        assert_eq!(hash(3, 4, 5), hash(3, 4, 5));
        assert_ne!(hash(3, 4, 5), hash(3, 4, 6));
        assert_ne!(hash(3, 4, 5), hash(4, 3, 5));
    }

    #[test]
    fn lut_is_neutral_at_the_center_and_darkest_in_the_corners() {
        let lut = FinishLut::build(64, 36, 0.5);
        assert_eq!(lut.factors.len(), 64 * 36);
        assert_eq!(lut.factors[18 * 64 + 32], 256);
        let corner = lut.factors[0];
        assert!((126..=134).contains(&corner), "corner factor {corner}");
        assert!(lut.factors.iter().all(|&f| f <= 256));
    }

    #[test]
    fn vignette_darkens_corners_and_keeps_the_center() {
        let mut surface = gray(64, 36, 200);
        surface.apply_finish(0.5, 0.0, 0);
        assert_eq!(pixel(&surface, 32, 18)[0], 200);
        let corner = pixel(&surface, 0, 0);
        assert!((98..=102).contains(&corner[0]), "corner {}", corner[0]);
        assert_eq!(corner[3], 255);
        let edge = pixel(&surface, 0, 18)[0];
        assert!(corner[0] < edge && edge <= 200);
    }

    #[test]
    fn nothing_changes_without_vignette_or_grain() {
        let mut surface = gray(16, 9, 90);
        surface.apply_finish(0.0, 0.0, 3);
        assert!(surface
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [90, 90, 90, 255]));
        assert!(surface.painted.is_none());
    }

    #[test]
    fn grain_is_deterministic_bounded_and_reseeded() {
        let mut a = gray(48, 27, 128);
        let mut b = gray(48, 27, 128);
        let mut c = gray(48, 27, 128);
        a.apply_finish(0.0, 0.1, 7);
        b.apply_finish(0.0, 0.1, 7);
        c.apply_finish(0.0, 0.1, 8);
        assert_eq!(a.pixels, b.pixels);
        assert_ne!(a.pixels, c.pixels);
        // 0.1 of 64 levels is 6.4; rounding may add one more.
        let widest = a
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| (i32::from(p[0]) - 128).abs())
            .max()
            .unwrap();
        assert!((1..=7).contains(&widest), "widest deviation {widest}");
        // Monochrome: all three channels move together.
        assert!(a
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] == p[1] && p[1] == p[2]));
    }

    #[test]
    fn grain_is_roughly_zero_mean() {
        let mut surface = gray(64, 64, 128);
        surface.apply_finish(0.0, 0.2, 11);
        let total: i64 = surface
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| i64::from(p[0]) - 128)
            .sum();
        let mean = total as f64 / (64.0 * 64.0);
        assert!(mean.abs() < 0.5, "mean {mean}");
    }

    #[test]
    fn the_table_is_rebuilt_when_the_strength_changes_and_the_whole_frame_is_marked() {
        let mut surface = gray(32, 18, 255);
        surface.apply_finish(0.5, 0.0, 0);
        assert_eq!(
            surface.finish_lut.as_ref().unwrap().strength_bits,
            0.5f32.to_bits()
        );
        surface.apply_finish(0.25, 0.0, 0);
        assert_eq!(
            surface.finish_lut.as_ref().unwrap().strength_bits,
            0.25f32.to_bits()
        );
        let painted = surface.painted.unwrap();
        assert_eq!(
            (painted.x0, painted.y0, painted.x1, painted.y1),
            (0, 0, 32, 18)
        );
    }
}
