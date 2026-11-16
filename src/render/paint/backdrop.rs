//! `backdrop-filter: blur()`: blurs what is already painted behind an element, inside its rounded box.
//!
//! Wide blurs (sigma of 16 px or more) run on a reduced copy of the region and are scaled back up
//! with bilinear filtering: the result is smooth, so detail is not lost, and the work drops by the
//! square of the reduction. The reduction keeps at least 8 px of sigma at the reduced size.
//!
//! Three successive box blurs approximate the gaussian (as in CSS the radius is the standard deviation).
//! Only the pixels inside the box are sampled and edge samples are clamped, so the result is a pure
//! function of the pixels behind the element and stays deterministic.

use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::float_ops::round_to_u8;
use crate::render::paint::Surface;

/// Radius in pixels of each box blur so that three of them approximate a gaussian of `sigma`.
pub(super) fn box_radius(sigma: f32) -> usize {
    let width = (4.0 * sigma * sigma + 1.0).sqrt();
    ((width - 1.0) / 2.0).round().max(0.0) as usize
}

/// Reduction factor (1, 2, or 4) for a blur of `sigma` px over a `width` x `height` region:
/// the largest that leaves the reduced blur at least 8 px wide. Narrow blurs and regions smaller
/// than two sigmas (where edge clamping dominates and the work is small anyway) stay full size.
fn reduction(sigma: f32, width: usize, height: usize) -> usize {
    if (width.min(height) as f32) < 2.0 * sigma {
        return 1;
    }
    [4, 2]
        .into_iter()
        .find(|&f| sigma / f as f32 >= 8.0)
        .unwrap_or(1)
}

/// Blurs `plane` (`width` x `height`) with a gaussian of `sigma` px on a copy reduced `factor`
/// times (block averages), then writes the bilinear upscale back into `plane`.
fn blur_plane_reduced(plane: &mut [f32], width: usize, height: usize, sigma: f32, factor: usize) {
    let (rw, rh) = (width.div_ceil(factor), height.div_ceil(factor));
    let mut small = vec![0.0_f32; rw * rh];
    for (ry, out) in small.chunks_exact_mut(rw).enumerate() {
        let (y0, y1) = (ry * factor, ((ry + 1) * factor).min(height));
        for (rx, value) in out.iter_mut().enumerate() {
            let (x0, x1) = (rx * factor, ((rx + 1) * factor).min(width));
            let mut sum = 0.0_f32;
            for y in y0..y1 {
                sum += plane[y * width + x0..y * width + x1].iter().sum::<f32>();
            }
            *value = sum / ((y1 - y0) * (x1 - x0)) as f32;
        }
    }
    // Match the strength of the full-size blur (three equal boxes of `box_radius(sigma)`),
    // which is what the unreduced path would produce, not the nominal sigma.
    let width_px = (2 * box_radius(sigma) + 1) as f32;
    let effective = (width_px * width_px - 1.0).max(0.0).sqrt() / 2.0;
    blur_plane_radii(&mut small, rw, rh, box_radii(effective / factor as f32));
    // Sample positions: pixel centers mapped onto block centers, clamped at the edges.
    let taps = |n: usize, reduced: usize| -> Vec<(usize, usize, f32)> {
        (0..n)
            .map(|i| {
                let u = ((i as f32 + 0.5) / factor as f32 - 0.5).max(0.0);
                let a = (u as usize).min(reduced - 1);
                (a, (a + 1).min(reduced - 1), (u - a as f32).clamp(0.0, 1.0))
            })
            .collect()
    };
    let (columns, rows) = (taps(width, rw), taps(height, rh));
    for (y, out) in plane.chunks_exact_mut(width).enumerate() {
        let (ra, rb, ty) = rows[y];
        let (top, bottom) = (
            &small[ra * rw..(ra + 1) * rw],
            &small[rb * rw..(rb + 1) * rw],
        );
        for (value, &(ca, cb, tx)) in out.iter_mut().zip(&columns) {
            let upper = top[ca] + (top[cb] - top[ca]) * tx;
            let lower = bottom[ca] + (bottom[cb] - bottom[ca]) * tx;
            *value = upper + (lower - upper) * ty;
        }
    }
}

/// One box blur of radius `radius` over `line`, clamping samples at both ends. `tmp` is scratch space.
fn box_line(line: &mut [f32], tmp: &mut Vec<f32>, radius: usize) {
    box_lines([line], tmp, radius);
}

