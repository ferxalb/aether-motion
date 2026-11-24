//! Deterministic `f32` math for procedural shaders.
//!
//! The platform `sin`, `cos`, `exp`, `ln`, `powf`, and `atan2` call the system math library,
//! whose last bits differ between operating systems. These replacements use only IEEE addition,
//! multiplication, division, square root, comparisons, and bit operations, which are exact in
//! every SIMD width, so a shader gives the same pixels on every machine. Every function is
//! branch-free (selects only) and `#[inline(always)]`, so the row loops of `vm.rs` vectorize. No
//! function relies on fused multiply-add. Accuracy is a few units in the sixth decimal place,
//! far below one 8-bit color level.

use std::f32::consts::{FRAC_PI_2, PI};

/// Values at or above this magnitude are already integers in `f32`.
const INTEGRAL: f32 = 8_388_608.0;

/// `floor` without a library call: truncation through `i32`, then one step down for negative
/// fractions. Exact for every finite `f32`; NaN gives a finite value.
#[inline(always)]
pub fn floor(x: f32) -> f32 {
    // `max`/`min` rather than `clamp`: they map NaN to a bound, so the conversion is defined.
    #[allow(clippy::manual_clamp)]
    let clamped = x.max(-INTEGRAL).min(INTEGRAL);
    let truncated = clamped as i32 as f32;
    let floored = truncated - if truncated > clamped { 1.0 } else { 0.0 };
    if x.abs() >= INTEGRAL {
        x
    } else {
        floored
    }
}

#[inline(always)]
pub fn fract(x: f32) -> f32 {
    x - floor(x)
}

/// GLSL `mod`: `a - b * floor(a / b)`, so the result has the sign of `b`. Zero `b` gives 0.
#[inline(always)]
pub fn modulo(a: f32, b: f32) -> f32 {
    let r = a - b * floor(a / b);
    if b == 0.0 {
        0.0
    } else {
        r
    }
}

/// The angle `x` reduced to `[-pi, pi]` with a two-part `2 pi`.
#[inline(always)]
fn reduce(x: f32) -> f32 {
    const TAU_HI: f32 = 6.283_185_5;
    const TAU_LO: f32 = -1.748_455_5e-7;
    let k = floor(x * (1.0 / std::f32::consts::TAU) + 0.5);
    (x - k * TAU_HI) - k * TAU_LO
}

/// `sin(x)`: the angle is reduced to `[-pi, pi]`, folded into `[-pi/2, pi/2]`, and evaluated
/// with the odd Taylor polynomial of degree 11.
#[inline(always)]
pub fn sin(x: f32) -> f32 {
    sin_reduced(reduce(x))
}

/// `sin` of an angle in `[-3pi/2, 3pi/2]`.
#[inline(always)]
fn sin_reduced(r: f32) -> f32 {
    let r = if r > FRAC_PI_2 {
        PI - r
    } else if r < -FRAC_PI_2 {
        -PI - r
    } else {
        r
    };
    let r2 = r * r;
    let p = -2.505_210_8e-8_f32;
    let p = p * r2 + 2.755_731_9e-6;
    let p = p * r2 - 1.984_127e-4;
    let p = p * r2 + 8.333_333e-3;
    let p = p * r2 - 1.666_666_7e-1;
    let p = p * r2 + 1.0;
    // Large or non-finite inputs have no meaningful phase; keep the result in range (`max` and
    // `min` rather than `clamp`, which would keep a NaN).
    #[allow(clippy::manual_clamp)]
    let s = (r * p).max(-1.0).min(1.0);
    s
}

#[inline(always)]
pub fn cos(x: f32) -> f32 {
    // The quarter turn is added after the reduction, where it rounds at the scale of pi.
    sin_reduced(reduce(x) + FRAC_PI_2)
}

/// `2^x` for `x` clamped to `[-126, 127]`: the integer part goes into the exponent bits and the
/// fraction through a degree 7 polynomial of `e^(f ln 2)`.
#[inline(always)]
pub fn exp2(x: f32) -> f32 {
    #[allow(clippy::manual_clamp)]
    let x = x.max(-126.0).min(127.0);
    let i = floor(x);
    let f = (x - i) * std::f32::consts::LN_2;
    let p = 1.0 / 5040.0_f32;
    let p = p * f + 1.0 / 720.0;
    let p = p * f + 1.0 / 120.0;
    let p = p * f + 1.0 / 24.0;
    let p = p * f + 1.0 / 6.0;
    let p = p * f + 0.5;
    let p = p * f + 1.0;
    let p = p * f + 1.0;
    let scale = f32::from_bits(((i as i32 + 127) as u32) << 23);
    p * scale
}

