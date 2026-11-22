//! Deterministic CPU rasterizer for [`Frame`]s: clipping, fixed-point coverage, per-sample depth
//! testing, and supersampled resolve, in bounded tile buffers.
//!
//! Each triangle is transformed to clip space with the shared [`ClipMatrix`] (in `f64`), clipped
//! against the near and far planes and a guard band around the target, and snapped to a grid of
//! 1/256 sample. Coverage uses exact integer edge functions with the top-left fill rule, so
//! adjacent triangles cover every sample exactly once. Depth is reversed (1 near, 0 far), linear in
//! screen space, evaluated per sample center from the snapped corners, and stored as `f32`; a
//! sample keeps the first triangle that is strictly nearer, so ties resolve in draw order.
//!
//! The target is processed in 64 px tiles of `antialias^2` samples per pixel. Only one tile's
//! color and depth live at a time (at most 512 KiB at 4x4 supersampling), and each finished tile
//! is resolved to straight-alpha RGBA rows (the mean of the covered samples' colors; alpha is the
//! covered share) and handed to a sink. Every value depends only on the frame, the placement, and
//! the target rectangle, so renders are identical across threads, segments, and CPUs.

use super::camera::{ClipMatrix, Placement, View};
use super::frame::Triangle;

/// Receives resolved rows: `(x, y, row)` with `row` straight-alpha RGBA starting at target px
/// `(x, y)`.
pub type RowSink<'a> = dyn FnMut(usize, usize, &[[u8; 4]]) + 'a;

/// Tile edge in output px.
pub const TILE: usize = 64;
/// Fractional bits of snapped sample coordinates.
const SUBPIXEL_BITS: u32 = 8;
const ONE: i64 = 1 << SUBPIXEL_BITS;
/// Guard band: geometry is clipped to this many target widths and heights around the center.
const GUARD: f64 = 2.0;

/// The part of a target a viewport covers, in target px, and its supersampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// `[x0, y0, x1, y1]` target px (the viewport's box, cut to the target).
    pub rect: [usize; 4],
    /// The viewport's box in samples relative to `rect`'s corner: `[x0, y0, x1, y1]`.
    pub samples: [i64; 4],
    pub antialias: u32,
}

impl Region {
    /// The region of a `target` px surface that a viewport at `placement` covers, or `None` when
    /// it is empty.
    pub fn of(placement: &Placement, target: [usize; 2], antialias: u32) -> Option<Self> {
        let ss = f64::from(antialias);
        let left = placement.origin[0];
        let top = placement.origin[1];
        let right = left + placement.scale * placement.size[0];
        let bottom = top + placement.scale * placement.size[1];
        if !(left.is_finite() && top.is_finite() && right.is_finite() && bottom.is_finite()) {
            return None;
        }
        // Viewport edges on the sample grid, in absolute samples.
        let edge = |v: f64| (v * ss).round().clamp(-1e15, 1e15) as i64;
        let s = [edge(left), edge(top), edge(right), edge(bottom)];
        let a = i64::from(antialias);
        let rect = [
            s[0].div_euclid(a).clamp(0, target[0] as i64) as usize,
            s[1].div_euclid(a).clamp(0, target[1] as i64) as usize,
            (s[2] + a - 1).div_euclid(a).clamp(0, target[0] as i64) as usize,
            (s[3] + a - 1).div_euclid(a).clamp(0, target[1] as i64) as usize,
        ];
        if rect[0] >= rect[2] || rect[1] >= rect[3] {
            return None;
        }
        let (ox, oy) = (rect[0] as i64 * a, rect[1] as i64 * a);
        let samples = [
            (s[0] - ox).max(0),
            (s[1] - oy).max(0),
            (s[2] - ox).min((rect[2] - rect[0]) as i64 * a),
            (s[3] - oy).min((rect[3] - rect[1]) as i64 * a),
        ];
        (samples[0] < samples[2] && samples[1] < samples[3]).then_some(Self {
            rect,
            samples,
            antialias,
        })
    }

