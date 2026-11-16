//! Exact, call-free float helpers for per-pixel loops.
//!
//! The baseline x86-64 target has no rounding instruction, so `f32::round` compiles to a call into
//! the soft-float runtime, and `f32::hypot` calls libm. These helpers return bit-for-bit the same
//! results for the cases the painters use, so swapping them in never changes a pixel.

/// `value.round().clamp(0.0, 255.0) as u8`, which also equals `value.round() as u8`, without the
/// runtime call: exact, because the fractional part of a float is computed without error.
#[inline]
pub(in crate::render) fn round_to_u8(value: f32) -> u8 {
    let value = value.clamp(0.0, 255.0);
    let whole = value as u8;
    whole + u8::from(value - f32::from(whole) >= 0.5 && whole < 255)
}

/// `x.hypot(y)` for finite arguments, inlined: the squares are exact in `f64`, so the result is
/// the correctly rounded `sqrt(x² + y²)`. glibc's `hypotf` uses this same evaluation, so on Linux
/// the result is bit-identical to `f32::hypot` (a test checks it); elsewhere it removes the
/// platform's libm from the result.
#[inline]
pub(in crate::render) fn hypot(x: f32, y: f32) -> f32 {
    if x.is_infinite() || y.is_infinite() {
        return f32::INFINITY;
    }
    let (x, y) = (f64::from(x), f64::from(y));
    (x * x + y * y).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_to_u8_matches_round_and_clamp() {
        for i in -600..=5600 {
            let v = i as f32 * 0.05 - 4.0;
            assert_eq!(round_to_u8(v), v.round().clamp(0.0, 255.0) as u8, "{v}");
            assert_eq!(round_to_u8(v), v.round() as u8, "{v}");
        }
        for v in [
            0.499_999_97_f32,
            0.5,
            -0.5,
            -0.499_999_97,
            254.5,
            254.499_97,
            255.0,
            255.5,
            300.0,
            -3.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ] {
            assert_eq!(round_to_u8(v), v.round().clamp(0.0, 255.0) as u8, "{v}");
            assert_eq!(round_to_u8(v), v.round() as u8, "{v}");
        }
        // The values on both sides of every rounding boundary.
        for k in -2..=257 {
            let half = k as f32 + 0.5;
            for v in [half.next_down(), half, half.next_up(), (k as f32).next_down(), k as f32] {
                assert_eq!(round_to_u8(v), v.round() as u8, "{v}");
            }
        }
    }

    #[test]
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn hypot_matches_glibc_bit_for_bit() {
        let mut state = 0xDEAD_BEEF_u32;
        for _ in 0..500_000 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let x = (state % 40_000) as f32 / 13.0 - 1500.0;
            let y = ((state >> 8) % 40_000) as f32 / 7.0 - 2800.0;
            assert_eq!(hypot(x, y).to_bits(), x.hypot(y).to_bits(), "hypot({x}, {y})");
            let (a, b) = (f32::from_bits(state >> 1), f32::from_bits(state.rotate_left(7) >> 1));
            if a.is_finite() && b.is_finite() {
                assert_eq!(hypot(a, b).to_bits(), a.hypot(b).to_bits(), "hypot({a:e}, {b:e})");
            }
        }
    }

    #[test]
    fn hypot_handles_infinity_like_c() {
        assert_eq!(hypot(f32::INFINITY, f32::NAN), f32::INFINITY);
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert!(hypot(f32::NAN, 1.0).is_nan());
    }
}
