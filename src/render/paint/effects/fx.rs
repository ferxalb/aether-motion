//! Pixel effects outside CSS `filter` (see `model::FxMotion`): a tint toward a solid color, a red/blue
//! channel split, and a glitch that displaces horizontal slices. They run on the effect layer's
//! premultiplied planes (values 0..=255) after the color functions and before `drop-shadow`.
//!
//! The glitch pattern is a pure function of the element's box, the strength, and a seed that changes
//! `GLITCH_RATE` times a second, so output stays deterministic and independent of the frame rate.

use crate::model::Color;

/// The element's box inside the layer, in layer pixels: `[x0, y0, x1, y1]`, clamped to the layer.
#[derive(Clone, Copy)]
pub(super) struct Span {
    pub(super) x0: usize,
    pub(super) y0: usize,
    pub(super) x1: usize,
    pub(super) y1: usize,
}

/// Replaces `amount` of each pixel's color with `color`, keeping its alpha.
pub(super) fn apply_tint(planes: &mut [Vec<f32>], color: Color, amount: f32) {
    let [r, g, b, a] = planes else {
        return;
    };
    let keep = 1.0 - amount;
    let target = [color.r, color.g, color.b].map(|c| f32::from(c) / 255.0 * amount);
    for (((r, g), b), a) in r
        .iter_mut()
        .zip(g.iter_mut())
        .zip(b.iter_mut())
        .zip(a.iter())
    {
        if *a > 0.0 {
            *r = *r * keep + target[0] * *a;
            *g = *g * keep + target[1] * *a;
            *b = *b * keep + target[2] * *a;
        }
    }
}

/// Reads red from `offset` px to the right and blue from `offset` px to the left, so red appears
/// shifted left and blue right; alpha covers all three. Positions outside the layer are empty.
pub(super) fn apply_rgb_split(planes: &mut [Vec<f32>], width: usize, offset: f32) {
    let d = offset.round() as i64;
    if d == 0 || width == 0 {
        return;
    }
    let height = planes[0].len() / width;
    let (red, blue, alpha) = (planes[0].clone(), planes[2].clone(), planes[3].clone());
    let at = |plane: &[f32], row: usize, x: i64| -> f32 {
        if x < 0 || x >= width as i64 {
            0.0
        } else {
            plane[row + x as usize]
        }
    };
    for y in 0..height {
        let row = y * width;
        for x in 0..width {
            let (right, left) = (x as i64 + d, x as i64 - d);
            let i = row + x;
            planes[0][i] = at(&red, row, right);
            planes[2][i] = at(&blue, row, left);
            planes[3][i] = alpha[i]
                .max(at(&alpha, row, right))
                .max(at(&alpha, row, left));
        }
    }
}