    pub fn width(&self) -> usize {
        self.rect[2] - self.rect[0]
    }

    pub fn height(&self) -> usize {
        self.rect[3] - self.rect[1]
    }

    /// The clip matrix of this region's raster (NDC spans `rect`).
    pub fn matrix(&self, view: &View, placement: &Placement) -> ClipMatrix {
        ClipMatrix::new(
            view,
            placement,
            [
                self.rect[0] as f64,
                self.rect[1] as f64,
                self.width() as f64,
                self.height() as f64,
            ],
        )
    }
}

/// A triangle ready to rasterize. `repr(C)` because the CUDA backend uploads these as they are
/// (112 bytes; its kernel declares the same layout).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub(crate) struct Setup {
    /// Snapped corners in 1/256 sample, relative to the region's corner.
    pub(crate) v: [[i64; 2]; 3],
    /// Inclusive-exclusive sample bounds `[x0, y0, x1, y1]`.
    pub(crate) bounds: [i64; 4],
    /// `z = z0 + zx * x + zy * y` at sample centers (`x`, `y` in samples).
    pub(crate) depth: [f64; 3],
    pub(crate) color: u32,
}

/// Reusable buffers of the rasterizer (bounded by one tile and the frame's triangles).
#[derive(Default)]
pub struct Scratch {
    setups: Vec<Setup>,
    band: Vec<u32>,
    tile: Vec<u32>,
    color: Vec<u32>,
    depth: Vec<f32>,
    out: Vec<[u8; 4]>,
}

impl Scratch {
    /// Bytes currently held.
    pub fn bytes(&self) -> usize {
        self.setups.capacity() * std::mem::size_of::<Setup>()
            + (self.band.capacity() + self.tile.capacity() + self.color.capacity()) * 4
            + self.depth.capacity() * 4
            + self.out.capacity() * 4
    }
}

pub(crate) const COVERED: u32 = 0xff00_0000;

/// Draws `triangles` seen by `view` into `region`, calling `sink(x, y, row)` with straight-alpha
/// RGBA for each resolved row segment that holds geometry (`x`, `y` target px).
pub fn rasterize(
    view: &View,
    triangles: &[Triangle],
    placement: &Placement,
    region: &Region,
    scratch: &mut Scratch,
    sink: &mut RowSink,
) {
    let ss = region.antialias as usize;
    setups(view, triangles, placement, region, &mut scratch.setups);
    if scratch.setups.is_empty() {
        return;
    }
    let tile_samples = TILE * ss;
    let n = (ss * ss) as u32;
    scratch.color.resize(tile_samples * tile_samples, 0);
    scratch.depth.resize(tile_samples * tile_samples, 0.0);
    scratch.out.resize(TILE, [0; 4]);
    let (w, h) = (region.width(), region.height());
    for ty in (0..h).step_by(TILE) {
        let band = [(ty * ss) as i64, ((ty + TILE).min(h) * ss) as i64];
        scratch.band.clear();
        scratch.band.extend(
            scratch
                .setups
                .iter()
                .enumerate()
                .filter(|(_, s)| s.bounds[1] < band[1] && s.bounds[3] > band[0])
                .map(|(i, _)| i as u32),
        );
        if scratch.band.is_empty() {
            continue;
        }
        for tx in (0..w).step_by(TILE) {
            let tw = TILE.min(w - tx);
            let th = TILE.min(h - ty);
            let x_range = [(tx * ss) as i64, ((tx + tw) * ss) as i64];
            scratch.tile.clear();
            let setups = &scratch.setups;
            scratch
                .tile
                .extend(scratch.band.iter().copied().filter(|&i| {
                    let b = setups[i as usize].bounds;
                    b[0] < x_range[1] && b[2] > x_range[0]
                }));
            if scratch.tile.is_empty() {
                continue;
            }
            let stride = tw * ss;
            let count = stride * th * ss;
            // A sample is covered exactly when its depth is above 0 (the far plane), since nothing
            // at or beyond it passes the depth test: colors need no clearing.
            scratch.depth[..count].fill(0.0);
            let origin = [x_range[0], band[0]];
            let limit = [x_range[1], band[0] + (th * ss) as i64];
            for &i in &scratch.tile {
                fill(
                    &scratch.setups[i as usize],
                    origin,
                    limit,
                    stride,
                    &mut scratch.color,
                    &mut scratch.depth,
                );
            }
            // Resolve each output row of the tile.
            for py in 0..th {
                let mut any = false;
                for px in 0..tw {
                    let (mut covered, mut sum) = (0_u32, [0_u32; 3]);
                    for sy in 0..ss {
                        let row = (py * ss + sy) * stride + px * ss;
                        let depths = &scratch.depth[row..row + ss];
                        for (&c, &z) in scratch.color[row..row + ss].iter().zip(depths) {
                            if z > 0.0 {
                                covered += 1;
                                sum[0] += c & 0xff;
                                sum[1] += c >> 8 & 0xff;
                                sum[2] += c >> 16 & 0xff;
                            }
                        }
                    }
                    scratch.out[px] = if covered == 0 {
                        [0; 4]
                    } else {
                        any = true;
                        let mean = |s: u32| ((s + covered / 2) / covered) as u8;
                        [
                            mean(sum[0]),
                            mean(sum[1]),
                            mean(sum[2]),
                            ((covered * 255 + n / 2) / n) as u8,
                        ]
                    };
                }
                if any {
                    sink(
                        region.rect[0] + tx,
                        region.rect[1] + ty + py,
                        &scratch.out[..tw],
                    );
                }
            }
        }
    }
}

