//! Frame-wide finishing applied after everything is drawn: vignette and film grain.
//!
//! Both are pure functions of pixel position (and, for grain, of the frame time), so output stays
//! deterministic and independent of render order. The vignette is a per-pixel brightness table built
//! once per surface size and strength, so a frame costs one integer multiply per channel; the grain is
//! an integer hash per pixel. All arithmetic is fixed point (1/256 steps), in branch-free row loops
//! that are also compiled for AVX2 and chosen at run time (same results on every CPU). Rows are
//! processed in blocks of eight pixels (eight hashes, then one uniform operation on all 32 bytes),
//! which vectorizes; a test checks the blocks against the per-pixel loop they replaced.

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

/// Plain loops (no closures), so the row code inlines into the AVX2 copy of [`finish_rows`]: a
/// closure would be compiled as a separate function without the target feature.
#[inline(always)]
fn finish_rows_impl(
    pixels: &mut [u8],
    width: usize,
    factors: Option<&[u16]>,
    seed: u32,
    amplitude: i32,
) {
    for (y, row) in pixels.chunks_exact_mut(width * 4).enumerate() {
        match factors {
            Some(table) if amplitude > 0 => {
                finish_row::<true>(row, &table[y * width..(y + 1) * width], y, seed, amplitude)
            }
            Some(table) => finish_row::<false>(row, &table[y * width..(y + 1) * width], y, seed, 0),
            None => grain_row(row, y, seed, amplitude),
        }
    }
}

/// Noise of pixel `(x, y)` in 1/256 of a level, for `amplitude` > 0.
#[inline(always)]
fn noise(x: usize, y: usize, seed: u32, amplitude: i32) -> i32 {
    let bits = ((hash(x as u32, y as u32, seed) >> 8) & 0xFFFF) as i32;
    div_65535((bits * 2 - 65535) * amplitude)
}

/// `v / 65535`, truncated toward zero like Rust's `/`, in 32-bit operations so rows of noise
/// vectorize. With `x = 65535 q + r` (`0 <= r < 65535`) and `q < 2^16`, `x >> 16` is `q` or `q - 1`
/// (when `r < q`), so `(x + 1 + (x >> 16)) >> 16 = q + (r + 1 - [r < q]) / 2^16 = q` exactly; every
/// magnitude here is at most `2^31`.
#[inline(always)]
fn div_65535(v: i32) -> i32 {
    let x = v.unsigned_abs();
    let q = ((x + 1 + (x >> 16)) >> 16) as i32;
    // Branch-free sign: `(q ^ s) - s` is `q` for `s = 0` and `-q` for `s = -1`.
    let sign = v >> 31;
    (q ^ sign) - sign
}

/// [`noise`] of the `BLOCK` pixels from `(x0, y)`, one simple pass per hash step so the eight hashes
/// are computed side by side (the bits equal [`hash`]'s: XOR is associative).
#[inline(always)]
fn noise_block(x0: usize, y: usize, seed: u32, amplitude: i32, out: &mut [i32; BLOCK]) {
    let base = (y as u32).wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    let mut h = [0_u32; BLOCK];
    for (i, h) in h.iter_mut().enumerate() {
        *h = ((x0 + i) as u32).wrapping_mul(0x9E37_79B1) ^ base;
    }
    for h in &mut h {
        *h ^= *h >> 15;
    }
    for h in &mut h {
        *h = h.wrapping_mul(0x2C1B_3C6D);
    }
    for h in &mut h {
        *h ^= *h >> 12;
    }
    for h in &mut h {
        *h = h.wrapping_mul(0x297A_2D39);
    }
    for h in &mut h {
        *h ^= *h >> 15;
    }
    for (out, &h) in out.iter_mut().zip(&h) {
        let bits = ((h >> 8) & 0xFFFF) as i32;
        *out = div_65535((bits * 2 - 65535) * amplitude);
    }
}

#[inline(always)]
fn shade(pixel: &mut [u8; 4], factor: i32, noise: i32) {
    for channel in &mut pixel[..3] {
        let value = (i32::from(*channel) * factor + noise + 128) >> 8;
        *channel = value.clamp(0, 255) as u8;
    }
}

/// Pixels per block of the row loops: the noise of a block is computed first (eight hashes side
/// by side), then each pixel is shaded as one `u32` lane (red, green, and blue extracted, shaded
/// like [`shade`], and packed back with the alpha byte unchanged), which vectorizes.
const BLOCK: usize = 8;

/// Shades one block of `BLOCK` RGBA pixels by the per-pixel `factors` (1/256 steps) and `noise`
/// (1/256 of a level); the same result as [`shade`] on each pixel.
#[inline(always)]
fn shade_block(bytes: &mut [u8], factors: &[i32; BLOCK], noise: &[i32; BLOCK]) {
    let mut lanes = [0_u32; BLOCK];
    for (lane, pixel) in lanes.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *lane = u32::from_le_bytes(*pixel);
    }
    for ((lane, &factor), &noise) in lanes.iter_mut().zip(factors).zip(noise) {
        let channel = |shift: u32| {
            let value = ((*lane >> shift) & 0xFF) as i32;
            (((value * factor + noise + 128) >> 8).clamp(0, 255) as u32) << shift
        };
        *lane = (*lane & 0xFF00_0000) | channel(0) | channel(8) | channel(16);
    }
    for (pixel, lane) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(lanes) {
        *pixel = lane.to_le_bytes();
    }
}