/// SplitMix64: a small, well-mixed hash for the glitch pattern.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Uniform value in `[0, 1)` for `(seed, band, salt)`.
fn unit(seed: u32, band: u64, salt: u64) -> f32 {
    let h = mix(u64::from(seed) << 32 ^ band.wrapping_mul(0x1000_0001) ^ salt << 56);
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// One displaced slice: rows `[y0, y1)` of the box move `shift` px (positive is right), and red and
/// blue separate by `split` px.
#[derive(Debug, PartialEq)]
struct Slice {
    y0: usize,
    y1: usize,
    shift: i64,
    split: i64,
}

/// The slices of a glitch of `strength` over a box `w` by `h` px; rows between slices stay put.
fn slices(w: usize, h: usize, strength: f32, seed: u32) -> Vec<Slice> {
    let mut out = Vec::new();
    if w == 0 || h == 0 || strength <= 0.0 {
        return out;
    }
    let (min_h, max_h) = ((h as f32 * 0.015).max(1.0), (h as f32 * 0.12).max(2.0));
    let mut y = 0.0_f32;
    let mut band = 0_u64;
    while (y as usize) < h {
        let size = min_h + (max_h - min_h) * unit(seed, band, 1);
        let (y0, y1) = (y as usize, ((y + size) as usize).min(h));
        // Stronger glitches displace more slices, further.
        if unit(seed, band, 2) < 0.15 + 0.55 * strength {
            let direction = unit(seed, band, 3) * 2.0 - 1.0;
            let shift = (direction * strength * 0.12 * w as f32).round() as i64;
            let split = (unit(seed, band, 4) * strength * 0.012 * w as f32).round() as i64;
            if y1 > y0 && (shift != 0 || split != 0) {
                out.push(Slice {
                    y0,
                    y1,
                    shift,
                    split,
                });
            }
        }
        y += size;
        band += 1;
    }
    out
}

/// Displaces slices of the box horizontally. Pixels are read from inside the box only (the box edge
/// repeats), so a full-size scene never shows a hole at the frame edge.
pub(super) fn apply_glitch(
    planes: &mut [Vec<f32>],
    width: usize,
    span: Span,
    strength: f32,
    seed: u32,
) {
    let (w, h) = (
        span.x1.saturating_sub(span.x0),
        span.y1.saturating_sub(span.y0),
    );
    let cuts = slices(w, h, strength, seed);
    if cuts.is_empty() {
        return;
    }
    let mut row = vec![0.0_f32; w];
    for slice in &cuts {
        for y in span.y0 + slice.y0..span.y0 + slice.y1 {
            let base = y * width + span.x0;
            for (channel, plane) in planes.iter_mut().enumerate() {
                let extra = match channel {
                    0 => slice.split,
                    2 => -slice.split,
                    _ => 0,
                };
                let source = &plane[base..base + w];
                for (x, out) in row.iter_mut().enumerate() {
                    let from = (x as i64 - slice.shift + extra).clamp(0, w as i64 - 1);
                    *out = source[from as usize];
                }
                plane[base..base + w].copy_from_slice(&row);
            }
            // Keep premultiplied color within alpha after the channels moved apart.
            let [r, g, b, a] = planes else {
                return;
            };
            let range = base..base + w;
            let colors = r[range.clone()]
                .iter()
                .zip(&g[range.clone()])
                .zip(&b[range.clone()]);
            for (((r, g), b), a) in colors.zip(&mut a[range]) {
                let most = r.max(*g).max(*b);
                if most > *a {
                    *a = most.min(255.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planes(
        width: usize,
        height: usize,
        value: impl Fn(usize, usize) -> [f32; 4],
    ) -> Vec<Vec<f32>> {
        let mut out = vec![vec![0.0; width * height]; 4];
        for y in 0..height {
            for x in 0..width {
                for (c, v) in value(x, y).into_iter().enumerate() {
                    out[c][y * width + x] = v;
                }
            }
        }
        out
    }

    #[test]
    fn tint_keeps_alpha_and_reaches_the_color() {
        let mut p = planes(2, 1, |x, _| {
            if x == 0 {
                [10.0, 20.0, 30.0, 255.0]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            }
        });
        apply_tint(&mut p, Color::WHITE, 1.0);
        assert_eq!(
            [p[0][0], p[1][0], p[2][0], p[3][0]],
            [255.0, 255.0, 255.0, 255.0]
        );
        assert_eq!([p[0][1], p[3][1]], [0.0, 0.0]);
        let mut half = planes(1, 1, |_, _| [0.0, 0.0, 0.0, 255.0]);
        apply_tint(&mut half, Color::WHITE, 0.5);
        assert_eq!(half[0][0], 127.5);
    }

    #[test]
    fn split_moves_red_left_and_blue_right() {
        let mut p = planes(5, 1, |x, _| {
            if x == 2 {
                [200.0, 100.0, 50.0, 255.0]
            } else {
                [0.0; 4]
            }
        });
        apply_rgb_split(&mut p, 5, 1.0);
        assert_eq!(p[0], vec![0.0, 200.0, 0.0, 0.0, 0.0]);
        assert_eq!(p[1], vec![0.0, 0.0, 100.0, 0.0, 0.0]);
        assert_eq!(p[2], vec![0.0, 0.0, 0.0, 50.0, 0.0]);
        assert_eq!(p[3], vec![0.0, 255.0, 255.0, 255.0, 0.0]);
    }

    #[test]
    fn glitch_is_deterministic_and_grows_with_strength() {
        let a = slices(1920, 1080, 0.8, 7);
        assert_eq!(a, slices(1920, 1080, 0.8, 7));
        assert_ne!(a, slices(1920, 1080, 0.8, 8));
        assert!(slices(1920, 1080, 0.0, 7).is_empty());
        let reach = |s: &[Slice]| s.iter().map(|c| c.shift.abs()).max().unwrap_or(0);
        assert!(reach(&a) <= (0.12 * 0.8 * 1920.0_f32).round() as i64);
        assert!(a.iter().all(|c| c.y0 < c.y1 && c.y1 <= 1080));
    }

    #[test]
    fn glitch_reads_only_inside_the_box() {
        let (w, h) = (64, 64);
        let mut p = planes(w, h, |_, _| [255.0, 255.0, 255.0, 255.0]);
        let span = Span {
            x0: 0,
            y0: 0,
            x1: w,
            y1: h,
        };
        apply_glitch(&mut p, w, span, 1.0, 3);
        // A uniform opaque box stays uniform and opaque: edge pixels repeat instead of leaving holes.
        assert!(p.iter().all(|plane| plane.iter().all(|&v| v == 255.0)));
    }
}