/// Clips and snaps `triangles` into `out` (cleared first): the geometry stage every backend that
/// draws integer-snapped triangles shares, in draw order.
pub(crate) fn setups(
    view: &View,
    triangles: &[Triangle],
    placement: &Placement,
    region: &Region,
    out: &mut Vec<Setup>,
) {
    let matrix = region.matrix(view, placement);
    let ss = region.antialias as usize;
    let size = [(region.width() * ss) as f64, (region.height() * ss) as f64];
    out.clear();
    for triangle in triangles {
        setup(triangle, &matrix, size, region.samples, out);
    }
}

/// Clips one triangle and appends the rasterizable pieces.
fn setup(
    triangle: &Triangle,
    matrix: &ClipMatrix,
    size: [f64; 2],
    clip: [i64; 4],
    out: &mut Vec<Setup>,
) {
    let mut polygon: Vec<[f64; 4]> = triangle.corners.iter().map(|&p| matrix.apply(p)).collect();
    let planes: [fn([f64; 4]) -> f64; 6] = [
        |c| c[2],
        |c| c[3] - c[2],
        |c| GUARD * c[3] - c[0],
        |c| GUARD * c[3] + c[0],
        |c| GUARD * c[3] - c[1],
        |c| GUARD * c[3] + c[1],
    ];
    for plane in planes {
        if polygon.iter().all(|&c| plane(c) >= 0.0) {
            continue;
        }
        polygon = clip_plane(&polygon, plane);
        if polygon.len() < 3 {
            return;
        }
    }
    let color = u32::from(triangle.color[0])
        | u32::from(triangle.color[1]) << 8
        | u32::from(triangle.color[2]) << 16
        | COVERED;
    let points: Vec<([i64; 2], f64)> = polygon
        .iter()
        .map(|c| {
            let (x, y, z) = (c[0] / c[3], c[1] / c[3], c[2] / c[3]);
            let sx = (x + 1.0) / 2.0 * size[0];
            let sy = (1.0 - y) / 2.0 * size[1];
            (
                [
                    (sx * ONE as f64).round() as i64,
                    (sy * ONE as f64).round() as i64,
                ],
                z.clamp(0.0, 1.0),
            )
        })
        .collect();
    for i in 1..points.len() - 1 {
        let mut corners = [points[0], points[i], points[i + 1]];
        let area = edge(corners[0].0, corners[1].0, corners[2].0);
        if area == 0 {
            continue;
        }
        if area < 0 {
            corners.swap(1, 2);
        }
        let v = corners.map(|c| c.0);
        let lo = |axis: usize| v.iter().map(|p| p[axis]).min().unwrap_or(0);
        let hi = |axis: usize| v.iter().map(|p| p[axis]).max().unwrap_or(0);
        // Samples whose centers (n + 1/2) can lie inside.
        let first = |min: i64| (min - ONE / 2).div_euclid(ONE);
        let last = |max: i64| (max - ONE / 2).div_euclid(ONE) + 1;
        let bounds = [
            first(lo(0)).max(clip[0]),
            first(lo(1)).max(clip[1]),
            last(hi(0)).min(clip[2]),
            last(hi(1)).min(clip[3]),
        ];
        if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
            continue;
        }
        // Depth plane through the snapped corners, in sample units.
        let p = v.map(|c| [c[0] as f64 / ONE as f64, c[1] as f64 / ONE as f64]);
        let z = corners.map(|c| c.1);
        let (e1, e2) = (
            [p[1][0] - p[0][0], p[1][1] - p[0][1]],
            [p[2][0] - p[0][0], p[2][1] - p[0][1]],
        );
        let det = e1[0] * e2[1] - e1[1] * e2[0];
        let (dz1, dz2) = (z[1] - z[0], z[2] - z[0]);
        let zx = (dz1 * e2[1] - dz2 * e1[1]) / det;
        let zy = (dz2 * e1[0] - dz1 * e2[0]) / det;
        let z0 = z[0] - zx * p[0][0] - zy * p[0][1];
        out.push(Setup {
            v,
            bounds,
            depth: [z0, zx, zy],
            color,
        });
    }
}