/// `N` independent lines blurred in lockstep. Each line keeps its own running sum and performs
/// exactly the operations of a lone `box_line` in the same order (results are bit-identical), but
/// the `N` dependency chains overlap, which is what makes the horizontal pass fast.
fn box_lines<const N: usize>(mut lines: [&mut [f32]; N], tmp: &mut Vec<f32>, radius: usize) {
    let n = lines[0].len();
    if n == 0 || radius == 0 {
        return;
    }
    let r = radius as isize;
    let last = n as isize - 1;
    let window = (2 * radius + 1) as f32;
    let mut sums: [f32; N] =
        std::array::from_fn(|k| (-r..=r).map(|j| lines[k][j.clamp(0, last) as usize]).sum());
    tmp.clear();
    tmp.resize(N * n, 0.0);
    // Interior pixels need no index clamping; only the first and last `radius` pixels do.
    let interior_from = (r + 1).min(n as isize) as usize;
    let interior_to = ((n as isize - r - 1).max(interior_from as isize) as usize).min(n);
    for i in 0..interior_from {
        let (add, sub) = (
            (i as isize + r + 1).min(last) as usize,
            (i as isize - r).max(0) as usize,
        );
        for k in 0..N {
            tmp[k * n + i] = sums[k] / window;
            sums[k] += lines[k][add] - lines[k][sub];
        }
    }
    for i in interior_from..interior_to {
        for k in 0..N {
            tmp[k * n + i] = sums[k] / window;
            sums[k] += lines[k][i + radius + 1] - lines[k][i - radius];
        }
    }
    for i in interior_to.max(interior_from)..n {
        let (add, sub) = (
            (i as isize + r + 1).min(last) as usize,
            (i as isize - r).max(0) as usize,
        );
        for k in 0..N {
            tmp[k * n + i] = sums[k] / window;
            sums[k] += lines[k][add] - lines[k][sub];
        }
    }
    for (k, line) in lines.iter_mut().enumerate() {
        line.copy_from_slice(&tmp[k * n..(k + 1) * n]);
    }
}

/// One vertical box blur of radius `radius` from `src` into `dst` (both `width` x `height`).
///
/// A running sum per column slides down the rows, so memory is read row by row (cache friendly)
/// and the cost does not depend on the radius. `sums` is scratch space of `width` values.
fn blur_vertical(
    src: &[f32],
    dst: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    sums: &mut Vec<f32>,
) {
    if width == 0 || height == 0 || radius == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let r = radius as isize;
    let last = height as isize - 1;
    let window = (2 * radius + 1) as f32;
    let row = |y: isize| {
        let y = y.clamp(0, last) as usize;
        &src[y * width..(y + 1) * width]
    };
    sums.clear();
    sums.resize(width, 0.0);
    for k in -r..=r {
        for (sum, value) in sums.iter_mut().zip(row(k)) {
            *sum += value;
        }
    }
    for y in 0..height as isize {
        let out = &mut dst[y as usize * width..(y as usize + 1) * width];
        for (target, sum) in out.iter_mut().zip(sums.iter()) {
            *target = sum / window;
        }
        let (add, sub) = (row(y + r + 1), row(y - r));
        for ((sum, a), s) in sums.iter_mut().zip(add).zip(sub) {
            *sum += a - s;
        }
    }
}

/// Box radii for three passes whose combined variance matches a gaussian of `sigma` px: a mix of
/// two neighboring widths, so the blur strength is accurate even at small sigma where one
/// integer radius would be off by several percent.
fn box_radii(sigma: f32) -> [usize; 3] {
    let ideal = (4.0 * sigma * sigma + 1.0).sqrt();
    let mut lower = ideal.floor() as i64;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let lower = lower.max(1) as f32;
    let narrower = ((12.0 * sigma * sigma - 3.0 * lower * lower - 12.0 * lower - 9.0)
        / (-4.0 * lower - 4.0))
        .round()
        .clamp(0.0, 3.0) as usize;
    let radius = |width: f32| ((width - 1.0) / 2.0) as usize;
    std::array::from_fn(|i| radius(if i < narrower { lower } else { lower + 2.0 }))
}

/// Three horizontal then three vertical box blurs over a `width` x `height` plane.
pub(super) fn blur_plane(plane: &mut [f32], width: usize, height: usize, radius: usize) {
    blur_plane_radii(plane, width, height, [radius; 3]);
}

