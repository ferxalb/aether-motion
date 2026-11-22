//! `mix-blend-mode` compositing of a premultiplied layer pixel over an opaque frame pixel.
//!
//! Separable and non-separable (`hue`, `saturation`, `color`, `luminosity`) blend functions
//! follow W3C Compositing and Blending Level 1 in sRGB space. With an opaque backdrop the result
//! is `(1 - αs) * Cb + αs * B(Cb, Cs)`; `plus-lighter` adds the premultiplied source and clamps. `normal` keeps the exact rounding of plain source-over.
//! Texels come from blurred planes or straight from the two drawing passes (`Texels`), with
//! identical arithmetic either way.

use crate::model::BlendMode;
use crate::render::geometry::PixelRect;
use crate::render::paint::clip::premultiplied_pixel;
use crate::render::paint::effects::color::ColorChain;
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

/// `Lum(C)`: the luminosity weights of Compositing Level 1.
fn lum([r, g, b]: [f32; 3]) -> f32 {
    0.3 * r + 0.59 * g + 0.11 * b
}

/// `ClipColor(C)`: brings out-of-range channels back into 0..=1 keeping the luminosity.
fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c
}

/// `SetLum(C, l)`.
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

/// `Sat(C)`.
fn sat([r, g, b]: [f32; 3]) -> f32 {
    r.max(g).max(b) - r.min(g).min(b)
}

/// `SetSat(C, s)`: scales the channels between the smallest and the largest to saturation `s`.
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    // Indices of the smallest, middle, and largest channels; ties keep the channel order.
    let mut order = [0_usize, 1, 2];
    order.sort_by(|&i, &j| c[i].total_cmp(&c[j]));
    let [min, mid, max] = order;
    let mut out = [0.0_f32; 3];
    if c[max] > c[min] {
        out[mid] = (c[mid] - c[min]) * s / (c[max] - c[min]);
        out[max] = s;
    }
    out
}

/// A premultiplied layer: four planes (red, green, blue, alpha, each 0..=255) of `width` columns
/// whose top-left pixel sits at `origin` in frame pixels.
pub(in crate::render) struct Layer<'p> {
    pub(in crate::render) texels: Texels<'p>,
    pub(in crate::render) width: usize,
    pub(in crate::render) origin: (i32, i32),
}

/// Where a layer's premultiplied texels come from.
pub(in crate::render) enum Texels<'p> {
    /// Four planes (red, green, blue, alpha), after blur or drop shadow.
    Planes(&'p [Vec<f32>]),
    /// The black and white drawing passes, recovered and recolored on the fly (no blur or shadow).
    Passes {
        black: &'p [u8],
        white: &'p [u8],
        colors: &'p ColorChain,
    },
}

impl Layer<'_> {
    /// Replaces `out` with the `count` texels starting at layer index `first`.
    #[inline(always)]
    pub(in crate::render) fn fill_row(&self, first: usize, count: usize, out: &mut Vec<[f32; 4]>) {
        out.clear();
        let span = first..first + count;
        match self.texels {
            Texels::Planes(planes) => {
                let [r, g, b, a] = planes else {
                    return;
                };
                out.extend(
                    r[span.clone()]
                        .iter()
                        .zip(&g[span.clone()])
                        .zip(&b[span.clone()])
                        .zip(&a[span])
                        .map(|(((&r, &g), &b), &a)| [r, g, b, a]),
                );
            }
            Texels::Passes {
                black,
                white,
                colors,
            } => {
                let bytes = span.start * 4..span.end * 4;
                out.extend(
                    black[bytes.clone()]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(white[bytes].as_chunks::<4>().0)
                        .map(|(b, w)| {
                            let texel = premultiplied_pixel(b, w).map(f32::from);
                            if texel[3] > 0.0 {
                                colors.recolor(texel)
                            } else {
                                texel
                            }
                        }),
                );
            }
        }
    }
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
        BlendMode::Hue => non_separable(frame, stride, bounds, layer, |cb, cs| {
            set_lum(set_sat(cs, sat(cb)), lum(cb))
        }),
        BlendMode::Saturation => non_separable(frame, stride, bounds, layer, |cb, cs| {
            set_lum(set_sat(cb, sat(cs)), lum(cb))
        }),
        BlendMode::Color => {
            non_separable(frame, stride, bounds, layer, |cb, cs| set_lum(cs, lum(cb)))
        }
        BlendMode::Luminosity => {
            non_separable(frame, stride, bounds, layer, |cb, cs| set_lum(cb, lum(cs)))
        }
    }
}