/// Twice the signed area of `(a, b, p)`: positive when `p` lies on the interior side of `a -> b`
/// for triangles wound with a positive area (clockwise on a y-down screen).
fn edge(a: [i64; 2], b: [i64; 2], p: [i64; 2]) -> i64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// Whether samples exactly on edge `a -> b` belong to the triangle (top or left edge).
fn top_left(a: [i64; 2], b: [i64; 2]) -> bool {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    (dy == 0 && dx > 0) || dy < 0
}

/// Sutherland–Hodgman against `plane(c) >= 0` in clip space.
fn clip_plane(polygon: &[[f64; 4]], plane: fn([f64; 4]) -> f64) -> Vec<[f64; 4]> {
    let mut out = Vec::with_capacity(polygon.len() + 1);
    for (i, &current) in polygon.iter().enumerate() {
        let previous = polygon[(i + polygon.len() - 1) % polygon.len()];
        let (vc, vp) = (plane(current), plane(previous));
        if (vc >= 0.0) != (vp >= 0.0) {
            let t = vp / (vp - vc);
            out.push(std::array::from_fn(|k| {
                previous[k] + (current[k] - previous[k]) * t
            }));
        }
        if vc >= 0.0 {
            out.push(current);
        }
    }
    out
}

/// Covers the samples of one tile (`origin` .. `limit`, absolute region samples) with `s`.
fn fill(
    s: &Setup,
    origin: [i64; 2],
    limit: [i64; 2],
    stride: usize,
    color: &mut [u32],
    depth: &mut [f32],
) {
    let x0 = s.bounds[0].max(origin[0]);
    let x1 = s.bounds[2].min(limit[0]);
    let y0 = s.bounds[1].max(origin[1]);
    let y1 = s.bounds[3].min(limit[1]);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let [a, b, c] = s.v;
    let edges = [(a, b, c), (b, c, a), (c, a, b)];
    let center = |n: i64| n * ONE + ONE / 2;
    let start = [center(x0), center(y0)];
    // Edge values at the first sample (biased so that "inside" is `>= 0`) and their steps.
    let mut row_values = [0_i64; 3];
    let mut step_x = [0_i64; 3];
    let mut step_y = [0_i64; 3];
    for (k, &(p, q, _)) in edges.iter().enumerate() {
        let bias = i64::from(!top_left(p, q));
        row_values[k] = edge(p, q, start) - bias;
        step_x[k] = -(q[1] - p[1]) * ONE;
        step_y[k] = (q[0] - p[0]) * ONE;
    }
    let [z0, zx, zy] = s.depth;
    let width = x1 - x0;
    for y in y0..y1 {
        // The covered samples of this row, solved exactly from each edge's integer inequality
        // `value + step * i >= 0` (the same decision a per-sample test makes).
        let (mut lo, mut hi) = (0_i64, width);
        for k in 0..3 {
            let (value, step) = (row_values[k], step_x[k]);
            if step > 0 {
                lo = lo.max((-value).div_euclid(step) + i64::from((-value).rem_euclid(step) != 0));
            } else if step < 0 {
                hi = hi.min(value.div_euclid(-step) + 1);
            } else if value < 0 {
                hi = lo;
            }
        }
        if lo < hi {
            let row = (y - origin[1]) as usize * stride;
            let zrow = z0 + zy * (y as f64 + 0.5);
            let base = row + (x0 - origin[0]) as usize;
            for i in lo..hi {
                let x = x0 + i;
                let z = (zrow + zx * (x as f64 + 0.5)) as f32;
                let at = base + i as usize;
                if z > depth[at] {
                    depth[at] = z;
                    color[at] = s.color;
                }
            }
        }
        for k in 0..3 {
            row_values[k] += step_y[k];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d::camera::View;
    use crate::scene3d::Projection;

    fn front_view() -> View {
        View {
            eye: [0.0, 0.0, 1000.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            projection: Projection::Perspective { focal: 1000.0 },
            zoom: 1.0,
            near: 10.0,
            far: 50_000.0,
        }
    }

    fn draw(triangles: &[Triangle], size: usize, ss: u32) -> Vec<[u8; 4]> {
        let placement = Placement {
            origin: [0.0, 0.0],
            scale: 1.0,
            size: [size as f64; 2],
        };
        let region = Region::of(&placement, [size, size], ss).unwrap();
        let mut image = vec![[0; 4]; size * size];
        rasterize(
            &front_view(),
            triangles,
            &placement,
            &region,
            &mut Scratch::default(),
            &mut |x, y, row| image[y * size + x..y * size + x + row.len()].copy_from_slice(row),
        );
        image
    }

    fn quad(z: f64, half: f64, color: [u8; 3]) -> [Triangle; 2] {
        let c = [
            [-half, -half, z],
            [half, -half, z],
            [half, half, z],
            [-half, half, z],
        ];
        [
            Triangle {
                corners: [c[0], c[1], c[2]],
                color,
            },
            Triangle {
                corners: [c[0], c[2], c[3]],
                color,
            },
        ]
    }

    #[test]
    fn shared_edges_cover_every_sample_once() {
        // Two triangles of one square: no gaps or double coverage along the diagonal.
        let image = draw(&quad(0.0, 40.0, [255, 0, 0]), 100, 1);
        let covered = image.iter().filter(|p| p[3] == 255).count();
        assert_eq!(covered, 80 * 80);
        assert!(image.iter().all(|p| p[3] == 0 || p[3] == 255));
    }

    #[test]
    fn nearer_geometry_wins_regardless_of_order() {
        let near = quad(100.0, 20.0, [0, 255, 0]);
        let far = quad(-100.0, 40.0, [0, 0, 255]);
        for order in [[near, far], [far, near]] {
            let image = draw(&order.concat(), 100, 2);
            assert_eq!(image[50 * 100 + 50], [0, 255, 0, 255]);
            assert_eq!(image[50 * 100 + 22], [0, 0, 255, 255]);
        }
    }

    #[test]
    fn supersampling_antialiases_edges() {
        let tri = [Triangle {
            corners: [[-40.0, -40.0, 0.0], [40.0, -40.0, 0.0], [40.0, 33.0, 0.0]],
            color: [255, 255, 255],
        }];
        let image = draw(&tri, 100, 4);
        let partial = image.iter().filter(|p| p[3] > 0 && p[3] < 255).count();
        assert!(partial > 50, "{partial} partially covered pixels");
    }

    #[test]
    fn geometry_behind_the_camera_is_clipped() {
        // A floor running from the far distance past the eye.
        let floor = Triangle {
            corners: [
                [-500.0, 50.0, -2000.0],
                [500.0, 50.0, -2000.0],
                [0.0, 50.0, 3000.0],
            ],
            color: [9, 9, 9],
        };
        let image = draw(&[floor], 100, 1);
        assert!(image.iter().any(|p| p[3] == 255));
        // Entirely behind the eye: nothing.
        let behind = Triangle {
            corners: [
                [-50.0, 0.0, 1500.0],
                [50.0, 0.0, 1500.0],
                [0.0, 50.0, 1500.0],
            ],
            color: [9, 9, 9],
        };
        assert!(draw(&[behind], 100, 1).iter().all(|p| p[3] == 0));
    }

    /// The per-sample loop the span solver replaced.
    fn fill_reference(
        s: &Setup,
        origin: [i64; 2],
        limit: [i64; 2],
        stride: usize,
        color: &mut [u32],
        depth: &mut [f32],
    ) {
        let (x0, x1) = (s.bounds[0].max(origin[0]), s.bounds[2].min(limit[0]));
        let (y0, y1) = (s.bounds[1].max(origin[1]), s.bounds[3].min(limit[1]));
        let [a, b, c] = s.v;
        for y in y0..y1.max(y0) {
            for x in x0..x1.max(x0) {
                let p = [x * ONE + ONE / 2, y * ONE + ONE / 2];
                let inside = [(a, b), (b, c), (c, a)]
                    .iter()
                    .all(|&(p0, p1)| edge(p0, p1, p) - i64::from(!top_left(p0, p1)) >= 0);
                if inside {
                    let z = (s.depth[0]
                        + s.depth[2] * (y as f64 + 0.5)
                        + s.depth[1] * (x as f64 + 0.5)) as f32;
                    let i = (y - origin[1]) as usize * stride + (x - origin[0]) as usize;
                    if z > depth[i] {
                        depth[i] = z;
                        color[i] = s.color;
                    }
                }
            }
        }
    }

    #[test]
    fn span_solver_matches_per_sample_tests() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |range: i64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % (2 * range as u64)) as i64 - range
        };
        for _ in 0..400 {
            let mut v = [[0_i64; 2]; 3];
            for corner in &mut v {
                *corner = [next(40 * ONE) + 32 * ONE, next(40 * ONE) + 32 * ONE];
            }
            if edge(v[0], v[1], v[2]) == 0 {
                continue;
            }
            if edge(v[0], v[1], v[2]) < 0 {
                v.swap(1, 2);
            }
            let setup = Setup {
                v,
                bounds: [-100, -100, 200, 200],
                depth: [0.5, 1e-3, -2e-3],
                color: 7 | COVERED,
            };
            let (origin, limit, stride) = ([0, 0], [64, 64], 64);
            let mut expected = (vec![0; 64 * 64], vec![0.0; 64 * 64]);
            let mut actual = (vec![0; 64 * 64], vec![0.0; 64 * 64]);
            fill_reference(
                &setup,
                origin,
                limit,
                stride,
                &mut expected.0,
                &mut expected.1,
            );
            fill(&setup, origin, limit, stride, &mut actual.0, &mut actual.1);
            assert_eq!(expected, actual, "{v:?}");
        }
    }

    #[test]
    fn output_does_not_depend_on_scratch_history() {
        let triangles = [
            quad(0.0, 33.3, [10, 200, 30]),
            quad(50.0, 12.7, [200, 2, 3]),
        ]
        .concat();
        let a = draw(&triangles, 150, 3);
        let b = draw(&triangles, 150, 3);
        assert_eq!(a, b);
    }
}
