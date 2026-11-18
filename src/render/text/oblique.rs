//! Synthetic oblique for italic text without an italic face (`font-style: italic` on the embedded
//! Roboto, for example), as browsers synthesize it: glyphs lean right by the CSS default `oblique`
//! angle of 14 degrees, around the baseline, so advances and line layout do not change.
//!
//! The fill coverage is sheared row by row with a two-tap linear split of each pixel (exact integer
//! rounding, so it is deterministic); glyph strokes shear their outlines instead (see `stroke.rs`).

/// Horizontal shift per pixel of height above the baseline: tan(14°).
pub(in crate::render) const SKEW: f32 = 0.249_328;

/// Shears a glyph bitmap of `metrics` (rows top to bottom, `width` x `height`) to the right by
/// [`SKEW`] per pixel above the baseline, widening the bitmap and moving `xmin` as needed.
pub(in crate::render) fn slant(metrics: &mut fontdue::Metrics, coverage: &[u8]) -> Vec<u8> {
    let (width, height) = (metrics.width, metrics.height);
    if width == 0 || height == 0 {
        return coverage.to_vec();
    }
    // Pixel-center heights above the baseline of the bottom and top rows.
    let bottom = metrics.ymin as f32 + 0.5;
    let top = metrics.ymin as f32 + height as f32 - 0.5;
    let first = (SKEW * bottom).floor() as i32;
    let last = (SKEW * top).floor() as i32 + 1;
    let out_width = width + (last - first) as usize;
    // Coverage x256, so each split is exact before the final rounding.
    let mut sums = vec![0u32; out_width * height];
    for (row, line) in coverage.chunks_exact(width).enumerate() {
        let shift = SKEW * (top - row as f32) - first as f32;
        let whole = shift.floor();
        let right = ((shift - whole) * 256.0).round() as u32;
        let base = row * out_width + whole as usize;
        for (x, &value) in line.iter().enumerate() {
            if value == 0 {
                continue;
            }
            let value = u32::from(value);
            sums[base + x] += value * (256 - right);
            sums[base + x + 1] += value * right;
        }
    }
    metrics.xmin += first;
    metrics.width = out_width;
    sums.into_iter()
        .map(|sum| ((sum + 128) >> 8).min(255) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(width: usize, height: usize, ymin: i32) -> fontdue::Metrics {
        fontdue::Metrics {
            xmin: 0,
            ymin,
            width,
            height,
            advance_width: width as f32,
            advance_height: 0.0,
            bounds: fontdue::OutlineBounds::default(),
        }
    }

    #[test]
    fn upright_bar_leans_right_and_keeps_its_ink() {
        // A 1 px wide, 8 px tall bar sitting on the baseline.
        let mut m = metrics(1, 8, 0);
        let slanted = slant(&mut m, &[255; 8]);
        assert_eq!(slanted.len(), m.width * m.height);
        assert!(m.width > 1);
        let column = |row: usize| {
            let line = &slanted[row * m.width..(row + 1) * m.width];
            let ink: u32 = line.iter().map(|&v| u32::from(v)).sum();
            let center: f32 = line
                .iter()
                .enumerate()
                .map(|(x, &v)| x as f32 * f32::from(v))
                .sum::<f32>()
                / ink as f32;
            (ink, center)
        };
        // Every row keeps its coverage (within rounding) and the top leans furthest right.
        for row in 0..8 {
            let (ink, _) = column(row);
            assert!((254..=256).contains(&ink), "row {row}: {ink}");
        }
        let lean = column(0).1 - column(7).1;
        assert!((lean - SKEW * 7.0).abs() < 0.05, "{lean}");
    }

    #[test]
    fn descenders_lean_left_of_the_origin() {
        let mut m = metrics(2, 10, -4);
        slant(&mut m, &[200; 20]);
        assert!(m.xmin < 0, "rows below the baseline shift left");
        let mut empty = metrics(0, 0, 0);
        assert!(slant(&mut empty, &[]).is_empty());
    }
}
