//! General gradient fills: angled and corner `linear-gradient`, multi-stop, translucent stops,
//! and `radial-gradient`. Evaluated per pixel with an ordered dither so dark ramps do not band.
//! The two-opaque-stop axis case stays on the faster `Surface::fill_gradient`.
//!
//! A square gradient that is drawn again with the same size (a drifting glow, for example) is
//! rasterized once into a fixed-point tile; later frames only blend that tile at the new offset.
//! Full and quarter element opacities use exact integer tile blending, preserving ordered dithering.
//! Stop colors are premultiplied once per draw, including uncached fractional-motion draws.

#[cfg(test)]
use crate::model::Color;
use crate::model::{Gradient, GradientKind, LinearDirection, Stop};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::float_ops::{hypot, round_to_u8};
use crate::render::paint::surface::Surface;
use crate::render::paint::tile_cache::TileCache;
use anyhow::Result;

/// 4x4 Bayer matrix, mapped to thresholds in (-0.5, 0.5).
const BAYER: [[f32; 4]; 4] = [
    [0.0, 8.0, 2.0, 10.0],
    [12.0, 4.0, 14.0, 6.0],
    [3.0, 11.0, 1.0, 9.0],
    [15.0, 7.0, 13.0, 5.0],
];

fn dither(x: usize, y: usize) -> f32 {
    (BAYER[y & 3][x & 3] + 0.5) / 16.0 - 0.5
}

/// At full element opacity every term in the cached float blend is an exact multiple
/// of 1/4096. The numerator stays below 2^24, so this preserves its rounding exactly.
fn blend_full_opacity(dest: &mut [[u8; 4]], source: &[[u16; 4]], first: usize, y: usize) {
    let bias = BAYER[y & 3].map(|b| (b as i32 * 2 - 15) * 128 + 2048);
    for (i, (pixel, texel)) in dest.iter_mut().zip(source).enumerate() {
        let keep = 4096 - i32::from(texel[3]);
        let d = bias[(first + i) & 3];
        for c in 0..3 {
            let numerator = i32::from(texel[c]) * 16 + i32::from(pixel[c]) * keep + d;
            pixel[c] = (numerator >> 12).clamp(0, 255) as u8;
        }
        if texel[3] > 0 {
            pixel[3] = 255;
        }
    }
}

/// Quarter opacities keep every intermediate float result exactly representable in 24 bits.
#[inline(always)]
fn blend_quarter_opacity(
    dest: &mut [[u8; 4]],
    source: &[[u16; 4]],
    first: usize,
    y: usize,
    quarter: i32,
) {
    let bias = BAYER[y & 3].map(|b| (b as i32 * 2 - 15) * 512 + 8192);
    for (i, (pixel, texel)) in dest.iter_mut().zip(source).enumerate() {
        let keep = 16384 - i32::from(texel[3]) * quarter;
        let d = bias[(first + i) & 3];
        for c in 0..3 {
            let numerator = i32::from(texel[c]) * (quarter * 16) + i32::from(pixel[c]) * keep + d;
            pixel[c] = (numerator >> 14).clamp(0, 255) as u8;
        }
        if texel[3] > 0 {
            pixel[3] = 255;
        }
    }
}

/// Stop colors premultiplied once per draw, preserving the sampler's operation order.
struct Sampler<'a> {
    stops: &'a [Stop],
    colors: Vec<[f32; 4]>,
}

impl<'a> Sampler<'a> {
    fn new(stops: &'a [Stop]) -> Self {
        let colors = stops
            .iter()
            .map(|stop| {
                let c = stop.color;
                let a = f32::from(c.a) / 255.0;
                [
                    f32::from(c.r) * a,
                    f32::from(c.g) * a,
                    f32::from(c.b) * a,
                    a,
                ]
            })
            .collect();
        Self { stops, colors }
    }

