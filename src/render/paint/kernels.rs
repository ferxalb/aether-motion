//! Branch-free per-row pixel kernels with runtime CPU dispatch.
//!
//! Each kernel is written once, as plain scalar Rust whose per-pixel work has no branches (selects
//! only), so the compiler can vectorize it. On x86-64 the same source is also compiled with AVX2
//! enabled and chosen at run time when the CPU has it; elsewhere (and on older x86-64 CPUs) the
//! baseline build runs. Neither build enables FMA or reassociates arithmetic, and IEEE division,
//! square root, and conversions are exact in every width, so both produce bit-identical pixels: the
//! output never depends on the machine. Tests compare every kernel with the scalar reference it
//! replaces.

/// Whether the AVX2 builds of the kernels run: the CPU has AVX2 and the `AETHER_SIMD` environment
/// variable is not `off`. Read once per process. Both builds give identical pixels, so the switch
/// exists only to compare their speed and to verify that identity end to end.
#[cfg(target_arch = "x86_64")]
pub(in crate::render) fn use_avx2() -> bool {
    static AVX2: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVX2.get_or_init(|| {
        std::arch::is_x86_feature_detected!("avx2")
            && std::env::var("AETHER_SIMD").map_or(true, |value| value != "off")
    })
}

/// `round_to_u8` for finite values, in a form that vectorizes: clamp with `max`/`min` (a NaN would
/// become 0), truncate, and add one when the exact fractional part reaches one half.
#[inline(always)]
// `max`/`min` rather than `clamp`: they map NaN to a bound, which the unchecked conversion needs.
#[allow(clippy::manual_clamp)]
pub(in crate::render) fn round_clamped(value: f32) -> u8 {
    let value = value.max(0.0).min(255.0);
    // SAFETY: `value` is in 0..=255 (never NaN after `max`), so the conversion is exact.
    let whole = unsafe { value.to_int_unchecked::<i32>() };
    let up = value - whole as f32 >= 0.5 && whole < 255;
    (whole + i32::from(up)) as u8
}

/// A gradient with exactly two stops, its stop colors premultiplied (as `Sampler` holds them).
#[derive(Clone, Copy, Debug)]
pub(in crate::render) struct TwoStops {
    pub(in crate::render) positions: [f32; 2],
    pub(in crate::render) colors: [[f32; 4]; 2],
}

/// One row of a directly drawn gradient: the box's horizontal edges on this row, its vertical
/// coverage, the element opacity, and the row's dither thresholds by `x & 3`.
#[derive(Clone, Copy, Debug)]
pub(in crate::render) struct GradientRow {
    pub(in crate::render) left: f32,
    pub(in crate::render) right: f32,
    pub(in crate::render) vertical: f32,
    pub(in crate::render) alpha: f32,
    pub(in crate::render) dither: [f32; 4],
}

