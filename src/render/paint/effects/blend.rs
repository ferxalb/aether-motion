//! `mix-blend-mode` compositing of a premultiplied layer pixel over an opaque frame pixel.
//!
//! Separable blend functions follow W3C Compositing and Blending Level 1 in sRGB space. With an
//! opaque backdrop the result is `(1 - αs) * Cb + αs * B(Cb, Cs)`; `plus-lighter` adds the
//! premultiplied source and clamps. `normal` keeps the exact rounding of plain source-over.

use crate::model::BlendMode;
use crate::render::geometry::PixelRect;
use crate::render::paint::float_ops::round_to_u8;

fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb * 2.0 * cs
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 {
            ((16.0 * cb - 12.0) * cb + 4.0) * cb
        } else {
            cb.sqrt()
        };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

fn color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

fn color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

/// A premultiplied layer: four planes (red, green, blue, alpha, each 0..=255) of `width` columns
/// whose top-left pixel sits at `origin` in frame pixels.
pub(in crate::render) struct Layer<'p> {
    pub(in crate::render) planes: &'p [Vec<f32>],
    pub(in crate::render) width: usize,
    pub(in crate::render) origin: (i32, i32),
}

/// Composites `layer` onto the opaque RGBA `frame` (`stride` pixels per row) inside `bounds`.
pub(in crate::render) fn composite_layer(
    mode: BlendMode,
    frame: &mut [u8],
    stride: usize,
    bounds: PixelRect,
    layer: &Layer,
) {
    // One monomorphized loop per mode keeps the blend function inlined in the pixel loop.
    match mode {
        BlendMode::Normal => run(frame, stride, bounds, layer, |cb, s, a| {
            let keep = 1.0 - a;
            [0, 1, 2].map(|c| round_to_u8(s[c] + cb[c] * keep))
        }),
        BlendMode::PlusLighter => run(frame, stride, bounds, layer, |cb, s, _| {
            [0, 1, 2].map(|c| round_to_u8(s[c] + cb[c]))
        }),
        BlendMode::Multiply => separable(frame, stride, bounds, layer, |cb, cs| cb * cs),
        BlendMode::Screen => separable(frame, stride, bounds, layer, |cb, cs| cb + cs - cb * cs),
        BlendMode::Overlay => separable(frame, stride, bounds, layer, |cb, cs| hard_light(cs, cb)),
        BlendMode::Darken => separable(frame, stride, bounds, layer, f32::min),
        BlendMode::Lighten => separable(frame, stride, bounds, layer, f32::max),
        BlendMode::ColorDodge => separable(frame, stride, bounds, layer, color_dodge),
        BlendMode::ColorBurn => separable(frame, stride, bounds, layer, color_burn),
        BlendMode::HardLight => separable(frame, stride, bounds, layer, hard_light),
        BlendMode::SoftLight => separable(frame, stride, bounds, layer, soft_light),
        BlendMode::Difference => separable(frame, stride, bounds, layer, |cb, cs| (cb - cs).abs()),
        BlendMode::Exclusion => separable(frame, stride, bounds, layer, |cb, cs| {
            cb + cs - 2.0 * cb * cs
        }),
    }
}

/// `(1 - αs) * Cb + αs * B(Cb, Cs)` on values in 0..=1, written back as 8-bit values.
#[inline(always)]
fn separable(
    frame: &mut [u8],
    stride: usize,
    bounds: PixelRect,
    layer: &Layer,
    blend: impl Fn(f32, f32) -> f32,
) {
    run(frame, stride, bounds, layer, |cb, s, a| {
        let unpremultiply = 1.0 / (255.0 * a);
        let keep = 1.0 - a;
        [0, 1, 2].map(|c| {
            let b = cb[c] * (1.0 / 255.0);
            let cs = (s[c] * unpremultiply).clamp(0.0, 1.0);
            round_to_u8((keep * b + a * blend(b, cs)) * 255.0)
        })
    })
}

/// Visits every covered pixel: `pixel(backdrop, premultiplied source, alpha)` gives its color,
/// with backdrop and source channels in 0..=255 and alpha in 0..=1.
#[inline(always)]
fn run(
    frame: &mut [u8],
    stride: usize,
    bounds: PixelRect,
    layer: &Layer,
    pixel: impl Fn([f32; 3], [f32; 3], f32) -> [u8; 3],
) {
    let [r, g, b, alpha] = layer.planes else {
        return;
    };
    let (x0, y0) = layer.origin;
    let count = bounds.x1 - bounds.x0;
    for y in bounds.y0..bounds.y1 {
        let first = (y as i32 - y0) as usize * layer.width + (bounds.x0 as i32 - x0) as usize;
        let span = first..first + count;
        let dest = &mut frame[(y * stride + bounds.x0) * 4..(y * stride + bounds.x1) * 4];
        let source = r[span.clone()]
            .iter()
            .zip(&g[span.clone()])
            .zip(&b[span.clone()])
            .zip(&alpha[span]);
        for (px, (((&sr, &sg), &sb), &sa)) in dest.as_chunks_mut::<4>().0.iter_mut().zip(source) {
            if sa <= 0.0 {
                continue;
            }
            let backdrop = [px[0], px[1], px[2]].map(f32::from);
            let [nr, ng, nb] = pixel(backdrop, [sr, sg, sb], sa / 255.0);
            *px = [nr, ng, nb, 255];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over(mode: BlendMode, dest: [u8; 3], source: [u8; 3], alpha: f32) -> [u8; 3] {
        let mut px = [dest[0], dest[1], dest[2], 255];
        let mut planes: Vec<Vec<f32>> =
            source.iter().map(|&c| vec![f32::from(c) * alpha]).collect();
        planes.push(vec![alpha * 255.0]);
        let layer = Layer {
            planes: &planes,
            width: 1,
            origin: (0, 0),
        };
        let bounds = PixelRect {
            x0: 0,
            y0: 0,
            x1: 1,
            y1: 1,
        };
        composite_layer(mode, &mut px, 1, bounds, &layer);
        [px[0], px[1], px[2]]
    }

    #[test]
    fn separable_modes_match_their_formulas() {
        let (gray, red) = ([128, 128, 128], [255, 0, 0]);
        assert_eq!(over(BlendMode::Multiply, gray, red, 1.0), [128, 0, 0]);
        assert_eq!(over(BlendMode::Screen, gray, red, 1.0), [255, 128, 128]);
        assert_eq!(over(BlendMode::Difference, gray, red, 1.0), [127, 128, 128]);
        assert_eq!(over(BlendMode::Darken, gray, red, 1.0), [128, 0, 0]);
        assert_eq!(over(BlendMode::Lighten, gray, red, 1.0), [255, 128, 128]);
        assert_eq!(
            over(BlendMode::PlusLighter, gray, red, 1.0),
            [255, 128, 128]
        );
        // Half coverage mixes the blended color with the untouched backdrop.
        assert_eq!(
            over(BlendMode::Multiply, [200, 200, 200], [0, 0, 0], 0.5),
            [100, 100, 100]
        );
    }

    #[test]
    fn white_and_black_are_neutral_where_the_spec_says() {
        let c = [10, 120, 240];
        assert_eq!(over(BlendMode::Multiply, c, [255, 255, 255], 1.0), c);
        assert_eq!(over(BlendMode::Screen, c, [0, 0, 0], 1.0), c);
        assert_eq!(over(BlendMode::SoftLight, c, [128, 128, 128], 1.0)[1], 120);
    }
}
