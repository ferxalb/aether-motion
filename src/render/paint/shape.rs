//! Distance and error-function helpers shared by border and shadow painting.

/// Signed distance from `(x, y)` to a rounded rectangle; negative inside.
pub(in crate::render) fn rounded_distance(
    x: f32,
    y: f32,
    [x0, y0, x1, y1]: [f32; 4],
    radius: f32,
) -> f32 {
    let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let radius = radius.clamp(0.0, hx.min(hy).max(0.0));
    let qx = (x - (x0 + hx)).abs() - (hx - radius);
    let qy = (y - (y0 + hy)).abs() - (hy - radius);
    // Only the corner arcs need a square root.
    let outside = if qx > 0.0 && qy > 0.0 {
        qx.hypot(qy)
    } else {
        qx.max(qy)
    };
    outside - radius
}

/// Complementary error function (Abramowitz & Stegun 7.1.26, error below 1.5e-7).
pub(in crate::render) fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_72 + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let tail = poly * (-z * z).exp();
    if x >= 0.0 {
        tail
    } else {
        2.0 - tail
    }
}