/// Blends a two-stop gradient over `dest` (pixel `first` onwards of one row) from the gradient
/// parameter `ts` of each pixel, exactly as sampling each pixel and blending it one by one would.
pub(in crate::render) fn two_stop_row(
    dest: &mut [[u8; 4]],
    ts: &[f32],
    first: usize,
    row: &GradientRow,
    stops: &TwoStops,
) {
    #[cfg(target_arch = "x86_64")]
    if use_avx2() {
        // SAFETY: the CPU supports AVX2, checked just above.
        return unsafe { two_stop_row_avx2(dest, ts, first, row, stops) };
    }
    two_stop_row_impl(dest, ts, first, row, stops);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn two_stop_row_avx2(
    dest: &mut [[u8; 4]],
    ts: &[f32],
    first: usize,
    row: &GradientRow,
    stops: &TwoStops,
) {
    two_stop_row_impl(dest, ts, first, row, stops);
}

#[inline(always)]
fn two_stop_row_impl(
    dest: &mut [[u8; 4]],
    ts: &[f32],
    first: usize,
    row: &GradientRow,
    stops: &TwoStops,
) {
    let [p0, p1] = stops.positions;
    let [c0, c1] = stops.colors;
    let span = p1 - p0;
    for (i, (pixel, &t)) in dest.iter_mut().zip(ts).enumerate() {
        let x = (first + i) as f32;
        let coverage = (row.right.min(x + 1.0) - row.left.max(x)).clamp(0.0, 1.0) * row.vertical;
        // `Sampler::sample`: the first stop at or before it, the last at or after, else a mix.
        let f = if span <= 0.0 { 1.0 } else { (t - p0) / span };
        let (low, high) = (t <= p0, t >= p1);
        let sample: [f32; 4] = std::array::from_fn(|c| {
            let mixed = c0[c] + (c1[c] - c0[c]) * f;
            if low {
                c0[c]
            } else if high {
                c1[c]
            } else {
                mixed
            }
        });
        let a = (row.alpha * coverage).clamp(0.0, 1.0);
        let paint = a > 0.0 && sample[3] > 0.0;
        let keep = 1.0 - sample[3] * a;
        let d = row.dither[(first + i) & 3];
        for c in 0..3 {
            let blended = sample[c] * a + f32::from(pixel[c]) * keep + d;
            let value = round_clamped(blended);
            pixel[c] = if paint { value } else { pixel[c] };
        }
        pixel[3] = if paint { 255 } else { pixel[3] };
    }
}

/// Radial gradient parameters `hypot((x + 0.5 - center) / radius, v)` of the pixels `columns`
/// of one row, where `vv` is the row's `v * v` in `f64` (see `float_ops::hypot`), into `out`.
pub(in crate::render) fn radial_row(
    out: &mut Vec<f32>,
    columns: std::ops::Range<usize>,
    center: f32,
    radius: f32,
    vv: f64,
) {
    out.clear();
    out.resize(columns.len(), 0.0);
    #[cfg(target_arch = "x86_64")]
    if use_avx2() {
        // SAFETY: the CPU supports AVX2, checked just above.
        return unsafe { radial_row_avx2(out, columns.start, center, radius, vv) };
    }
    radial_row_impl(out, columns.start, center, radius, vv);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn radial_row_avx2(out: &mut [f32], first: usize, center: f32, radius: f32, vv: f64) {
    radial_row_impl(out, first, center, radius, vv);
}

#[inline(always)]
fn radial_row_impl(out: &mut [f32], first: usize, center: f32, radius: f32, vv: f64) {
    for (i, t) in out.iter_mut().enumerate() {
        let u = ((first + i) as f32 + 0.5 - center) / radius;
        let wide = f64::from(u);
        let value = (wide * wide + vv).sqrt() as f32;
        *t = if u.is_infinite() {
            f32::INFINITY
        } else {
            value
        };
    }
}

/// Generates a public kernel `$name` that runs `$body` (an `#[inline(always)]` function with the
/// same arguments) compiled for AVX2 when the CPU has it, and for the baseline target otherwise.
macro_rules! dispatched {
    ($(#[$doc:meta])* $name:ident => $body:ident ($($arg:ident: $ty:ty),* $(,)?)) => {
        $(#[$doc])*
        pub(in crate::render) fn $name($($arg: $ty),*) {
            #[cfg(target_arch = "x86_64")]
            if $crate::render::paint::kernels::use_avx2() {
                #[target_feature(enable = "avx2")]
                fn avx2($($arg: $ty),*) {
                    $body($($arg),*)
                }
                // SAFETY: the CPU supports AVX2, checked just above.
                return unsafe { avx2($($arg),*) };
            }
            $body($($arg),*)
        }
    };
}

/// Fractional bits of the horizontally filtered rows of the ladder resampler (`svg_levels.rs`),
/// whose weights have 14 fractional bits.
pub(in crate::render) const RESAMPLE_MIDDLE_BITS: u32 = 6;
const RESAMPLE_H_SHIFT: u32 = 14 - RESAMPLE_MIDDLE_BITS;

dispatched!(
    /// Premultiplies straight RGBA `src` into `out` (`c * a / 255`, rounded to nearest).
    premultiply_row => premultiply_row_impl(out: &mut [u8], src: &[u8])
);

#[inline(always)]
fn premultiply_row_impl(out: &mut [u8], src: &[u8]) {
    for (p, s) in out
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(src.as_chunks::<4>().0)
    {
        let a = u32::from(s[3]);
        for c in 0..3 {
            p[c] = ((u32::from(s[c]) * a + 127) / 255) as u8;
        }
        p[3] = s[3];
    }
}

dispatched!(
    /// One horizontal resampling pass: output pixel `x` of `out` (4 channels) is the sum over
    /// `taps` premultiplied pixels of `row` from `starts[x]`, weighted by `weights[x * taps..]`
    /// (14 fractional bits), rounded to `RESAMPLE_MIDDLE_BITS` fractional bits.
    filter_columns => filter_columns_impl(
        out: &mut [i32],
        row: &[u8],
        starts: &[usize],
        weights: &[i32],
        taps: usize,
    )
);

#[inline(always)]
fn filter_columns_impl(out: &mut [i32], row: &[u8], starts: &[usize], weights: &[i32], taps: usize) {
    for ((o, &start), w) in out
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(starts)
        .zip(weights.chunks_exact(taps))
    {
        let pixels = &row[start * 4..(start + taps) * 4];
        let mut acc = [1_i32 << (RESAMPLE_H_SHIFT - 1); 4];
        for (pixel, &weight) in pixels.as_chunks::<4>().0.iter().zip(w) {
            for c in 0..4 {
                acc[c] += weight * i32::from(pixel[c]);
            }
        }
        for c in 0..4 {
            o[c] = acc[c] >> RESAMPLE_H_SHIFT;
        }
    }
}

dispatched!(
    /// Adds `weight * line` to `acc`, element by element (the vertical resampling pass).
    accumulate_row => accumulate_row_impl(acc: &mut [i32], line: &[i32], weight: i32)
);

#[inline(always)]
fn accumulate_row_impl(acc: &mut [i32], line: &[i32], weight: i32) {
    for (a, &v) in acc.iter_mut().zip(line) {
        *a += weight * v;
    }
}

/// Converts premultiplied sums with `shift` fractional bits to straight RGBA in `dest`: alpha is
/// clamped to 0..=255, each color to 0..=alpha (filter ringing cannot make an invalid pixel), and
/// colors are divided by alpha rounding to nearest. Transparent results leave `dest` unchanged.
pub(in crate::render) fn unpremultiply_row(dest: &mut [u8], acc: &[i32], shift: u32) {
    for (pixel, acc) in dest
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(acc.as_chunks::<4>().0)
    {
        let a = (acc[3] >> shift).clamp(0, 255) as u32;
        if a == 0 {
            continue;
        }
        let straight = |v: i32| {
            let c = (v >> shift).clamp(0, a as i32) as u32;
            if a == 255 {
                c as u8
            } else {
                ((c * 255 + a / 2) / a).min(255) as u8
            }
        };
        *pixel = [straight(acc[0]), straight(acc[1]), straight(acc[2]), a as u8];
    }
}

dispatched!(
    /// Blends shadow color `rgb` through coverage `mask` (x65535) at `k` opacity per mask unit;
    /// coverage below 0.002 leaves a pixel unchanged (see `shadow.rs`).
    shadow_mask => shadow_mask_impl(dest: &mut [[u8; 4]], mask: &[u16], rgb: [f32; 3], k: f32)
);

#[inline(always)]
fn shadow_mask_impl(dest: &mut [[u8; 4]], mask: &[u16], rgb: [f32; 3], k: f32) {
    for (pixel, &m) in dest.iter_mut().zip(mask) {
        shadow_pixel(pixel, m, rgb, k);
    }
}

/// One pixel of [`shadow_mask`], for callers that visit pixels one at a time.
#[inline(always)]
// `max`/`min` rather than `clamp`: they map NaN to a bound, which the unchecked conversion needs.
#[allow(clippy::manual_clamp)]
pub(in crate::render) fn shadow_pixel(pixel: &mut [u8; 4], m: u16, rgb: [f32; 3], k: f32) {
    let a = f32::from(m) * k;
    let paint = a > 0.002;
    for c in 0..3 {
        let value = rgb[c] * a + f32::from(pixel[c]) * (1.0 - a) + 0.5;
        // `as u8` of a value in 0..256, without the saturating conversion: identical.
        // SAFETY: clamped to 0..=255, so the conversion is exact and never NaN.
        let value = unsafe { value.max(0.0).min(255.0).to_int_unchecked::<i32>() } as u8;
        pixel[c] = if paint { value } else { pixel[c] };
    }
    pixel[3] = if paint { 255 } else { pixel[3] };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::paint::float_ops::round_to_u8;

    #[test]
    fn round_clamped_matches_round_to_u8() {
        for i in -2000..=60000 {
            let v = i as f32 * 0.005 - 4.0;
            assert_eq!(round_clamped(v), round_to_u8(v), "{v}");
        }
        for k in -2..=257 {
            let half = k as f32 + 0.5;
            for v in [
                half,
                f32::from_bits(half.to_bits() - 1),
                f32::from_bits(half.to_bits() + 1),
            ] {
                assert_eq!(round_clamped(v), round_to_u8(v), "{v}");
            }
        }
        for v in [
            0.499_999_97_f32,
            -0.0,
            300.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            assert_eq!(round_clamped(v), round_to_u8(v), "{v}");
        }
    }

    #[test]
    fn shadow_mask_matches_the_branching_blend() {
        let mask: Vec<u16> = (0..=65535u32).step_by(97).map(|m| m as u16).collect();
        for (rgb, k) in [
            ([0.0, 0.0, 0.0], 0.8 / 65535.0),
            ([255.0, 128.0, 7.0], 1.0 / 65535.0),
        ] {
            let start: Vec<[u8; 4]> = (0..mask.len())
                .map(|i| [(i * 13) as u8, (i * 7) as u8, i as u8, 90])
                .collect();
            let mut expected = start.clone();
            for (pixel, &m) in expected.iter_mut().zip(&mask) {
                let a = f32::from(m) * k;
                if a > 0.002 {
                    for c in 0..3 {
                        pixel[c] = (rgb[c] * a + f32::from(pixel[c]) * (1.0 - a) + 0.5) as u8;
                    }
                    pixel[3] = 255;
                }
            }
            let mut actual = start.clone();
            shadow_mask(&mut actual, &mask, rgb, k);
            assert_eq!(actual, expected);
        }
    }

    /// The per-pixel loop this kernel replaced, verbatim.
    fn reference(
        dest: &mut [[u8; 4]],
        ts: &[f32],
        first: usize,
        row: &GradientRow,
        stops: &TwoStops,
    ) {
        let [p0, p1] = stops.positions;
        for (i, (pixel, &t)) in dest.iter_mut().zip(ts).enumerate() {
            let x = first + i;
            let coverage = (row.right.min(x as f32 + 1.0) - row.left.max(x as f32)).clamp(0.0, 1.0)
                * row.vertical;
            let [pr, pg, pb, pa] = if t <= p0 {
                stops.colors[0]
            } else if t >= p1 {
                stops.colors[1]
            } else {
                let span = p1 - p0;
                let f = if span <= 0.0 { 1.0 } else { (t - p0) / span };
                let (a, b) = (stops.colors[0], stops.colors[1]);
                std::array::from_fn(|c| a[c] + (b[c] - a[c]) * f)
            };
            let a = (row.alpha * coverage).clamp(0.0, 1.0);
            if a <= 0.0 || pa <= 0.0 {
                continue;
            }
            let d = row.dither[x & 3];
            for (channel, premult) in pixel.iter_mut().take(3).zip([pr, pg, pb]) {
                *channel = round_to_u8(premult * a + f32::from(*channel) * (1.0 - pa * a) + d);
            }
            pixel[3] = 255;
        }
    }

    #[test]
    fn two_stop_row_matches_the_per_pixel_blend() {
        let premultiplied = |[r, g, b, a]: [f32; 4]| [r * a, g * a, b * a, a];
        let stop_sets = [
            (
                [0.0, 1.0],
                [[111.0, 141.0, 255.0, 0.34], [111.0, 141.0, 255.0, 0.0]],
            ),
            (
                [0.2, 0.7],
                [[255.0, 0.0, 30.0, 1.0], [0.0, 90.0, 255.0, 0.6]],
            ),
            (
                [0.5, 0.5],
                [[10.0, 20.0, 30.0, 1.0], [200.0, 210.0, 220.0, 1.0]],
            ),
        ];
        for (positions, colors) in stop_sets {
            let stops = TwoStops {
                positions,
                colors: colors.map(premultiplied),
            };
            for (alpha, left, right, vertical) in [
                (1.0, 3.25, 290.5, 1.0),
                (0.37, -4.0, 300.0, 0.5),
                (0.0, 0.0, 300.0, 1.0),
            ] {
                let row = GradientRow {
                    left,
                    right,
                    vertical,
                    alpha,
                    dither: [-0.47, 0.03, -0.22, 0.28],
                };
                let ts: Vec<f32> = (0..300).map(|i| i as f32 / 250.0 - 0.1).collect();
                let start: Vec<[u8; 4]> = (0..300)
                    .map(|i| [(i * 7) as u8, (255 - i) as u8, (i / 2) as u8, 200])
                    .collect();
                for first in [0, 1, 5] {
                    let mut expected = start.clone();
                    reference(&mut expected, &ts, first, &row, &stops);
                    let mut actual = start.clone();
                    two_stop_row(&mut actual, &ts, first, &row, &stops);
                    assert_eq!(
                        actual, expected,
                        "{positions:?} alpha {alpha} first {first}"
                    );
                    let mut baseline = start.clone();
                    two_stop_row_impl(&mut baseline, &ts, first, &row, &stops);
                    assert_eq!(baseline, expected);
                }
            }
        }
    }
}
