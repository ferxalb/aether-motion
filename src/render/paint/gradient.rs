//! General gradient fills: angled and corner `linear-gradient`, multi-stop, translucent stops,
//! and `radial-gradient`. Evaluated per pixel with an ordered dither so dark ramps do not band.
//! The two-opaque-stop axis case stays on the faster `Surface::fill_gradient`.

use crate::model::{Color, Gradient, GradientKind, LinearDirection, Stop};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::surface::Surface;

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

/// Premultiplied RGBA (0..=255 scale) at `t`, clamped to the first and last stop.
fn sample(stops: &[Stop], t: f32) -> [f32; 4] {
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

    fn t(&self, x: f32, y: f32) -> f32 {
        match *self {
            Self::Linear { center, dir, len } => {
                ((x - center.0) * dir.0 + (y - center.1) * dir.1) / len.max(1e-3) + 0.5
            }
            Self::Radial { center, radii } => {
                ((x - center.0) / radii.0).hypot((y - center.1) / radii.1)
            }
        }
    }
}

impl Surface {
    pub(in crate::render) fn fill_gradient_general(
        &mut self,
        rect: Rect,
        gradient: &Gradient,
        alpha: f32,
        radius: f32,
    ) {
        let (edges, radius) = self.fill_edges(rect, radius);
        let [x0, y0, x1, y1] = edges;
        let Some(bounds) = clip_pixels((self.width, self.height), x0, y0, x1, y1) else {
            return;
        };
        self.mark_painted(bounds);
        let float_edges = edges.map(|edge| edge as f32);
        let mapper = Mapper::new(&gradient.kind, float_edges);
        let width = self.width as usize;
        for y in bounds.y0..bounds.y1 {
            let (left, right) = Self::rounded_span(float_edges, radius, y);
            let first = (left.floor().max(bounds.x0 as f32)) as usize;
            let last = (right.ceil().min(bounds.x1 as f32)) as usize;
            for x in first..last {
                let coverage = (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0);
                let [pr, pg, pb, pa] =
                    sample(&gradient.stops, mapper.t(x as f32 + 0.5, y as f32 + 0.5));
                // `pa` carries the stop alpha; the premultiplied colors already include it.
                let a = (alpha * coverage).clamp(0.0, 1.0);
                if a <= 0.0 || pa <= 0.0 {
                    continue;
                }
                let d = dither(x, y);
                let offset = (y * width + x) * 4;
                let dest = &mut self.pixels[offset..offset + 4];
                for (channel, premult) in dest.iter_mut().take(3).zip([pr, pg, pb]) {
                    let blended = premult * a + f32::from(*channel) * (1.0 - pa * a) + d;
                    *channel = blended.round().clamp(0.0, 255.0) as u8;
                }
                dest[3] = 255;
            }
        }
    }
}
