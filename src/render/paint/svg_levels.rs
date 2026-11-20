//! Zoom-aware reuse of SVG rasters: while an element's scale is animating, its vector artwork is
//! rasterized at a few fixed "level" sizes (a geometric ladder, never below the drawn size) and each
//! frame's exact fitted size is resampled down from the nearest level above it.
//!
//! Scope: the level choice ([`level_size`]) and the resampler ([`resample_from_level`]). The image
//! cache (`images.rs`) decides when the ladder applies and stores levels and results; levels are
//! ordinary fitted rasters, so a level is shared with an exact draw at that size.
//!
//! The resampler is a separable Catmull-Rom filter (widened by the reduction ratio, at most one
//! ladder step) on premultiplied samples in integer fixed point, so it never tints transparent
//! edges and returns the same bits on every machine. Source rows are read once, in order, through a
//! row callback (packed levels unpack band by band) into a small ring of filtered rows, so scratch
//! memory is a few output rows rather than a second copy of the image.

use super::kernels;
use image::RgbaImage;

/// Ladder steps per doubling of scale. Each step is 2^(1/8) ≈ 1.09, so a frame is resampled down
/// by at most 9%: the filter reduces detail far less than a 2x mip step would.
pub(super) const LEVELS_PER_OCTAVE: f64 = 8.0;

/// Fixed-point bits of each filter weight (weights of one output sample sum to `1 << WEIGHT_BITS`).
/// The horizontal pass keeps `MIDDLE_BITS` fractional bits (its shift is in `kernels.rs`), so every
/// intermediate sum stays far inside `i32`.
const WEIGHT_BITS: u32 = 14;
const MIDDLE_BITS: u32 = kernels::RESAMPLE_MIDDLE_BITS;

/// Size of the ladder level for an image drawn `size` output px wide and tall (unrounded) at CSS
/// scale `scale` (keyframes, CSS animation, and camera zoom combined), whose exact fit is `target`.
/// Levels sit at `2^(k / LEVELS_PER_OCTAVE)` times the unscaled box, so nearby scales share one
/// raster; the level is computed from the unrounded box, so it does not flicker between two sizes
/// as the rounded target changes, and it is never smaller than the target on either axis.
pub(super) fn level_size(size: [f32; 2], scale: f32, target: [u32; 2]) -> [u32; 2] {
    let scale = f64::from(scale);
    if !scale.is_finite() || scale <= 0.0 {
        return target;
    }
    // Snap first so a scale a rounding error above a level does not jump to the next one.
    let steps = (scale.log2() * LEVELS_PER_OCTAVE * 1e6).round() / 1e6;
    let grow = (steps.ceil() / LEVELS_PER_OCTAVE).exp2() / scale;
    let level = |n: f32, t: u32| ((f64::from(n) * grow).round() as u32).max(t);
    [level(size[0], target[0]), level(size[1], target[1])]
}

/// Receives one source row: its index, its straight RGBA pixels, and the span `[first, last)`
/// outside which it is fully transparent.
pub(super) type RowSink<'a> = dyn FnMut(usize, &[u8], (usize, usize)) + 'a;