/// Like `blur_plane`, with a separate box radius for each of the three passes.
fn blur_plane_radii(plane: &mut [f32], width: usize, height: usize, radii: [usize; 3]) {
    let mut tmp = Vec::with_capacity(width * 4);
    for radius in radii {
        let mut groups = plane.chunks_exact_mut(width * 4);
        for group in &mut groups {
            let (a, rest) = group.split_at_mut(width);
            let (b, rest) = rest.split_at_mut(width);
            let (c, d) = rest.split_at_mut(width);
            box_lines([a, b, c, d], &mut tmp, radius);
        }
        for row in groups.into_remainder().chunks_exact_mut(width) {
            box_line(row, &mut tmp, radius);
        }
    }
    let mut scratch = vec![0.0_f32; plane.len()];
    let mut sums = Vec::new();
    // Ping-pong between the plane and the scratch buffer; three passes end in the scratch.
    blur_vertical(plane, &mut scratch, width, height, radii[0], &mut sums);
    blur_vertical(&scratch, plane, width, height, radii[1], &mut sums);
    blur_vertical(plane, &mut scratch, width, height, radii[2], &mut sums);
    plane.copy_from_slice(&scratch);
}

impl Surface {
    /// Replaces the pixels inside `rect` (rounded by `radius`) with their blurred version.
    ///
    /// `sigma` is the blur's standard deviation in output pixels; `alpha` mixes the blur with the original.
    pub(in crate::render) fn backdrop_blur(
        &mut self,
        rect: Rect,
        radius: f32,
        sigma: f32,
        alpha: f32,
    ) {
        let box_r = box_radius(sigma);
        if box_r == 0 || alpha <= 0.0 {
            return;
        }
        let (edges, radius_px) = self.fill_edges(rect, radius);
        let [x0, y0, x1, y1] = edges;
        let Some(bounds) = clip_pixels((self.width, self.height), x0, y0, x1, y1) else {
            return;
        };
        let (w, h) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
        let stride = self.width as usize * 4;
        let mut planes = vec![vec![0.0_f32; w * h]; 3];
        for y in 0..h {
            let base = (bounds.y0 + y) * stride + bounds.x0 * 4;
            let source = self.pixels[base..base + w * 4].as_chunks::<4>().0;
            for (c, plane) in planes.iter_mut().enumerate() {
                for (target, pixel) in plane[y * w..(y + 1) * w].iter_mut().zip(source) {
                    *target = f32::from(pixel[c]);
                }
            }
        }
        let factor = reduction(sigma, w, h);
        for plane in &mut planes {
            if factor > 1 {
                blur_plane_reduced(plane, w, h, sigma, factor);
            } else {
                blur_plane(plane, w, h, box_r);
            }
        }
        self.mark_painted(bounds);
        let float_edges = edges.map(|edge| edge as f32);
        for y in bounds.y0..bounds.y1 {
            let (left, right) = Self::rounded_span(float_edges, radius_px, y);
            let first = left.floor().max(bounds.x0 as f32) as usize;
            let last = right.ceil().min(bounds.x1 as f32) as usize;
            // Pixels fully inside the span have coverage exactly 1; only its two ends need the
            // fractional computation.
            let inner_from = (left.ceil().max(first as f32) as usize).min(last);
            let inner_to = (right.floor().min(last as f32) as usize).max(inner_from);
            let mix_inner = 1.0_f32 * alpha.min(1.0);
            for x in (first..inner_from).chain(inner_to..last) {
                let coverage = (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0);
                let mix = coverage * alpha.min(1.0);
                if mix <= 0.0 {
                    continue;
                }
                let blurred = (y - bounds.y0) * w + (x - bounds.x0);
                let offset = y * stride + x * 4;
                for (c, plane) in planes.iter().enumerate() {
                    let old = f32::from(self.pixels[offset + c]);
                    self.pixels[offset + c] = round_to_u8(plane[blurred] * mix + old * (1.0 - mix));
                }
                self.pixels[offset + 3] = 255;
            }
            if mix_inner > 0.0 && inner_from < inner_to {
                let row = &mut self.pixels[y * stride + inner_from * 4..y * stride + inner_to * 4];
                let blurred = (y - bounds.y0) * w + (inner_from - bounds.x0);
                let count = inner_to - inner_from;
                for (i, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    for (c, plane) in planes.iter().enumerate() {
                        let old = f32::from(pixel[c]);
                        pixel[c] = round_to_u8(
                            plane[blurred..blurred + count][i] * mix_inner
                                + old * (1.0 - mix_inner),
                        );
                    }
                    pixel[3] = 255;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Color;

    #[test]
    fn radius_follows_sigma() {
        assert_eq!(box_radius(0.0), 0);
        assert_eq!(box_radius(10.0), 10);
        assert!(box_radius(30.0) > box_radius(10.0));
    }

    #[test]
    fn box_line_matches_a_clamped_reference_for_every_radius() {
        let src: Vec<f32> = (0..13).map(|i| ((i * 29) % 17) as f32).collect();
        for radius in [1usize, 2, 5, 6, 7, 30] {
            let mut fast = src.clone();
            box_line(&mut fast, &mut Vec::new(), radius);
            let r = radius as isize;
            let mut sum: f32 = (-r..=r).map(|k| src[k.clamp(0, 12) as usize]).sum();
            for i in 0..13isize {
                assert_eq!(
                    fast[i as usize],
                    sum / (2 * radius + 1) as f32,
                    "r={radius} i={i}"
                );
                sum += src[(i + r + 1).min(12) as usize] - src[(i - r).max(0) as usize];
            }
        }
    }

    #[test]
    fn lockstep_lines_match_single_lines_bit_for_bit() {
        let rows: Vec<Vec<f32>> = (0..4)
            .map(|r| {
                (0..37)
                    .map(|i| ((i * 31 + r * 17) % 255) as f32 * 0.37)
                    .collect()
            })
            .collect();
        for radius in [1, 3, 18, 40] {
            let mut single = rows.clone();
            for row in &mut single {
                box_line(row, &mut Vec::new(), radius);
            }
            let mut group = rows.clone();
            let [a, b, c, d] = group.as_mut_slice() else {
                unreachable!()
            };
            box_lines([a, b, c, d], &mut Vec::new(), radius);
            assert_eq!(single, group, "radius {radius}");
        }
    }

    #[test]
    fn box_radii_match_the_requested_sigma_closely() {
        for sigma in [8.0_f32, 9.0, 10.0, 12.0, 18.0, 36.0] {
            // Variance of a box of width w is (w^2 - 1) / 12; passes add.
            let variance: f32 = box_radii(sigma)
                .iter()
                .map(|&r| ((2 * r + 1) as f32).powi(2) / 12.0 - 1.0 / 12.0)
                .sum();
            let error = (variance.sqrt() / sigma - 1.0).abs();
            assert!(error < 0.03, "sigma {sigma}: {error}");
        }
    }

    #[test]
    fn reduced_blur_stays_within_three_levels_of_the_full_size_blur() {
        for sigma in [16.0_f32, 36.0, 64.0] {
            let (w, h) = (sigma as usize * 8, sigma as usize * 5);
            let image: Vec<f32> = (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    let inside = (w / 6..w / 2).contains(&x) && (h / 5..h * 2 / 3).contains(&y);
                    let edge = if inside { 255.0 } else { 20.0 };
                    edge + (x as f32 * 0.4)
                })
                .collect();
            let factor = reduction(sigma, w, h);
            assert!(factor > 1, "sigma {sigma} should be reduced");
            let mut exact = image.clone();
            blur_plane(&mut exact, w, h, box_radius(sigma));
            let mut reduced = image.clone();
            blur_plane_reduced(&mut reduced, w, h, sigma, factor);
            let worst = exact
                .iter()
                .zip(&reduced)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            // A 235-level step edge is the harshest input; photographic content stays near 2.
            assert!(worst <= 3.0, "sigma {sigma}: worst error {worst}");
        }
    }

    #[test]
    fn narrow_blurs_are_not_reduced() {
        assert_eq!(reduction(15.9, 1000, 1000), 1);
        assert_eq!(reduction(16.0, 1000, 1000), 2);
        assert_eq!(reduction(32.0, 1000, 1000), 4);
        assert_eq!(reduction(64.0, 1000, 1000), 4);
        assert_eq!(reduction(200.0, 2000, 2000), 4);
        // A region narrower than two sigmas keeps the full-size blur.
        assert_eq!(reduction(64.0, 1000, 100), 1);
    }

    #[test]
    fn blur_keeps_flat_areas_and_spreads_edges() {
        let mut flat = vec![7.0_f32; 16];
        blur_plane(&mut flat, 4, 4, 2);
        assert!(flat.iter().all(|v| (v - 7.0).abs() < 1e-3));
        let mut line = vec![0.0, 0.0, 100.0, 0.0, 0.0];
        let mut tmp = Vec::new();
        box_line(&mut line, &mut tmp, 1);
        assert!((line[1] - 100.0 / 3.0).abs() < 1e-3);
        assert!((line[2] - 100.0 / 3.0).abs() < 1e-3);
        assert_eq!(line[0], 0.0);
    }

    fn naive_vertical(src: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
        let mut out = vec![0.0; src.len()];
        for y in 0..height {
            for x in 0..width {
                let total: f32 = (-(radius as isize)..=radius as isize)
                    .map(|k| {
                        let row = (y as isize + k).clamp(0, height as isize - 1) as usize;
                        src[row * width + x]
                    })
                    .sum();
                out[y * width + x] = total / (2 * radius + 1) as f32;
            }
        }
        out
    }

    #[test]
    fn the_sliding_vertical_pass_matches_a_naive_box_blur() {
        let (w, h) = (7, 11);
        let src: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 101) as f32).collect();
        for radius in [1, 2, 5, 20] {
            let mut dst = vec![0.0; src.len()];
            blur_vertical(&src, &mut dst, w, h, radius, &mut Vec::new());
            let expected = naive_vertical(&src, w, h, radius);
            for (got, want) in dst.iter().zip(&expected) {
                assert!(
                    (got - want).abs() < 1e-2,
                    "radius {radius}: {got} vs {want}"
                );
            }
        }
    }

    fn white_left_half() -> Surface {
        let mut surface = Surface::filled(40, 20, 1.0, Color::BLACK);
        let left = Rect {
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        surface.fill(left, Color::WHITE, 1.0, 0.0);
        surface
    }

    fn red(surface: &Surface, x: u32, y: u32) -> u8 {
        surface.pixels[((y * surface.width + x) * 4) as usize]
    }

    #[test]
    fn blur_softens_an_edge_inside_the_box_and_leaves_the_rest_alone() {
        let mut surface = white_left_half();
        let region = Rect {
            x: 10.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        surface.backdrop_blur(region, 0.0, 3.0, 1.0);
        // Either side of the edge at x = 20 moves towards the middle.
        assert!(
            (1..255).contains(&red(&surface, 19, 10)),
            "{}",
            red(&surface, 19, 10)
        );
        assert!(
            (1..255).contains(&red(&surface, 20, 10)),
            "{}",
            red(&surface, 20, 10)
        );
        assert!(red(&surface, 19, 10) > red(&surface, 20, 10));
        // Outside the box nothing changes.
        assert_eq!(red(&surface, 5, 10), 255);
        assert_eq!(red(&surface, 35, 10), 0);
        // Flat areas inside the box stay flat, and the alpha channel stays opaque.
        assert_eq!(red(&surface, 10, 10), 255);
        assert_eq!(
            surface.pixels[((10 * surface.width + 20) * 4 + 3) as usize],
            255
        );
    }

    #[test]
    fn blur_is_a_no_op_without_alpha_or_radius() {
        let original = white_left_half().pixels;
        let region = Rect {
            x: 10.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        let mut hidden = white_left_half();
        hidden.backdrop_blur(region, 0.0, 3.0, 0.0);
        assert_eq!(hidden.pixels, original);
        let mut sharp = white_left_half();
        sharp.backdrop_blur(region, 0.0, 0.0, 1.0);
        assert_eq!(sharp.pixels, original);
    }

    #[test]
    fn partial_alpha_mixes_the_blur_with_the_original() {
        let region = Rect {
            x: 10.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        let mut full = white_left_half();
        full.backdrop_blur(region, 0.0, 3.0, 1.0);
        let mut half = white_left_half();
        half.backdrop_blur(region, 0.0, 3.0, 0.5);
        let (f, h) = (i32::from(red(&full, 19, 10)), i32::from(red(&half, 19, 10)));
        assert!(h > f && h < 255, "{f} < {h} < 255");
    }

    #[test]
    fn rounded_corners_are_left_unblurred() {
        let mut surface = Surface::filled(40, 40, 1.0, Color::BLACK);
        let stripe = Rect {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 5.0,
        };
        surface.fill(stripe, Color::WHITE, 1.0, 0.0);
        let before = red(&surface, 10, 0);
        // A 30 px box with 15 px radius is a circle: its corner pixel is outside the shape.
        let region = Rect {
            x: 10.0,
            y: 0.0,
            w: 30.0,
            h: 30.0,
        };
        surface.backdrop_blur(region, 15.0, 4.0, 1.0);
        assert_eq!(red(&surface, 10, 0), before);
        assert!(red(&surface, 25, 4) < 255);
    }
}
