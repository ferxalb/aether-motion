//! Checked pixel-buffer sizes and coordinate conversions shared by frame and offscreen allocation.

use anyhow::{bail, Result};

pub(super) const MAX_RGBA_BYTES: usize = 128 * 1024 * 1024;

pub(super) fn rgba_bytes(width: usize, height: usize, context: &str) -> Result<usize> {
    let bytes = width.checked_mul(height).and_then(|n| n.checked_mul(4));
    match bytes {
        Some(n) if n <= MAX_RGBA_BYTES && n <= isize::MAX as usize => Ok(n),
        _ => bail!(
            "{context} requires more than 128 MiB or has unrepresentable dimensions; use --scale"
        ),
    }
}

pub(super) fn layer_dimensions(edges: [i32; 4], context: &str) -> Result<(usize, usize)> {
    let [x0, y0, x1, y1] = edges;
    let width = usize::try_from(i64::from(x1) - i64::from(x0))?;
    let height = usize::try_from(i64::from(y1) - i64::from(y0))?;
    rgba_bytes(width, height, context)?;
    Ok((width, height))
}

pub(super) fn pixel_edge(value: f32, context: &str) -> Result<i32> {
    // i32::MAX rounds up when converted to f32, so the upper bound is exclusive.
    if !value.is_finite() || value < i32::MIN as f32 || value >= i32::MAX as f32 {
        bail!(
            "{context} has an unrepresentable pixel coordinate; reduce its size, offset, or scale"
        );
    }
    Ok(value as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_limits_are_checked_before_multiplication_or_subtraction() {
        assert_eq!(rgba_bytes(1920, 1080, "frame").unwrap(), 8_294_400);
        assert_eq!(
            rgba_bytes(MAX_RGBA_BYTES / 4, 1, "frame").unwrap(),
            MAX_RGBA_BYTES
        );
        assert!(rgba_bytes(usize::MAX, 2, "frame").is_err());
        assert!(rgba_bytes(MAX_RGBA_BYTES / 4 + 1, 1, "frame").is_err());
        assert!(layer_dimensions([i32::MIN, 0, i32::MAX, 2], "layer").is_err());
        assert!(layer_dimensions([5, 0, 4, 2], "layer").is_err());
        assert_eq!(
            layer_dimensions([-100, -50, 100, 50], "layer").unwrap(),
            (200, 100)
        );
    }

    #[test]
    fn coordinate_checks_reject_nonfinite_and_rounded_upper_bound() {
        for n in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, i32::MAX as f32] {
            assert!(pixel_edge(n, "layer").is_err());
        }
        assert_eq!(pixel_edge(i32::MIN as f32, "layer").unwrap(), i32::MIN);
        assert_eq!(pixel_edge(-12.0, "layer").unwrap(), -12);
    }
}
