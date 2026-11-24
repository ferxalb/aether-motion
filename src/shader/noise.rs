//! Deterministic procedural noise for shaders: an integer lattice hash, gradient noise in two
//! and three dimensions, fractal sums of octaves (`fbm`), cellular distance (`voronoi`), and
//! per-cell random values (`hash`).
//!
//! Lattice coordinates come from `math::floor` and an `i32` conversion, the hash uses only
//! wrapping integer multiplication, shifts, and xors, and the interpolation uses IEEE `f32`
//! arithmetic, so every function is a pure function of its inputs on every machine. All
//! functions are branch-free and `#[inline(always)]` so the chunk loops in `vm.rs` vectorize.
//! `noise` and `fbm` return values in `[0, 1]` (0.5 on average); `voronoi` returns the distance
//! to the nearest feature point in cell units; `hash` returns `[0, 1)`.

use super::math::floor;

/// Largest lattice coordinate magnitude read; larger inputs wrap the pattern but stay defined.
const LATTICE: f32 = 1_000_000.0;

#[inline(always)]
fn cell(x: f32) -> (i32, f32) {
    #[allow(clippy::manual_clamp)]
    let x = x.max(-LATTICE).min(LATTICE);
    let base = floor(x);
    (base as i32, x - base)
}

/// Avalanche hash of up to three lattice coordinates (a variant of the `lowbias32` finalizer).
#[inline(always)]
fn hash3(x: i32, y: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

/// A value in `[-1, 1)` from ten bits of `h` starting at `shift`.
#[inline(always)]
fn signed(h: u32, shift: u32) -> f32 {
    ((h >> shift) & 1023) as f32 * (1.0 / 512.0) - 1.0
}

/// A value in `[0, 1)` from the top 24 bits of `h`.
#[inline(always)]
fn unit(h: u32) -> f32 {
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Quintic fade `6t^5 - 15t^4 + 10t^3`, flat in value, slope, and curvature at both ends.
#[inline(always)]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline(always)]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline(always)]
pub(super) fn to_unit(n: f32) -> f32 {
    // `max` and `min` rather than `clamp`, which would keep a NaN.
    #[allow(clippy::manual_clamp)]
    let unit = (0.5 + 0.5 * n).max(0.0).min(1.0);
    unit
}

/// Signed two-dimensional gradient noise, roughly in `[-1, 1]`.
#[inline(always)]
pub(super) fn gradient2(x: f32, y: f32) -> f32 {
    let (ix, fx) = cell(x);
    let (iy, fy) = cell(y);
    let corner = |dx: i32, dy: i32| {
        let h = hash3(ix.wrapping_add(dx), iy.wrapping_add(dy), 0);
        signed(h, 0) * (fx - dx as f32) + signed(h, 10) * (fy - dy as f32)
    };
    let u = fade(fx);
    let v = fade(fy);
    let bottom = lerp(corner(0, 0), corner(1, 0), u);
    let top = lerp(corner(0, 1), corner(1, 1), u);
    lerp(bottom, top, v)
}

/// Signed three-dimensional gradient noise, roughly in `[-1, 1]`.
#[inline(always)]
pub(super) fn gradient3(x: f32, y: f32, z: f32) -> f32 {
    let (ix, fx) = cell(x);
    let (iy, fy) = cell(y);
    let (iz, fz) = cell(z);
    let corner = |dx: i32, dy: i32, dz: i32| {
        let h = hash3(
            ix.wrapping_add(dx),
            iy.wrapping_add(dy),
            iz.wrapping_add(dz),
        );
        signed(h, 0) * (fx - dx as f32)
            + signed(h, 10) * (fy - dy as f32)
            + signed(h, 20) * (fz - dz as f32)
    };
    let u = fade(fx);
    let v = fade(fy);
    let w = fade(fz);
    let near = lerp(
        lerp(corner(0, 0, 0), corner(1, 0, 0), u),
        lerp(corner(0, 1, 0), corner(1, 1, 0), u),
        v,
    );
    let far = lerp(
        lerp(corner(0, 0, 1), corner(1, 0, 1), u),
        lerp(corner(0, 1, 1), corner(1, 1, 1), u),
        v,
    );
    lerp(near, far, w)
}

/// Moves fractal coordinates to the next octave: twice the frequency, turned by about 37
/// degrees, and shifted.
#[inline(always)]
pub(super) fn octave_step(x: f32, y: f32) -> (f32, f32) {
    (1.6 * x + 1.2 * y + 17.13, -1.2 * x + 1.6 * y + 31.71)
}

/// Two-dimensional gradient noise in `[0, 1]`.
#[inline(always)]
pub fn noise2(x: f32, y: f32) -> f32 {
    to_unit(gradient2(x, y))
}

/// Three-dimensional gradient noise in `[0, 1]`; the third coordinate is usually time.
#[inline(always)]
pub fn noise3(x: f32, y: f32, z: f32) -> f32 {
    to_unit(gradient3(x, y, z))
}

/// Fractal sum of `octaves` layers of 2D noise, each at twice the frequency and half the
/// amplitude of the last, turned by about 37 degrees and shifted between layers to hide the
/// lattice; normalized to `[0, 1]`.
#[inline(always)]
pub fn fbm2(x: f32, y: f32, octaves: u8) -> f32 {
    let (mut x, mut y) = (x, y);
    let mut sum = 0.0;
    let mut amplitude = 0.5;
    let mut total = 0.0;
    for _ in 0..octaves {
        sum += amplitude * gradient2(x, y);
        total += amplitude;
        amplitude *= 0.5;
        (x, y) = octave_step(x, y);
    }
    to_unit(sum / total)
}

/// Fractal sum of `octaves` layers of 3D noise; the third coordinate keeps its frequency ratio
/// so time-driven evolution looks equally lively at every octave.
#[inline(always)]
pub fn fbm3(x: f32, y: f32, z: f32, octaves: u8) -> f32 {
    let (mut x, mut y, mut z) = (x, y, z);
    let mut sum = 0.0;
    let mut amplitude = 0.5;
    let mut total = 0.0;
    for _ in 0..octaves {
        sum += amplitude * gradient3(x, y, z);
        total += amplitude;
        amplitude *= 0.5;
        (x, y) = octave_step(x, y);
        z = z * 2.0 + 7.77;
    }
    to_unit(sum / total)
}

/// Distance from `(x, y)` to the nearest feature point of a jittered unit grid (one point per
/// cell, searched in the 3x3 neighborhood): 0 at a point, about 0.5 at cell borders.
#[inline(always)]
pub fn voronoi2(x: f32, y: f32) -> f32 {
    let (ix, fx) = cell(x);
    let (iy, fy) = cell(y);
    let mut best = 8.0_f32;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let h = hash3(ix.wrapping_add(dx), iy.wrapping_add(dy), 0x5bd1);
            let px = dx as f32 + unit(h) - fx;
            let py = dy as f32 + ((h & 0xffff) as f32 * (1.0 / 65_536.0)) - fy;
            best = best.min(px * px + py * py);
        }
    }
    best.sqrt()
}

