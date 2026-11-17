//! Antialiased polygon coverage with exact signed-area accumulation (nonzero winding, clamped), used
//! by `clip-path`. Each edge deposits the area it covers in each pixel into an accumulation buffer; a
//! running sum along every row then gives the covered share of each pixel. Pure arithmetic in a
//! fixed order, so the result is deterministic.

/// Coverage (0..=1) of a closed polygon over a `width` x `height` grid of pixels whose top-left
/// corner is the origin. Points are in pixel units; parts outside the grid are clipped exactly.
pub(in crate::render) fn polygon_coverage(
    points: &[(f32, f32)],
    width: usize,
    height: usize,
) -> Vec<f32> {
    let stride = width + 2;
    let mut acc = vec![0.0_f32; stride * height + 2];
    if width == 0 || height == 0 || points.len() < 3 {
        return vec![0.0; width * height];
    }
    // Clamping x to the grid keeps each row's winding exact: an edge left of the grid adds to every
    // pixel of its rows (as it does at x = 0) and one right of it adds to none.
    let clamp = |(x, y): (f32, f32)| (x.clamp(0.0, width as f32), y);
    for i in 0..points.len() {
        let p0 = clamp(points[i]);
        let p1 = clamp(points[(i + 1) % points.len()]);
        line(&mut acc, stride, height, p0, p1);
    }
    let mut out = Vec::with_capacity(width * height);
    for row in acc.chunks_exact(stride).take(height) {
        let mut sum = 0.0_f32;
        for &a in &row[..width] {
            sum += a;
            out.push(sum.abs().min(1.0));
        }
    }
    out
}

/// Adds one edge's signed area to the accumulation buffer.
fn line(acc: &mut [f32], stride: usize, height: usize, p0: (f32, f32), p1: (f32, f32)) {
    if (p0.1 - p1.1).abs() <= f32::EPSILON || !(p0.1.is_finite() && p1.1.is_finite()) {
        return;
    }
    let (dir, p0, p1) = if p0.1 < p1.1 {
        (1.0, p0, p1)
    } else {
        (-1.0, p1, p0)
    };
    let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
    let top = p0.1.max(0.0);
    let bottom = p1.1.min(height as f32);
    if top >= bottom {
        return;
    }
    let mut x = p0.0 + (top - p0.1) * dxdy;
    for y in top as usize..(bottom.ceil() as usize).min(height) {
        let start = y * stride;
        let dy = ((y + 1) as f32).min(bottom) - (y as f32).max(top);
        let next = x + dxdy * dy;
        let d = dy * dir;
        let (x0, x1) = if x < next { (x, next) } else { (next, x) };
        let x0_floor = x0.floor();
        let x0i = x0_floor as usize;
        let x1_ceil = x1.ceil();
        let x1i = x1_ceil as usize;
        if x1i <= x0i + 1 {
            // The edge stays inside one pixel column on this row.
            let mid = 0.5 * (x + next) - x0_floor;
            acc[start + x0i] += d - d * mid;
            acc[start + x0i + 1] += d * mid;
        } else {
            let s = (x1 - x0).recip();
            let x0f = x0 - x0_floor;
            let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
            let x1f = x1 - x1_ceil + 1.0;
            let am = 0.5 * s * x1f * x1f;
            acc[start + x0i] += d * a0;
            if x1i == x0i + 2 {
                acc[start + x0i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - x0f);
                acc[start + x0i + 1] += d * (a1 - a0);
                for xi in x0i + 2..x1i - 1 {
                    acc[start + xi] += d * s;
                }
                let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                acc[start + x1i - 1] += d * (1.0 - a2 - am);
            }
            acc[start + x1i] += d * am;
        }
        x = next;
    }
}