/// `log2(x)`, with inputs below the smallest normal `f32` (zero and negatives included) read as
/// that value (about -126). The mantissa in `[1, 2)` goes through the `atanh` series.
#[inline(always)]
pub fn log2(x: f32) -> f32 {
    let x = x.max(f32::MIN_POSITIVE);
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let p = 1.0 / 11.0_f32;
    let p = p * s2 + 1.0 / 9.0;
    let p = p * s2 + 1.0 / 7.0;
    let p = p * s2 + 1.0 / 5.0;
    let p = p * s2 + 1.0 / 3.0;
    let p = p * s2 + 1.0;
    let ln_m = 2.0 * s * p;
    exponent as f32 + ln_m * std::f32::consts::LOG2_E
}

/// `a^b` for positive `a`; zero and negative bases give 0.
#[inline(always)]
pub fn pow(a: f32, b: f32) -> f32 {
    let r = exp2(b * log2(a));
    if a > 0.0 {
        r
    } else {
        0.0
    }
}

/// `atan2(y, x)` in `(-pi, pi]`: the smaller-over-larger ratio through a degree 11 odd minimax
/// polynomial (error about 1e-5 rad), then the octant corrections. `atan2(0, 0)` is 0.
#[inline(always)]
pub fn atan2(y: f32, x: f32) -> f32 {
    let ax = x.abs();
    let ay = y.abs();
    let big = ax.max(ay);
    let small = ax.min(ay);
    let z = if big > 0.0 { small / big } else { 0.0 };
    let s = z * z;
    let p = -0.011_721_2_f32;
    let p = p * s + 0.052_653_32;
    let p = p * s - 0.116_432_87;
    let p = p * s + 0.193_543_46;
    let p = p * s - 0.332_623_47;
    let p = p * s + 0.999_977_26;
    let a = z * p;
    let a = if ay > ax { FRAC_PI_2 - a } else { a };
    let a = if x < 0.0 { PI - a } else { a };
    if y < 0.0 {
        -a
    } else {
        a
    }
}

#[inline(always)]
pub fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// GLSL `step(edge, x)`: 0 below the edge, 1 at or above it.
#[inline(always)]
pub fn step(edge: f32, x: f32) -> f32 {
    if x < edge {
        0.0
    } else {
        1.0
    }
}

/// GLSL `smoothstep(e0, e1, x)`; equal edges act as a step at `e0`.
#[inline(always)]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let d = e1 - e0;
    #[allow(clippy::manual_clamp)]
    let t = if d != 0.0 {
        ((x - e0) / d).max(0.0).min(1.0)
    } else {
        step(e0, x)
    };
    t * t * (3.0 - 2.0 * t)
}

/// Safe division: a zero divisor gives 0 instead of an infinity or NaN.
#[inline(always)]
pub fn div(a: f32, b: f32) -> f32 {
    let r = a / b;
    if b == 0.0 {
        0.0
    } else {
        r
    }
}

/// Square root of the positive part (negative inputs give 0).
#[inline(always)]
pub fn sqrt(x: f32) -> f32 {
    x.max(0.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f64, tolerance: f64) -> bool {
        (f64::from(a) - b).abs() <= tolerance
    }

    #[test]
    fn floor_matches_the_library_floor() {
        for &x in &[
            -3.5_f32,
            -3.0,
            -0.25,
            -0.0,
            0.0,
            0.25,
            0.999_999_9,
            1.0,
            2.5,
            8_388_607.5,
            1e9,
            -1e9,
        ] {
            assert_eq!(floor(x), x.floor(), "{x}");
        }
        assert!(floor(f32::NAN).is_finite());
    }

    #[test]
    fn trigonometry_is_accurate() {
        let mut x = -50.0_f32;
        while x < 50.0 {
            assert!(close(sin(x), f64::from(x).sin(), 2e-6), "sin {x}");
            assert!(close(cos(x), f64::from(x).cos(), 2e-6), "cos {x}");
            x += 0.0137;
        }
    }

    #[test]
    fn exponentials_and_logarithms_are_accurate() {
        let mut x = -20.0_f32;
        while x < 20.0 {
            let reference = 2f64.powf(f64::from(x));
            assert!(close(exp2(x), reference, reference * 4e-6), "exp2 {x}");
            x += 0.031;
        }
        let mut x = 0.001_f32;
        while x < 1000.0 {
            assert!(close(log2(x), f64::from(x).log2(), 3e-6), "log2 {x}");
            x *= 1.07;
        }
        assert!(close(pow(2.0, 0.5), 2f64.sqrt(), 1e-6));
        assert_eq!(pow(-1.0, 2.0), 0.0);
    }

    #[test]
    fn atan2_covers_every_quadrant() {
        for i in 0..720 {
            let angle = f64::from(i) * std::f64::consts::PI / 360.0 - std::f64::consts::PI;
            let (y, x) = (angle.sin() as f32 * 3.0, angle.cos() as f32 * 3.0);
            let expected = f64::from(y).atan2(f64::from(x));
            assert!(close(atan2(y, x), expected, 2e-5), "atan2 at {angle}");
        }
        assert_eq!(atan2(0.0, 0.0), 0.0);
    }
}