/// A random value in `[0, 1)` for the integer cell containing `(x, y)`.
#[inline(always)]
pub fn hash2(x: f32, y: f32) -> f32 {
    let (ix, _) = cell(x);
    let (iy, _) = cell(y);
    unit(hash3(ix, iy, 0x2c1b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_continuous_and_bounded() {
        let mut previous = noise2(0.0, 0.37);
        let mut low = f32::MAX;
        let mut high = f32::MIN;
        for i in 1..20_000 {
            let x = i as f32 * 0.001;
            let value = noise2(x, 0.37);
            assert!((value - previous).abs() < 0.01, "jump at {x}");
            low = low.min(value);
            high = high.max(value);
            previous = value;
        }
        assert!(low < 0.3 && high > 0.7, "range {low}..{high}");
        for i in 0..2000 {
            let p = i as f32 * 0.173;
            for value in [
                noise3(p, -p * 0.7, p * 0.3),
                fbm2(p, p * 1.3, 6),
                fbm3(p, 0.5, p, 4),
            ] {
                assert!((0.0..=1.0).contains(&value));
            }
        }
    }

    #[test]
    fn noise_is_zero_crossing_at_lattice_points() {
        // Gradient noise is 0 (0.5 after mapping) on every integer point.
        for (x, y) in [(0.0, 0.0), (3.0, -7.0), (-120.0, 55.0)] {
            assert_eq!(noise2(x, y), 0.5);
        }
    }

    #[test]
    fn voronoi_and_hash_stay_in_range() {
        for i in 0..5000 {
            let p = i as f32 * 0.0731 - 150.0;
            let d = voronoi2(p, p * 0.61);
            assert!((0.0..1.5).contains(&d));
            let h = hash2(p, -p);
            assert!((0.0..1.0).contains(&h));
        }
        assert_eq!(hash2(2.2, 3.9), hash2(2.8, 3.1));
    }
}