/// Vignette (and, with `GRAIN`, grain) over one row of pixels with its row of factors; the same
/// result as [`shade`] with [`noise`] per pixel, in blocks the compiler can vectorize.
#[inline(always)]
fn finish_row<const GRAIN: bool>(
    row: &mut [u8],
    factors: &[u16],
    y: usize,
    seed: u32,
    amplitude: i32,
) {
    let mut blocks = row.chunks_exact_mut(BLOCK * 4);
    let mut table = factors.chunks_exact(BLOCK);
    let mut x0 = 0;
    for (bytes, table) in (&mut blocks).zip(&mut table) {
        let mut f = [0_i32; BLOCK];
        for (f, &t) in f.iter_mut().zip(table) {
            *f = i32::from(t);
        }
        let mut n = [0_i32; BLOCK];
        if GRAIN {
            noise_block(x0, y, seed, amplitude, &mut n);
        }
        shade_block(bytes, &f, &n);
        x0 += BLOCK;
    }
    for (i, (pixel, &factor)) in blocks
        .into_remainder()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(table.remainder())
        .enumerate()
    {
        let noise = if GRAIN {
            noise(x0 + i, y, seed, amplitude)
        } else {
            0
        };
        shade(pixel, i32::from(factor), noise);
    }
}

/// Grain alone over one row of pixels.
#[inline(always)]
fn grain_row(row: &mut [u8], y: usize, seed: u32, amplitude: i32) {
    let mut blocks = row.chunks_exact_mut(BLOCK * 4);
    let mut x0 = 0;
    for bytes in &mut blocks {
        let mut n = [0_i32; BLOCK];
        noise_block(x0, y, seed, amplitude, &mut n);
        shade_block(bytes, &[256; BLOCK], &n);
        x0 += BLOCK;
    }
    for (i, pixel) in blocks
        .into_remainder()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .enumerate()
    {
        shade(pixel, 256, noise(x0 + i, y, seed, amplitude));
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

    /// The per-pixel loop the blocked rows replaced.
    fn reference(
        pixels: &mut [u8],
        width: usize,
        factors: Option<&[u16]>,
        seed: u32,
        amplitude: i32,
    ) {
        for (y, row) in pixels.chunks_exact_mut(width * 4).enumerate() {
            for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let factor = factors.map_or(256, |t| i32::from(t[y * width + x]));
                // The noise as it was computed before the multiply-shift division.
                let bits = ((hash(x as u32, y as u32, seed) >> 8) & 0xFFFF) as i32;
                let n = if amplitude > 0 {
                    ((bits * 2 - 65535) * amplitude) / 65535
                } else {
                    0
                };
                shade(pixel, factor, n);
            }
        }
    }

    #[test]
    fn the_noise_division_matches_integer_division() {
        let amplitudes = [1, 7, 255, 4096, 16_384];
        for amplitude in amplitudes {
            for bits in 0..=65_535 {
                let v = (bits * 2 - 65_535) * amplitude;
                assert_eq!(div_65535(v), v / 65_535, "{v}");
            }
        }
        for v in [i32::MAX, i32::MIN + 1, 65_535, -65_535, 65_534, -65_534, 0] {
            assert_eq!(div_65535(v), v / 65_535, "{v}");
        }
        // Every multiple of 65535 and its neighbors, up to the largest magnitude.
        for q in 0..=32_768_i64 {
            for r in [-1_i64, 0, 1, 65_534] {
                let v = (q * 65_535 + r).clamp(0, i64::from(i32::MAX)) as i32;
                assert_eq!(div_65535(v), v / 65_535, "{v}");
                assert_eq!(div_65535(-v), -v / 65_535, "{v}");
            }
        }
    }

    #[test]
    fn block_noise_matches_the_per_pixel_noise() {
        for (x0, y, seed, amplitude) in
            [(0, 0, 0, 1), (8, 3, 77, 4000), (4088, 2159, 9_999, 16_384)]
        {
            let mut block = [0; BLOCK];
            noise_block(x0, y, seed, amplitude, &mut block);
            for (i, &n) in block.iter().enumerate() {
                assert_eq!(n, noise(x0 + i, y, seed, amplitude));
            }
        }
    }

    #[test]
    fn blocked_rows_match_the_per_pixel_loop() {
        // Odd widths leave a remainder after the blocks; levels cover clamping at both ends.
        for (width, height) in [(37, 5), (64, 3), (3, 4), (8, 2)] {
            let lut = FinishLut::build(width as u32, height as u32, 0.8);
            let source: Vec<u8> = (0..width * height * 4)
                .map(|i| (i as u32).wrapping_mul(2_654_435_761).rotate_left(7) as u8)
                .collect();
            for (factors, amplitude) in [
                (Some(lut.factors.as_slice()), 4000),
                (Some(lut.factors.as_slice()), 0),
                (None, 16_384),
            ] {
                let mut fast = source.clone();
                let mut slow = source.clone();
                finish_rows(&mut fast, width, factors, 99, amplitude);
                reference(&mut slow, width, factors, 99, amplitude);
                assert_eq!(fast, slow, "{width}x{height} grain {amplitude}");
            }
        }
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