/// Like [`separable`], with a blend function of the whole color: `B(Cb, Cs)` on values in 0..=1.
#[inline(always)]
fn non_separable(
    frame: &mut [u8],
    stride: usize,
    bounds: PixelRect,
    layer: &Layer,
    blend: impl Fn([f32; 3], [f32; 3]) -> [f32; 3],
) {
    run(frame, stride, bounds, layer, |cb, s, a| {
        let unpremultiply = 1.0 / (255.0 * a);
        let keep = 1.0 - a;
        let b = cb.map(|v| v * (1.0 / 255.0));
        let cs = s.map(|v| (v * unpremultiply).clamp(0.0, 1.0));
        let mixed = blend(b, cs);
        [0, 1, 2].map(|c| round_to_u8((keep * b[c] + a * mixed[c].clamp(0.0, 1.0)) * 255.0))
    })
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
    let (x0, y0) = layer.origin;
    let count = bounds.x1 - bounds.x0;
    let mut texels = Vec::with_capacity(count);
    for y in bounds.y0..bounds.y1 {
        let first = (y as i32 - y0) as usize * layer.width + (bounds.x0 as i32 - x0) as usize;
        layer.fill_row(first, count, &mut texels);
        let dest = &mut frame[(y * stride + bounds.x0) * 4..(y * stride + bounds.x1) * 4];
        for (px, &[sr, sg, sb, sa]) in dest.as_chunks_mut::<4>().0.iter_mut().zip(&texels) {
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
            texels: Texels::Planes(&planes),
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
    fn non_separable_modes_mix_hue_saturation_and_luminosity() {
        let (gray, red) = ([128, 128, 128], [255, 0, 0]);
        // Gray has no saturation, so hue and saturation over it keep it gray.
        assert_eq!(over(BlendMode::Hue, gray, red, 1.0), gray);
        assert_eq!(over(BlendMode::Saturation, gray, red, 1.0), gray);
        // Color takes the source's hue and saturation at the backdrop's luminosity.
        let color = over(BlendMode::Color, gray, red, 1.0);
        let expected =
            set_lum([1.0, 0.0, 0.0], lum([128.0 / 255.0; 3])).map(|v| (v * 255.0).round() as u8);
        assert_eq!(color, expected);
        assert!(color[0] > color[1] && color[1] == color[2], "{color:?}");
        // Luminosity takes the source's luminosity: red (Lum 0.3) over gray darkens it.
        assert_eq!(over(BlendMode::Luminosity, gray, red, 1.0), [77, 77, 77]);
        // Color and luminosity swap roles: the luminosity of a gray source over red keeps red's
        // hue at the gray's luminosity, as color of red over that gray does.
        assert_eq!(
            over(BlendMode::Luminosity, red, gray, 1.0),
            over(BlendMode::Color, gray, red, 1.0)
        );
        // Half coverage mixes with the backdrop.
        let half = over(BlendMode::Luminosity, gray, red, 0.5);
        assert_eq!(half, [102, 102, 102]);
    }

    #[test]
    fn set_sat_and_clip_color_follow_the_spec() {
        let scaled = set_sat([0.2, 0.6, 0.4], 0.5);
        for (got, want) in scaled.iter().zip([0.0, 0.5, 0.25]) {
            assert!((got - want).abs() < 1e-6, "{scaled:?}");
        }
        assert_eq!(set_sat([0.3, 0.3, 0.3], 0.8), [0.0; 3]);
        let clipped = clip_color([1.2, 0.5, 0.1]);
        assert!(
            clipped.iter().all(|v| (0.0..=1.0 + 1e-6).contains(v)),
            "{clipped:?}"
        );
        assert!((lum(clipped) - lum([1.2, 0.5, 0.1])).abs() < 1e-5);
    }

    #[test]
    fn white_and_black_are_neutral_where_the_spec_says() {
        let c = [10, 120, 240];
        assert_eq!(over(BlendMode::Multiply, c, [255, 255, 255], 1.0), c);
        assert_eq!(over(BlendMode::Screen, c, [0, 0, 0], 1.0), c);
        assert_eq!(over(BlendMode::SoftLight, c, [128, 128, 128], 1.0)[1], 120);
    }
}
