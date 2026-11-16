//! Fast triangle-filter resize for video frames: two separable passes over 8-bit RGBA with 16.16
//! fixed-point weights, so the result is exact integer math (deterministic on every machine).
//! `imageops::resize` produces nearly the same picture but takes several times longer, which
//! matters here because a video frame is resized on every output frame.
use image::RgbaImage;

/// Contributing source range and weights for each output coordinate along one axis.
struct Taps {
    /// `(first source index, number of taps)` per output coordinate.
    spans: Vec<(usize, usize)>,
    /// Weights (sum 65536 per coordinate), stored back to back in `spans` order.
    weights: Vec<i32>,
}

fn taps(source: u32, target: u32) -> Taps {
    let ratio = f64::from(source) / f64::from(target);
    // Triangle kernel; it widens when shrinking so every source pixel contributes.
    let scale = ratio.max(1.0);
    let mut spans = Vec::with_capacity(target as usize);
    let mut weights = Vec::new();
    for i in 0..target {
        let center = (f64::from(i) + 0.5) * ratio;
        let first = ((center - scale).floor().max(0.0)) as usize;
        let end = (((center + scale).ceil()) as usize)
            .min(source as usize)
            .max(first + 1);
        let raw: Vec<f64> = (first..end)
            .map(|j| (1.0 - ((j as f64 + 0.5 - center) / scale).abs()).max(0.0))
            .collect();
        let total: f64 = raw.iter().sum();
        let mut fixed: Vec<i32> = if total > 0.0 {
            raw.iter()
                .map(|w| (w / total * 65536.0).round() as i32)
                .collect()
        } else {
            // Degenerate window: nearest source pixel.
            let mut single = vec![0; end - first];
            single[0] = 65536;
            single
        };
        // Rounding must not change overall brightness: put the remainder on the largest weight.
        let drift = 65536 - fixed.iter().sum::<i32>();
        if let Some(largest) = (0..fixed.len()).max_by_key(|&k| fixed[k]) {
            fixed[largest] += drift;
        }
        spans.push((first, end - first));
        weights.extend(fixed);
    }
    Taps { spans, weights }
}

/// Resizes `image` to `w` x `h` (both at least 1).
pub(in crate::render) fn resize_triangle(image: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (sw, sh) = (image.width() as usize, image.height() as usize);
    let (dw, dh) = (w as usize, h as usize);
    let source = image.as_raw();

    // Horizontal pass: sw x sh -> dw x sh.
    let columns = taps(image.width(), w);
    let mut middle = vec![0_u8; dw * sh * 4];
    for y in 0..sh {
        let row = &source[y * sw * 4..(y + 1) * sw * 4];
        let out = &mut middle[y * dw * 4..(y + 1) * dw * 4];
        let mut at = 0;
        for (x, &(first, count)) in columns.spans.iter().enumerate() {
            let mut acc = [32768_i32; 4];
            for (k, &weight) in columns.weights[at..at + count].iter().enumerate() {
                let p = &row[(first + k) * 4..(first + k) * 4 + 4];
                for c in 0..4 {
                    acc[c] += weight * i32::from(p[c]);
                }
            }
            at += count;
            for c in 0..4 {
                out[x * 4 + c] = (acc[c] >> 16).clamp(0, 255) as u8;
            }
        }
    }

    // Vertical pass: dw x sh -> dw x dh.
    let rows = taps(image.height(), h);
    let mut result = vec![0_u8; dw * dh * 4];
    let stride = dw * 4;
    let mut at = 0;
    for (y, &(first, count)) in rows.spans.iter().enumerate() {
        let out = &mut result[y * stride..(y + 1) * stride];
        let mut acc = vec![32768_i32; stride];
        for (k, &weight) in rows.weights[at..at + count].iter().enumerate() {
            let line = &middle[(first + k) * stride..(first + k + 1) * stride];
            for (a, &p) in acc.iter_mut().zip(line) {
                *a += weight * i32::from(p);
            }
        }
        at += count;
        for (o, a) in out.iter_mut().zip(&acc) {
            *o = (a >> 16).clamp(0, 255) as u8;
        }
    }
    RgbaImage::from_raw(w, h, result).expect("buffer has w * h * 4 bytes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{imageops, Rgba};

    fn gradient(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([
                (x * 255 / w) as u8,
                (y * 255 / h) as u8,
                ((x ^ y) * 7 % 256) as u8,
                255,
            ])
        })
    }

    fn worst_difference(a: &RgbaImage, b: &RgbaImage) -> i32 {
        a.as_raw()
            .iter()
            .zip(b.as_raw())
            .map(|(&p, &q)| (i32::from(p) - i32::from(q)).abs())
            .max()
            .unwrap()
    }

    #[test]
    fn matches_the_image_crate_filter_closely() {
        let source = gradient(97, 61);
        for (w, h) in [(48, 30), (97, 61), (150, 90), (33, 61), (200, 20)] {
            let ours = resize_triangle(&source, w, h);
            let reference = imageops::resize(&source, w, h, imageops::FilterType::Triangle);
            assert_eq!(ours.dimensions(), (w, h));
            let worst = worst_difference(&ours, &reference);
            assert!(worst <= 6, "{w}x{h}: worst channel difference {worst}");
        }
    }

    #[test]
    fn flat_pictures_stay_flat() {
        let flat = RgbaImage::from_pixel(40, 30, Rgba([10, 200, 77, 255]));
        let out = resize_triangle(&flat, 13, 7);
        assert!(out.pixels().all(|p| *p == Rgba([10, 200, 77, 255])));
    }
}