    #[inline]
    fn sample(&self, t: f32) -> [f32; 4] {
        let last = self.stops.len() - 1;
        if t <= self.stops[0].pos {
            return self.colors[0];
        }
        if t >= self.stops[last].pos {
            return self.colors[last];
        }
        let next = self.stops.partition_point(|stop| stop.pos <= t);
        let (a, b) = (self.stops[next - 1], self.stops[next]);
        let span = b.pos - a.pos;
        let f = if span <= 0.0 { 1.0 } else { (t - a.pos) / span };
        let (pa, pb) = (self.colors[next - 1], self.colors[next]);
        std::array::from_fn(|i| pa[i] + (pb[i] - pa[i]) * f)
    }
}

/// Maps a pixel center to the gradient parameter `t` for one box.
enum Mapper {
    Linear {
        center: (f32, f32),
        dir: (f32, f32),
        len: f32,
    },
    Radial {
        center: (f32, f32),
        radii: (f32, f32),
    },
}

impl Mapper {
    fn new(kind: &GradientKind, [x0, y0, x1, y1]: [f32; 4]) -> Self {
        let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
        match *kind {
            GradientKind::Linear(direction) => {
                let dir = match direction {
                    LinearDirection::Angle(deg) => {
                        let r = deg.to_radians();
                        (r.sin(), -r.cos())
                    }
                    LinearDirection::Corner(sx, sy) => {
                        let (dx, dy) = (f32::from(sx) * h, f32::from(sy) * w);
                        let norm = dx.hypot(dy);
                        (dx / norm, dy / norm)
                    }
                };
                Self::Linear {
                    center: ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
                    dir,
                    len: (w * dir.0).abs() + (h * dir.1).abs(),
                }
            }
            GradientKind::Radial {
                circle,
                center,
                farthest_side,
            } => {
                let (cx, cy) = (x0 + center.0 * w, y0 + center.1 * h);
                let (fx, fy) = ((cx - x0).max(x1 - cx), (cy - y0).max(y1 - cy));
                let radii = match (circle, farthest_side) {
                    (true, true) => (fx.max(fy), fx.max(fy)),
                    (true, false) => (fx.hypot(fy), fx.hypot(fy)),
                    (false, true) => (fx, fy),
                    (false, false) => {
                        (fx * std::f32::consts::SQRT_2, fy * std::f32::consts::SQRT_2)
                    }
                };
                Self::Radial {
                    center: (cx, cy),
                    radii: (radii.0.max(1e-3), radii.1.max(1e-3)),
                }
            }
        }
    }

    #[inline]
    fn t(&self, x: f32, y: f32) -> f32 {
        match *self {
            Self::Linear { center, dir, len } => {
                ((x - center.0) * dir.0 + (y - center.1) * dir.1) / len.max(1e-3) + 0.5
            }
            Self::Radial { center, radii } => {
                hypot((x - center.0) / radii.0, (y - center.1) / radii.1)
            }
        }
    }
}

/// Alpha (0..=1) of `gradient` laid over the box with pixel edges `[x0, y0, x1, y1]`, sampled at the
/// center of every pixel of `rows` x `columns` (absolute pixel ranges), row by row. Used by
/// `mask-image`.
pub(in crate::render) fn alpha_plane(
    gradient: &Gradient,
    edges: [f32; 4],
    columns: std::ops::Range<usize>,
    rows: std::ops::Range<usize>,
) -> Vec<f32> {
    let mapper = Mapper::new(&gradient.kind, edges);
    let sampler = Sampler::new(&gradient.stops);
    let mut out = Vec::with_capacity(columns.len() * rows.len());
    for y in rows {
        for x in columns.clone() {
            out.push(sampler.sample(mapper.t(x as f32 + 0.5, y as f32 + 0.5))[3]);
        }
    }
    out
}