/// Catmull-Rom cubic (B = 0, C = 1/2): interpolating, so an unscaled pass is the identity.
fn catmull_rom(x: f64) -> f64 {
    let x = x.abs();
    if x < 1.0 {
        1.5 * x * x * x - 2.5 * x * x + 1.0
    } else if x < 2.0 {
        -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}

/// Fixed-length filter windows along one axis: output coordinate `i` reads source `starts[i]..`
/// `starts[i] + taps` with weights `weights[i * taps..][..taps]` (zero-padded; windows at the edge
/// are shifted inward so they stay inside the source). `starts` never decreases.
struct Taps {
    starts: Vec<usize>,
    weights: Vec<i32>,
    taps: usize,
}

fn taps(source: u32, target: u32) -> Taps {
    let ratio = f64::from(source) / f64::from(target);
    let scale = ratio.max(1.0);
    let support = 2.0 * scale;
    let one = 1_i32 << WEIGHT_BITS;
    let n = source as usize;
    let mut windows = Vec::with_capacity(target as usize);
    for i in 0..target {
        let center = (f64::from(i) + 0.5) * ratio;
        let first = (center - support).floor().max(0.0) as usize;
        let end = ((center + support).ceil() as usize).min(n).max(first + 1);
        let raw: Vec<f64> = (first..end)
            .map(|j| catmull_rom((j as f64 + 0.5 - center) / scale))
            .collect();
        let total: f64 = raw.iter().sum();
        let mut fixed: Vec<i32> = if total.abs() > 1e-9 {
            raw.iter()
                .map(|w| (w / total * f64::from(one)).round() as i32)
                .collect()
        } else {
            let mut single = vec![0; end - first];
            single[0] = one;
            single
        };
        // Rounding must not change overall brightness: the remainder goes on the largest weight.
        let drift = one - fixed.iter().sum::<i32>();
        if let Some(largest) = (0..fixed.len()).max_by_key(|&k| fixed[k]) {
            fixed[largest] += drift;
        }
        windows.push((first, fixed));
    }
    let taps = windows
        .iter()
        .map(|(_, w)| w.len())
        .max()
        .unwrap_or(1)
        .min(n);
    let mut starts = Vec::with_capacity(windows.len());
    let mut weights = vec![0; windows.len() * taps];
    for (i, (first, fixed)) in windows.into_iter().enumerate() {
        let start = first.min(n - taps);
        let offset = first - start;
        weights[i * taps + offset..i * taps + offset + fixed.len()].copy_from_slice(&fixed);
        starts.push(start);
    }
    Taps {
        starts,
        weights,
        taps,
    }
}

/// Resamples a straight-alpha `source` (`sw` x `sh`) to a `w` x `h` straight-alpha image.
/// `for_each_row` must call its argument with every source row in order, together with the span
/// `[first, last)` outside which the row is fully transparent (`(0, sw)` when unknown). Colors are
/// filtered premultiplied and clamped to a valid premultiplied pixel before they are converted back.
/// Transparent pixels contribute exactly zero, so spans and rows without any are skipped.
pub(super) fn resample_from_level(
    [sw, sh]: [u32; 2],
    [w, h]: [u32; 2],
    for_each_row: impl FnOnce(&mut RowSink<'_>),
) -> RgbaImage {
    let columns = taps(sw, w);
    let rows = taps(sh, h);
    let (dw, dh) = (w as usize, h as usize);
    let mut ring = Ring {
        len: rows.taps,
        rows: vec![0; rows.taps * dw * 4],
        spans: vec![(0, 0); rows.taps],
        stride: dw * 4,
    };
    let mut premultiplied = vec![0_u8; sw as usize * 4];
    let mut out = vec![0_u8; dw * dh * 4];
    let mut acc = vec![0_i32; dw * 4];
    let mut next_out = 0;
    let k = columns.taps;
    let mut process = |y: usize, row: &[u8], (first, last): (usize, usize)| {
        let last = last.min(sw as usize);
        // Output columns whose window reaches a pixel in `first..last`.
        let (lo, hi) = if first < last {
            let lo = columns.starts.partition_point(|&s| s + k <= first);
            let hi = columns.starts.partition_point(|&s| s < last);
            (lo, hi.max(lo))
        } else {
            (0, 0)
        };
        let slot = y % ring.len;
        let (old_lo, old_hi) = ring.spans[slot];
        ring.slot(slot)[old_lo * 4..old_hi * 4].fill(0);
        ring.spans[slot] = (lo, hi);
        if lo < hi {
            // Pixels outside `first..last` are transparent, so they premultiply to zero.
            let (from, to) = (columns.starts[lo], columns.starts[hi - 1] + k);
            kernels::premultiply_row(&mut premultiplied[from * 4..to * 4], &row[from * 4..to * 4]);
            kernels::filter_columns(
                &mut ring.slot(slot)[lo * 4..hi * 4],
                &premultiplied,
                &columns.starts[lo..hi],
                &columns.weights[lo * k..hi * k],
                k,
            );
        }
        // Every output row whose window ends at `y` is complete now.
        while next_out < dh && rows.starts[next_out] + rows.taps - 1 == y {
            let weights = &rows.weights[next_out * rows.taps..(next_out + 1) * rows.taps];
            let dest = &mut out[next_out * dw * 4..(next_out + 1) * dw * 4];
            vertical(&ring, rows.starts[next_out], weights, &mut acc, dest);
            next_out += 1;
        }
    };
    for_each_row(&mut process);
    debug_assert_eq!(next_out, dh, "every source row was read");
    RgbaImage::from_raw(w, h, out).expect("buffer has w * h * 4 bytes")
}

/// Horizontally filtered source rows (premultiplied, `MIDDLE_BITS` fractional bits) by source row
/// modulo `len`, each with the output-column span outside which it is zero.
struct Ring {
    len: usize,
    rows: Vec<i32>,
    spans: Vec<(usize, usize)>,
    stride: usize,
}

impl Ring {
    fn slot(&mut self, slot: usize) -> &mut [i32] {
        &mut self.rows[slot * self.stride..(slot + 1) * self.stride]
    }
}

fn vertical(ring: &Ring, top: usize, weights: &[i32], acc: &mut [i32], dest: &mut [u8]) {
    let (mut lo, mut hi) = (usize::MAX, 0);
    for (k, &weight) in weights.iter().enumerate() {
        let (a, b) = ring.spans[(top + k) % ring.len];
        if weight != 0 && a < b {
            lo = lo.min(a);
            hi = hi.max(b);
        }
    }
    if lo >= hi {
        return; // Every row of the window is transparent; `dest` is already zero.
    }
    let acc = &mut acc[lo * 4..hi * 4];
    acc.fill(1 << (WEIGHT_BITS + MIDDLE_BITS - 1));
    for (k, &weight) in weights.iter().enumerate() {
        let slot = (top + k) % ring.len;
        if weight == 0 || ring.spans[slot].0 >= ring.spans[slot].1 {
            continue;
        }
        let line = &ring.rows[slot * ring.stride..(slot + 1) * ring.stride];
        kernels::accumulate_row(acc, &line[lo * 4..hi * 4], weight);
    }
    kernels::unpremultiply_row(&mut dest[lo * 4..hi * 4], acc, WEIGHT_BITS + MIDDLE_BITS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn resample(image: &RgbaImage, w: u32, h: u32) -> RgbaImage {
        let sw = image.width() as usize;
        resample_from_level([image.width(), image.height()], [w, h], |f| {
            for (y, row) in image.as_raw().chunks_exact(sw * 4).enumerate() {
                f(y, row, (0, sw));
            }
        })
    }

    #[test]
    fn levels_never_undershoot_and_are_shared_by_nearby_scales() {
        let step = (1.0 / LEVELS_PER_OCTAVE).exp2();
        let level = |box_w: f32, box_h: f32, scale: f32| {
            let size = [box_w * scale, box_h * scale];
            let target = size.map(|n| n.round().max(1.0) as u32);
            (level_size(size, scale, target), target)
        };
        for scale in [0.37_f32, 0.9, 1.0, 1.2, 1.35, 1.62, 1.92, 3.3] {
            for (w, h) in [(1.0, 1.0), (97.0, 61.0), (1996.8, 1123.2), (3840.0, 2160.0)] {
                let ([lw, lh], [tw, th]) = level(w, h, scale);
                assert!(lw >= tw && lh >= th, "{scale} {w}x{h}");
                // At most one ladder step (plus a pixel of rounding) above the target.
                assert!(f64::from(lw) <= f64::from(tw) * step + 1.0);
                assert!(f64::from(lh) <= f64::from(th) * step + 1.0);
            }
        }
        // Every scale inside one step maps to the same level, whatever the rounded target.
        let shared = level(1387.0, 607.0, 1.42).0;
        for i in 0..200 {
            let scale = 1.4143 + 0.1278 * i as f32 / 200.0;
            assert_eq!(level(1387.0, 607.0, scale).0, shared, "{scale}");
        }
        // A scale on a level keeps its own size.
        assert_eq!(level(1000.0, 500.0, 2.0).0, [2000, 1000]);
        assert_eq!(level(1000.0, 500.0, 1.0).0, [1000, 500]);
    }

    /// The same integer filter with no streaming, no span skipping, and no SIMD dispatch.
    fn reference(image: &RgbaImage, w: u32, h: u32) -> RgbaImage {
        let (sw, sh) = (image.width() as usize, image.height() as usize);
        let (columns, rows) = (taps(sw as u32, w), taps(sh as u32, h));
        let premultiplied: Vec<i64> = image
            .as_raw()
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                let a = u32::from(p[3]);
                let pm = |c: u8| i64::from((u32::from(c) * a + 127) / 255);
                [pm(p[0]), pm(p[1]), pm(p[2]), i64::from(p[3])]
            })
            .collect();
        let mut middle = vec![0_i64; w as usize * sh * 4];
        for y in 0..sh {
            for x in 0..w as usize {
                for c in 0..4 {
                    let mut acc = 1_i64 << (WEIGHT_BITS - MIDDLE_BITS - 1);
                    for k in 0..columns.taps {
                        let sx = columns.starts[x] + k;
                        acc += i64::from(columns.weights[x * columns.taps + k])
                            * premultiplied[(y * sw + sx) * 4 + c];
                    }
                    middle[(y * w as usize + x) * 4 + c] = acc >> (WEIGHT_BITS - MIDDLE_BITS);
                }
            }
        }
        let shift = WEIGHT_BITS + MIDDLE_BITS;
        RgbaImage::from_fn(w, h, |x, y| {
            let (x, y) = (x as usize, y as usize);
            let sum = |c: usize| {
                let mut acc = 1_i64 << (shift - 1);
                for k in 0..rows.taps {
                    let sy = rows.starts[y] + k;
                    acc += i64::from(rows.weights[y * rows.taps + k])
                        * middle[(sy * w as usize + x) * 4 + c];
                }
                acc >> shift
            };
            let a = sum(3).clamp(0, 255);
            if a == 0 {
                return Rgba([0; 4]);
            }
            let straight = |c: i64| ((c.clamp(0, a) * 255 + a / 2) / a).min(255) as u8;
            Rgba([
                straight(sum(0)),
                straight(sum(1)),
                straight(sum(2)),
                a as u8,
            ])
        })
    }

    #[test]
    fn streaming_with_transparent_spans_matches_the_plain_filter() {
        // Sparse artwork: transparent margins, empty rows, thin strokes, and partial alpha.
        let image = RgbaImage::from_fn(211, 97, |x, y| {
            let inside = (30..170).contains(&x) && (9..80).contains(&y) && y % 23 != 0;
            let line = x == 2 * y || (x + y) % 41 == 0;
            if inside || line {
                let a = [255, 255, 160, 23, 1][((x / 3 + y) % 5) as usize];
                Rgba([(x * 3) as u8, (y * 5) as u8, ((x * y) % 251) as u8, a])
            } else {
                Rgba([0; 4])
            }
        });
        let spans: Vec<(usize, usize)> = image
            .as_raw()
            .as_chunks::<{ 211 * 4 }>()
            .0
            .iter()
            .map(|row| {
                let row = row.as_chunks::<4>().0;
                let first = row.iter().position(|p| p[3] != 0).unwrap_or(0);
                let last = row.iter().rposition(|p| p[3] != 0).map_or(0, |i| i + 1);
                (first, last)
            })
            .collect();
        for (w, h) in [(211, 97), (200, 91), (194, 89), (1, 1), (37, 96)] {
            let streamed = resample_from_level([211, 97], [w, h], |f| {
                for (y, row) in image
                    .as_raw()
                    .as_chunks::<{ 211 * 4 }>()
                    .0
                    .iter()
                    .enumerate()
                {
                    f(y, row.as_slice(), spans[y]);
                }
            });
            assert_eq!(streamed, reference(&image, w, h), "{w}x{h}");
        }
    }

    #[test]
    fn unscaled_resample_is_exact_and_flat_areas_stay_flat() {
        let image = RgbaImage::from_fn(53, 31, |x, y| {
            let a = [0, 1, 77, 254, 255][((x * 7 + y * 3) % 5) as usize];
            Rgba([(x * 5) as u8, (y * 8) as u8, ((x ^ y) * 9) as u8, a])
        });
        let same = resample(&image, 53, 31);
        for (p, q) in same.pixels().zip(image.pixels()) {
            if q[3] == 0 {
                assert_eq!(p.0, [0; 4]);
            } else {
                // Premultiplying a straight pixel and back loses at most this much.
                let tolerance = 255 / i32::from(q[3]) + 1;
                for c in 0..3 {
                    assert!((i32::from(p[c]) - i32::from(q[c])).abs() <= tolerance);
                }
                assert_eq!(p[3], q[3]);
            }
        }
        let flat = RgbaImage::from_pixel(97, 61, Rgba([10, 200, 77, 255]));
        assert!(resample(&flat, 89, 57)
            .pixels()
            .all(|p| *p == Rgba([10, 200, 77, 255])));
        // Transparent pixels contribute no color to their neighbors.
        let edge = RgbaImage::from_fn(40, 40, |x, _| {
            if x < 20 {
                Rgba([255, 0, 0, 0])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        for p in resample(&edge, 37, 37).pixels() {
            assert_eq!(p[0], 0, "{p:?}");
        }
    }
}