/// Points of an ellipse centered at `(cx, cy)` with radii `(rx, ry)`. The chord error stays under
/// about 0.02 px, and the vertices sit slightly outside the curve so the polygon's area equals the
/// ellipse's (an inscribed polygon would shrink it).
pub(in crate::render) fn ellipse_points(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<(f32, f32)> {
    let r = rx.max(ry).max(0.0);
    let n = ((std::f32::consts::PI / (1.0 - 0.02 / r.max(0.04)).clamp(-1.0, 1.0).acos()).ceil()
        as usize)
        .clamp(16, 4096);
    let step = std::f32::consts::TAU / n as f32;
    let grow = (step / step.sin()).sqrt();
    (0..n)
        .map(|i| {
            let a = i as f32 * step;
            (cx + rx * grow * a.cos(), cy + ry * grow * a.sin())
        })
        .collect()
}

/// Points of a rectangle `[x0, y0, x1, y1]` with corners rounded by `radius` (clamped to fit).
pub(in crate::render) fn rounded_rect_points(
    [x0, y0, x1, y1]: [f32; 4],
    radius: f32,
) -> Vec<(f32, f32)> {
    let r = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
    if r <= 0.0 {
        return vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    }
    let quarter = ellipse_points(0.0, 0.0, r, r).len().div_ceil(4).max(4);
    let mut out = Vec::with_capacity(quarter * 4 + 4);
    let corners = [
        (x1 - r, y0 + r, -90.0_f32),
        (x1 - r, y1 - r, 0.0),
        (x0 + r, y1 - r, 90.0),
        (x0 + r, y0 + r, 180.0),
    ];
    for (cx, cy, start) in corners {
        for i in 0..=quarter {
            let a = (start + 90.0 * i as f32 / quarter as f32).to_radians();
            out.push((cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_aligned_square_has_exact_fractional_edges() {
        let square = [(1.25, 1.0), (3.5, 1.0), (3.5, 3.0), (1.25, 3.0)];
        let c = polygon_coverage(&square, 5, 4);
        let at = |x: usize, y: usize| c[y * 5 + x];
        assert_eq!(at(0, 1), 0.0);
        assert!((at(1, 1) - 0.75).abs() < 1e-6);
        assert_eq!(at(2, 2), 1.0);
        assert!((at(3, 2) - 0.5).abs() < 1e-6);
        assert_eq!(at(4, 2), 0.0);
        assert_eq!(at(2, 0), 0.0);
    }

    #[test]
    fn total_coverage_matches_the_area() {
        let triangle: [(f32, f32); 3] = [(2.0, 1.0), (18.3, 4.7), (6.1, 15.2)];
        let area = 0.5
            * ((triangle[1].0 - triangle[0].0) * (triangle[2].1 - triangle[0].1)
                - (triangle[2].0 - triangle[0].0) * (triangle[1].1 - triangle[0].1))
                .abs();
        let total: f32 = polygon_coverage(&triangle, 20, 20).iter().sum();
        assert!((total - area).abs() < 1e-3, "{total} vs {area}");
        // Winding direction does not matter.
        let reversed: Vec<_> = triangle.iter().rev().copied().collect();
        assert_eq!(
            polygon_coverage(&triangle, 20, 20),
            polygon_coverage(&reversed, 20, 20)
        );
    }

    #[test]
    fn shapes_outside_the_grid_are_clipped() {
        let big = [(-10.0, -10.0), (30.0, -10.0), (30.0, 30.0), (-10.0, 30.0)];
        assert!(polygon_coverage(&big, 8, 8).iter().all(|&c| c == 1.0));
        let left = [(-10.0, 0.0), (-5.0, 0.0), (-5.0, 8.0), (-10.0, 8.0)];
        assert!(polygon_coverage(&left, 8, 8).iter().all(|&c| c == 0.0));
    }

    #[test]
    fn circle_area_is_close_to_pi_r_squared() {
        let points = ellipse_points(50.0, 50.0, 30.0, 30.0);
        let total: f32 = polygon_coverage(&points, 100, 100).iter().sum();
        let expected = std::f32::consts::PI * 900.0;
        assert!((total - expected).abs() / expected < 0.0005, "{total}");
    }
}