/// One rasterized gradient: premultiplied color (x256) and alpha (x4096) per texel, plus the
/// painted column range of each row so fully transparent margins cost nothing.
pub(in crate::render) struct Tile {
    width: usize,
    texels: Vec<[u16; 4]>,
    spans: Vec<(u32, u32)>,
}

impl Tile {
    fn build(gradient: &Gradient, width: usize, height: usize) -> Self {
        let mapper = Mapper::new(&gradient.kind, [0.0, 0.0, width as f32, height as f32]);
        let sampler = Sampler::new(&gradient.stops);
        let mut texels = Vec::with_capacity(width * height);
        let mut spans = Vec::with_capacity(height);
        for y in 0..height {
            let start = texels.len();
            for x in 0..width {
                let [r, g, b, a] = sampler.sample(mapper.t(x as f32 + 0.5, y as f32 + 0.5));
                texels.push([
                    (r * 256.0).round() as u16,
                    (g * 256.0).round() as u16,
                    (b * 256.0).round() as u16,
                    (a * 4096.0).round() as u16,
                ]);
            }
            let row = &texels[start..];
            let first = row.iter().position(|t| t[3] > 0);
            let last = row.iter().rposition(|t| t[3] > 0);
            spans.push(match (first, last) {
                (Some(f), Some(l)) => (f as u32, l as u32 + 1),
                _ => (0, 0),
            });
        }
        Self {
            width,
            texels,
            spans,
        }
    }
}

/// Gradient tiles keyed by gradient identity and pixel size.
pub(in crate::render) type GradientCache = TileCache<(usize, usize, usize), Tile>;

