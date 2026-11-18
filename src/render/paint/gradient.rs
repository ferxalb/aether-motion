//! General gradient fills: angled and corner `linear-gradient`, multi-stop, translucent stops,
//! and `radial-gradient`. Evaluated per pixel with an ordered dither so dark ramps do not band.
//! The two-opaque-stop axis case stays on the faster `Surface::fill_gradient`.
//!
//! A square, whole-pixel gradient is evaluated through fixed-point texels in box-relative
//! coordinates. When it is drawn again with the same size (a drifting glow, for example) the texels
//! are rasterized once into a tile and later frames only blend that tile at the new offset; an
//! uncached draw computes the same texels for its visible pixels, so both paths match exactly.
//! Full and quarter element opacities use exact integer tile blending, preserving ordered dithering.
//! Stop colors are premultiplied once per draw, including uncached fractional-motion draws.

use crate::model::{Color, Gradient, GradientKind, LinearDirection, Stop};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::float_ops::{hypot, round_to_u16, round_to_u8};
use crate::render::paint::kernels::{self, GradientRow, TwoStops};
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
        // With two stops, a `t` strictly between them always interpolates between stops 0 and 1.
        let next = if last == 1 {
            1
        } else {
            self.stops.partition_point(|stop| stop.pos <= t)
        };
        let (a, b) = (self.stops[next - 1], self.stops[next]);
        let span = b.pos - a.pos;
        let f = if span <= 0.0 { 1.0 } else { (t - a.pos) / span };
        let (pa, pb) = (self.colors[next - 1], self.colors[next]);
        std::array::from_fn(|i| pa[i] + (pb[i] - pa[i]) * f)
    }

    /// Position past which every sample is the fully transparent last stop, when there is one.
    fn transparent_after(&self) -> Option<f32> {
        let last = self.stops.len() - 1;
        let pos = self.stops[last].pos;
        (self.colors[last][3] == 0.0 && pos > self.stops[0].pos && pos > 0.0 && pos.is_finite())
            .then_some(pos)
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

    /// `t` at the centers of pixels `columns` of row `y`, bit-identical to [`Mapper::t`] (the row's
    /// own term is computed once per row).
    fn row(&self, y: usize, columns: std::ops::Range<usize>, out: &mut Vec<f32>) {
        out.clear();
        let py = y as f32 + 0.5;
        match *self {
            Self::Linear { center, dir, len } => {
                let across = (py - center.1) * dir.1;
                let len = len.max(1e-3);
                out.extend(
                    columns.map(|x| ((x as f32 + 0.5 - center.0) * dir.0 + across) / len + 0.5),
                );
            }
            Self::Radial { center, radii } => {
                let v = (py - center.1) / radii.1;
                if v.is_infinite() {
                    out.extend(columns.map(|_| f32::INFINITY));
                    return;
                }
                let vv = f64::from(v) * f64::from(v);
                kernels::radial_row(out, columns, center.0, radii.0, vv);
            }
        }
    }

    /// Columns of row `y` that can have `t` below `limit`: pixels outside are guaranteed to sample
    /// at or past it. Radial gradients only (`None` otherwise, or when nothing can be excluded).
    /// The bound is widened by a relative 1e-4 plus one pixel, far beyond `f32` rounding.
    fn columns_below(&self, y: usize, limit: f32) -> Option<std::ops::Range<f64>> {
        let Self::Radial { center, radii } = *self else {
            return None;
        };
        let limit = f64::from(limit) * (1.0 + 1e-4) + 1e-4;
        let v = (f64::from(y as f32 + 0.5) - f64::from(center.1)).abs() / f64::from(radii.1);
        if v >= limit {
            return Some(f64::INFINITY..f64::NEG_INFINITY);
        }
        let half = f64::from(radii.0) * (limit * limit - v * v).sqrt() + 1.0;
        let cx = f64::from(center.0);
        Some(cx - half - 0.5..cx + half - 0.5)
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

/// A gradient laid over a box in output pixels, sampled per pixel (gradient-filled text).
pub(in crate::render) struct GradientPaint<'a> {
    mapper: Mapper,
    sampler: Sampler<'a>,
}

impl<'a> GradientPaint<'a> {
    pub(in crate::render) fn new(gradient: &'a Gradient, edges: [f32; 4]) -> Self {
        Self {
            mapper: Mapper::new(&gradient.kind, edges),
            sampler: Sampler::new(&gradient.stops),
        }
    }

    /// Straight (unpremultiplied) color at the center of pixel `(x, y)`.
    pub(in crate::render) fn color_at(&self, x: usize, y: usize) -> Color {
        let [r, g, b, a] = self
            .sampler
            .sample(self.mapper.t(x as f32 + 0.5, y as f32 + 0.5));
        if a <= 0.0 {
            return Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            };
        }
        let channel = |v: f32| (v / a).round().clamp(0.0, 255.0) as u8;
        Color {
            r: channel(r),
            g: channel(g),
            b: channel(b),
            a: (a * 255.0).round().clamp(0.0, 255.0) as u8,
        }
    }
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
        let mut ts = Vec::with_capacity(width);
        for y in 0..height {
            let start = texels.len();
            mapper.row(y, 0..width, &mut ts);
            texels.extend(ts.iter().map(|&t| texel(&sampler, t)));
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

/// The fixed-point texel at gradient parameter `t` (of a tile-relative pixel center, from
/// [`Mapper::row`]): premultiplied color x256 and alpha x4096; a texel whose alpha rounds to zero
/// is transparent black, so it leaves every pixel unchanged in all blend paths.
/// Cached tiles and uncached draws of a tileable gradient both come from here, so a draw has the same
/// pixels whether or not its tile is cached (and so for any thread count or render split).
#[inline]
fn texel(sampler: &Sampler, t: f32) -> [u16; 4] {
    let [r, g, b, a] = sampler.sample(t);
    let alpha = round_to_u16(a * 4096.0);
    if alpha == 0 {
        // Transparent black: a faint color left on a zero-alpha texel could still nudge a pixel.
        return [0; 4];
    }
    [
        round_to_u16(r * 256.0),
        round_to_u16(g * 256.0),
        round_to_u16(b * 256.0),
        alpha,
    ]
}

/// Blends one row of texels onto `dest` (pixel `first` of row `y`) at element opacity `alpha`
/// (0..=1). Texels with zero alpha leave their pixel unchanged.
fn blend_row(dest: &mut [[u8; 4]], source: &[[u16; 4]], first: usize, y: usize, alpha: f32) {
    if alpha == 1.0 {
        return blend_full_opacity(dest, source, first, y);
    }
    if alpha == 0.25 || alpha == 0.5 || alpha == 0.75 {
        return blend_quarter_opacity(dest, source, first, y, (alpha * 4.0) as i32);
    }
    let dither_row = [0, 1, 2, 3].map(|i| dither(i, y));
    for (i, (pixel, texel)) in dest.iter_mut().zip(source).enumerate() {
        let d = dither_row[(first + i) & 3];
        let keep = 1.0 - f32::from(texel[3]) * (alpha / 4096.0);
        for c in 0..3 {
            let blended = f32::from(texel[c]) * (alpha / 256.0) + f32::from(pixel[c]) * keep + d;
            pixel[c] = (blended + 0.5).clamp(0.0, 255.0) as u8;
        }
        if texel[3] > 0 {
            pixel[3] = 255;
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
            let tile = self.gradient_cache.get_or_build(
                key,
                w.checked_mul(h)
                    .and_then(|n| n.checked_mul(8))
                    .unwrap_or(usize::MAX),
                || Tile::build(gradient, w, h),
            )?;
            let alpha = alpha.clamp(0.0, 1.0);
            let pixels = self.pixels.as_chunks_mut::<4>().0;
            if let Some(tile) = tile {
                for y in bounds.y0..bounds.y1 {
                    let ty = (y as i32 - y0) as usize;
                    let (lo, hi) = tile.spans[ty];
                    let first = (bounds.x0 as i32).max(x0 + lo as i32) as usize;
                    let last = (bounds.x1 as i32).min(x0 + hi as i32).max(0) as usize;
                    if first >= last {
                        continue;
                    }
                    let row = &tile.texels[ty * tile.width..(ty + 1) * tile.width];
                    let source = &row[(first as i32 - x0) as usize..(last as i32 - x0) as usize];
                    blend_row(
                        &mut pixels[y * width + first..y * width + last],
                        source,
                        first,
                        y,
                        alpha,
                    );
                }
            } else {
                // The same texels as the tile, computed for the visible pixels only.
                let mapper = Mapper::new(&gradient.kind, [0.0, 0.0, w as f32, h as f32]);
                let sampler = Sampler::new(&gradient.stops);
                let (first, last) = (bounds.x0, bounds.x1);
                let columns = (first as i32 - x0) as usize..(last as i32 - x0) as usize;
                let (mut row, mut ts) = (Vec::with_capacity(last - first), Vec::new());
                for y in bounds.y0..bounds.y1 {
                    let ty = (y as i32 - y0) as usize;
                    mapper.row(ty, columns.clone(), &mut ts);
                    row.clear();
                    row.extend(ts.iter().map(|&t| texel(&sampler, t)));
                    blend_row(
                        &mut pixels[y * width + first..y * width + last],
                        &row,
                        first,
                        y,
                        alpha,
                    );
                }
            }
            return Ok(());
        }
        let mapper = Mapper::new(&gradient.kind, float_edges);
        let sampler = Sampler::new(&gradient.stops);
        let transparent_after = sampler.transparent_after();
        let two_stops = (gradient.stops.len() == 2).then(|| TwoStops {
            positions: [gradient.stops[0].pos, gradient.stops[1].pos],
            colors: [sampler.colors[0], sampler.colors[1]],
        });
        let mut ts = Vec::with_capacity(bounds.x1 - bounds.x0);
        for y in bounds.y0..bounds.y1 {
            let (left, right) = Self::rounded_span(float_edges, radius, y);
            let mut first = (left.floor().max(bounds.x0 as f32)) as usize;
            let mut last = (right.ceil().min(bounds.x1 as f32)) as usize;
            // Pixels that sample the transparent last stop paint nothing; skip them unevaluated.
            if let Some(range) = transparent_after.and_then(|pos| mapper.columns_below(y, pos)) {
                first = first.max(range.start.ceil().max(0.0) as usize);
                last = last.min((range.end.floor() + 1.0).max(0.0) as usize);
            }
            if first >= last {
                continue;
            }
            // Row invariants, hoisted out of the pixel loop (same expressions, same values).
            let vertical =
                (float_edges[3].min(y as f32 + 1.0) - float_edges[1].max(y as f32)).clamp(0.0, 1.0);
            let dither_row = [0, 1, 2, 3].map(|i| dither(i, y));
            mapper.row(y, first..last, &mut ts);
            let row = &mut self.pixels[(y * width + first) * 4..(y * width + last) * 4];
            if let Some(stops) = &two_stops {
                let params = GradientRow {
                    left,
                    right,
                    vertical,
                    alpha,
                    dither: dither_row,
                };
                kernels::two_stop_row(row.as_chunks_mut::<4>().0, &ts, first, &params, stops);
                continue;
            }
            for ((x, dest), &t) in (first..last).zip(row.as_chunks_mut::<4>().0).zip(&ts) {
                let coverage =
                    (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0) * vertical;
                let [pr, pg, pb, pa] = sampler.sample(t);
                // `pa` carries the stop alpha; the premultiplied colors already include it.
                let a = (alpha * coverage).clamp(0.0, 1.0);
                if a <= 0.0 || pa <= 0.0 {
                    continue;
                }
                let d = dither_row[x & 3];
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
    fn row_evaluation_and_transparent_skip_match_per_pixel_sampling() {
        let boxes = [
            [-300.4, -350.7, 1100.4, 850.2],
            [12.25, 3.5, 77.75, 40.0],
            [0.0, 0.0, 1.0, 900.0],
        ];
        let kinds = [
            glow().kind,
            GradientKind::Radial {
                circle: true,
                center: (0.2, 0.9),
                farthest_side: false,
            },
            GradientKind::Linear(LinearDirection::Angle(135.0)),
            GradientKind::Linear(LinearDirection::Corner(1, -1)),
        ];
        let stops = glow().stops;
        let sampler = Sampler::new(&stops);
        let limit = sampler.transparent_after().unwrap();
        let mut row = Vec::new();
        for edges in boxes {
            for kind in &kinds {
                let mapper = Mapper::new(kind, edges);
                for y in (0..1000).step_by(7) {
                    mapper.row(y, 0..1200, &mut row);
                    let kept = mapper.columns_below(y, limit);
                    for (x, &t) in (0..1200).zip(&row) {
                        let exact = mapper.t(x as f32 + 0.5, y as f32 + 0.5);
                        assert_eq!(t.to_bits(), exact.to_bits(), "{kind:?} {edges:?} {x},{y}");
                        let inside = kept
                            .as_ref()
                            .is_none_or(|r| (x as f64) >= r.start && (x as f64) <= r.end);
                        if !inside {
                            assert_eq!(sampler.sample(t)[3], 0.0, "skipped pixel {x},{y} paints");
                        }
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
    fn zero_alpha_texels_never_change_pixels() {
        // A bright stop fading to nothing: near the edge, alpha rounds to zero before color does.
        let stops = vec![
            Stop {
                pos: 0.0,
                color: Color::WHITE,
            },
            Stop {
                pos: 1.0,
                color: Color {
                    r: 255,
                    g: 255,
                    b: 255,
                    a: 0,
                },
            },
        ];
        let sampler = Sampler::new(&stops);
        for i in 0..=100_000 {
            let t = 0.999 + i as f32 * 1e-8;
            let texel = texel(&sampler, t);
            if texel[3] == 0 {
                assert_eq!(texel, [0; 4], "t {t}");
            }
        }
        for alpha in [1.0, 0.75, 0.3] {
            for y in 0..4 {
                let mut pixels: Vec<[u8; 4]> = (0..=255).map(|v| [v, v, 255 - v, 9]).collect();
                let before = pixels.clone();
                blend_row(&mut pixels, &[[0; 4]; 256], 0, y, alpha);
                assert_eq!(pixels, before);
            }
        }
    }

    #[test]
    fn cached_tile_matches_direct_drawing_at_full_opacity() {
        let gradient = glow();
        let rect = |x| Rect {
            x,
            y: -3.0,
            w: 120.0,
            h: 60.0,
        };
        let positions = [-40.0, -39.0, -41.0, -39.0, 2.0];
        let mut warm = Surface::filled(96, 54, 1.0, Color::BLACK);
        let mut cold = Surface::filled(96, 54, 1.0, Color::BLACK);
        for x in positions {
            warm.fill_gradient_general(rect(x), &gradient, 1.0, 0.0)
                .unwrap();
            cold.gradient_cache = GradientCache::default();
            cold.fill_gradient_general(rect(x), &gradient, 1.0, 0.0)
                .unwrap();
        }
        assert!(warm.pixels == cold.pixels, "warm and cold caches differ");
    }

    #[test]
    fn cached_tile_matches_direct_drawing_exactly() {
        let gradient = glow();
        let rect = |x| Rect {
            x,
            y: -7.0,
            w: 90.0,
            h: 70.0,
        };
        let mut cached = Surface::filled(64, 48, 1.0, Color::BLACK);
        for x in [-30.0, -29.0, -26.5, -29.0, 0.0] {
            cached
                .fill_gradient_general(rect(x), &gradient, 0.8, 0.0)
                .unwrap();
        }
        // Direct drawing of all three on a cache-less surface each time.
        let mut expected = Surface::filled(64, 48, 1.0, Color::BLACK);
        for x in [-30.0, -29.0, -26.5, -29.0, 0.0] {
            expected.gradient_cache = GradientCache::default();
            expected
                .fill_gradient_general(rect(x), &gradient, 0.8, 0.0)
                .unwrap();
        }
        assert!(
            cached.pixels == expected.pixels,
            "tile differs from direct drawing"
        );
    }
}