impl Surface {
    pub(in crate::render) fn fill_gradient_general(
        &mut self,
        rect: Rect,
        gradient: &Gradient,
        alpha: f32,
        radius: f32,
    ) -> Result<()> {
        let (edges, radius) = self.fill_edges(rect, radius);
        let [x0, y0, x1, y1] = edges;
        let Some(bounds) = clip_pixels((self.width, self.height), x0, y0, x1, y1) else {
            return Ok(());
        };
        self.mark_painted(bounds);
        let width = self.width as usize;
        let float_edges = [
            rect.x * self.scale,
            rect.y * self.scale,
            (rect.x + rect.w) * self.scale,
            (rect.y + rect.h) * self.scale,
        ];
        if radius == 0.0 && float_edges.iter().all(|edge| edge.fract() == 0.0) {
            let (w, h) = (
                (i64::from(x1) - i64::from(x0)) as usize,
                (i64::from(y1) - i64::from(y0)) as usize,
            );
            if alpha <= 0.0 {
                return Ok(());
            }
            let key = (gradient as *const Gradient as usize, w, h);
            if let Some(tile) = self.gradient_cache.get_or_build(
                key,
                w.checked_mul(h)
                    .and_then(|n| n.checked_mul(8))
                    .unwrap_or(usize::MAX),
                || Tile::build(gradient, w, h),
            )? {
                let alpha = alpha.clamp(0.0, 1.0);
                for y in bounds.y0..bounds.y1 {
                    let ty = (y as i32 - y0) as usize;
                    let (lo, hi) = tile.spans[ty];
                    let first = (bounds.x0 as i32).max(x0 + lo as i32) as usize;
                    let last = (bounds.x1 as i32).min(x0 + hi as i32).max(0) as usize;
                    if first >= last {
                        continue;
                    }
                    let row = &tile.texels[ty * tile.width..(ty + 1) * tile.width];
                    let dither_row = [0, 1, 2, 3].map(|i| dither(i, y));
                    let dest = &mut self.pixels[(y * width + first) * 4..(y * width + last) * 4];
                    let source = &row[(first as i32 - x0) as usize..(last as i32 - x0) as usize];
                    if alpha == 1.0 {
                        blend_full_opacity(dest.as_chunks_mut::<4>().0, source, first, y);
                        continue;
                    }
                    if alpha == 0.25 || alpha == 0.5 || alpha == 0.75 {
                        blend_quarter_opacity(
                            dest.as_chunks_mut::<4>().0,
                            source,
                            first,
                            y,
                            (alpha * 4.0) as i32,
                        );
                        continue;
                    }
                    for (i, (pixel, texel)) in dest
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(source)
                        .enumerate()
                    {
                        let d = dither_row[(first + i) & 3];
                        let keep = 1.0 - f32::from(texel[3]) * (alpha / 4096.0);
                        for c in 0..3 {
                            let blended = f32::from(texel[c]) * (alpha / 256.0)
                                + f32::from(pixel[c]) * keep
                                + d;
                            pixel[c] = (blended + 0.5).clamp(0.0, 255.0) as u8;
                        }
                        if texel[3] > 0 {
                            pixel[3] = 255;
                        }
                    }
                }
                return Ok(());
            }
        }
        let mapper = Mapper::new(&gradient.kind, float_edges);
        let sampler = Sampler::new(&gradient.stops);
        for y in bounds.y0..bounds.y1 {
            let (left, right) = Self::rounded_span(float_edges, radius, y);
            let first = (left.floor().max(bounds.x0 as f32)) as usize;
            let last = (right.ceil().min(bounds.x1 as f32)) as usize;
            // Row invariants, hoisted out of the pixel loop (same expressions, same values).
            let vertical =
                (float_edges[3].min(y as f32 + 1.0) - float_edges[1].max(y as f32)).clamp(0.0, 1.0);
            let dither_row = [0, 1, 2, 3].map(|i| dither(i, y));
            for x in first..last {
                let coverage =
                    (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0) * vertical;
                let [pr, pg, pb, pa] = sampler.sample(mapper.t(x as f32 + 0.5, y as f32 + 0.5));
                // `pa` carries the stop alpha; the premultiplied colors already include it.
                let a = (alpha * coverage).clamp(0.0, 1.0);
                if a <= 0.0 || pa <= 0.0 {
                    continue;
                }
                let d = dither_row[x & 3];
                let offset = (y * width + x) * 4;
                let dest = &mut self.pixels[offset..offset + 4];
                for (channel, premult) in dest.iter_mut().take(3).zip([pr, pg, pb]) {
                    let blended = premult * a + f32::from(*channel) * (1.0 - pa * a) + d;
                    *channel = round_to_u8(blended);
                }
                dest[3] = 255;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glow() -> Gradient {
        let color = |r, g, b, a| Color { r, g, b, a };
        Gradient {
            kind: GradientKind::Radial {
                circle: false,
                center: (0.5, 0.5),
                farthest_side: true,
            },
            stops: vec![
                Stop {
                    pos: 0.0,
                    color: color(111, 141, 255, 87),
                },
                Stop {
                    pos: 1.0,
                    color: color(111, 141, 255, 0),
                },
            ],
        }
    }

    fn reference_sample(stops: &[Stop], t: f32) -> [f32; 4] {
        let premultiplied = |c: Color| {
            let a = f32::from(c.a) / 255.0;
            [
                f32::from(c.r) * a,
                f32::from(c.g) * a,
                f32::from(c.b) * a,
                a,
            ]
        };
        let (first, last) = (stops[0], stops[stops.len() - 1]);
        if t <= first.pos {
            return premultiplied(first.color);
        }
        if t >= last.pos {
            return premultiplied(last.color);
        }
        let next = stops.partition_point(|stop| stop.pos <= t);
        let (a, b) = (stops[next - 1], stops[next]);
        let span = b.pos - a.pos;
        let f = if span <= 0.0 { 1.0 } else { (t - a.pos) / span };
        let (pa, pb) = (premultiplied(a.color), premultiplied(b.color));
        std::array::from_fn(|i| pa[i] + (pb[i] - pa[i]) * f)
    }

    #[test]
    fn prepared_stops_and_quarter_blends_preserve_original_values() {
        let mut gradient = glow();
        gradient.stops.insert(
            1,
            Stop {
                pos: 0.37,
                color: Color {
                    r: 231,
                    g: 0,
                    b: 73,
                    a: 201,
                },
            },
        );
        let sampler = Sampler::new(&gradient.stops);
        for i in -100..1100 {
            let t = i as f32 / 1000.0;
            assert_eq!(
                sampler.sample(t).map(f32::to_bits),
                reference_sample(&gradient.stops, t).map(f32::to_bits)
            );
        }
        for q in 1..4 {
            let alpha = q as f32 / 4.0;
            for y in 0..4 {
                for first in 0..4 {
                    for a in [0, 1, 13, 1024, 2048, 4095, 4096] {
                        let source: Vec<[u16; 4]> = (0..=255u16)
                            .map(|v| [v * 256, (255 - v) * 256, v * 123, a])
                            .collect();
                        let mut actual: Vec<[u8; 4]> =
                            (0..=255).map(|v| [v, 255 - v, v / 2, 37]).collect();
                        let mut expected = actual.clone();
                        for (i, (pixel, texel)) in expected.iter_mut().zip(&source).enumerate() {
                            let keep = 1.0 - f32::from(texel[3]) * (alpha / 4096.0);
                            for c in 0..3 {
                                let blended = f32::from(texel[c]) * (alpha / 256.0)
                                    + f32::from(pixel[c]) * keep
                                    + dither(first + i, y);
                                pixel[c] = (blended + 0.5).clamp(0.0, 255.0) as u8;
                            }
                            if texel[3] > 0 {
                                pixel[3] = 255;
                            }
                        }
                        blend_quarter_opacity(&mut actual, &source, first, y, q);
                        assert_eq!(actual, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn integer_full_opacity_matches_cached_float_blend_exactly() {
        for y in 0..4 {
            for first in 0..4 {
                for a in [0, 1, 17, 1024, 2048, 4095, 4096] {
                    for color in [0, 1, 255, 12345, 32768, 65280] {
                        let source = vec![[color, color / 2, color / 3, a]; 256];
                        let mut actual: Vec<[u8; 4]> =
                            (0..=255).map(|v| [v, 255 - v, v / 2, 37]).collect();
                        let mut expected = actual.clone();
                        for (i, (pixel, texel)) in expected.iter_mut().zip(&source).enumerate() {
                            let keep = 1.0 - f32::from(texel[3]) / 4096.0;
                            for c in 0..3 {
                                let blended = f32::from(texel[c]) / 256.0
                                    + f32::from(pixel[c]) * keep
                                    + dither(first + i, y);
                                pixel[c] = (blended + 0.5).clamp(0.0, 255.0) as u8;
                            }
                            if texel[3] > 0 {
                                pixel[3] = 255;
                            }
                        }
                        blend_full_opacity(&mut actual, &source, first, y);
                        assert_eq!(
                            actual, expected,
                            "alpha {a}, color {color}, phase {first},{y}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cached_tile_matches_direct_drawing_within_one_level() {
        let gradient = glow();
        let rect = |x| Rect {
            x,
            y: -7.0,
            w: 90.0,
            h: 70.0,
        };
        let mut cached = Surface::filled(64, 48, 1.0, Color::BLACK);
        for x in [-30.0, -29.0, -26.5] {
            cached
                .fill_gradient_general(rect(x), &gradient, 0.8, 0.0)
                .unwrap();
        }
        // Direct drawing of all three on a cache-less surface each time.
        let mut expected = Surface::filled(64, 48, 1.0, Color::BLACK);
        for x in [-30.0, -29.0, -26.5] {
            expected.gradient_cache = GradientCache::default();
            expected
                .fill_gradient_general(rect(x), &gradient, 0.8, 0.0)
                .unwrap();
        }
        let worst = cached
            .pixels
            .iter()
            .zip(&expected.pixels)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 1, "tile differs from direct drawing by {worst}");
    }
}
