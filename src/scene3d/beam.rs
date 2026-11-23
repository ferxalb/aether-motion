//! A bounded native light shaft, integrated along camera rays with optional box shadows.

use super::mesh::{cross, dot, sub};
use super::{
    camera::{Placement, View},
    raster::Region,
};
use crate::{model::Color, space::math::Affine};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub struct Beam {
    pub direction: [f64; 3],
    pub length: f64,
    pub radius: f64,
    pub density: f64,
    pub color: Color,
    pub occluder: Option<usize>,
}

impl Beam {
    pub(super) fn endpoints(&self, placed: Affine) -> [[f64; 3]; 2] {
        let direction = placed.m.map(|row| dot(row, self.direction));
        let norm = libm::sqrt(dot(direction, direction));
        let tip = if norm > 1e-12 {
            point_at(placed.t, direction, self.length / norm)
        } else {
            placed.t
        };
        [placed.t, tip]
    }
}

pub(super) struct Prepared {
    light: [f64; 3],
    axis: [f64; 3],
    length: f64,
    radius: f64,
    density: f64,
    pub color: [f64; 3],
    bounds: [[f64; 3]; 2],
    blocker: Option<Blocker>,
}

pub(super) struct Resolve<'a> {
    beam: &'a Prepared,
    view: &'a View,
    placement: &'a Placement,
    region: &'a Region,
    screen_bounds: Option<[[f64; 2]; 2]>,
}

impl<'a> Resolve<'a> {
    pub fn new(
        beam: &'a Prepared,
        view: &'a View,
        placement: &'a Placement,
        region: &'a Region,
    ) -> Self {
        let mut screen_bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
        let projected = (0..8).all(|corner| {
            let p = [0, 1, 2].map(|axis| beam.bounds[(corner >> axis) & 1][axis]);
            let Some(p) = view.project_css(p, placement.size) else {
                return false;
            };
            for (i, css) in p.into_iter().enumerate() {
                let target = placement.origin[i] + css * placement.scale - region.rect[i] as f64;
                screen_bounds[0][i] = screen_bounds[0][i].min(target);
                screen_bounds[1][i] = screen_bounds[1][i].max(target);
            }
            true
        });
        Self {
            beam,
            view,
            placement,
            region,
            screen_bounds: projected.then_some(screen_bounds),
        }
    }

    pub fn covers_pixel(&self, [x, y]: [usize; 2]) -> bool {
        self.screen_bounds.is_none_or(|[lo, hi]| {
            let x = x as f64;
            let y = y as f64;
            x <= hi[0] && x + 1.0 >= lo[0] && y <= hi[1] && y + 1.0 >= lo[1]
        })
    }

    /// Average premultiplied linear radiance, then return the viewport's straight-alpha sRGB.
    pub fn pixel(
        &self,
        [x, y]: [usize; 2],
        stride: usize,
        depths: &[f32],
        colors: &[u32],
    ) -> [u8; 4] {
        let ss = self.region.antialias as usize;
        let mut sum = [0.0; 3];
        let mut legacy = [0_u32; 3];
        let mut covered = 0_u32;
        let mut alpha = 0.0;
        let mut lit = false;
        for sy in 0..ss {
            for sx in 0..ss {
                let [u, v] = [x * ss + sx, y * ss + sy];
                let i = sy * stride + sx;
                let depth = depths[i];
                let opaque = depth > 0.0;
                let [x0, y0, x1, y1] = self.region.samples;
                let present = (x0..x1).contains(&(u as i64)) && (y0..y1).contains(&(v as i64));
                let opacity = if present {
                    let sample = [u, v].map(|s| (s as f64 + 0.5) / ss as f64);
                    let target = [0, 1].map(|axis| self.region.rect[axis] as f64 + sample[axis]);
                    let (origin, direction) = self.view.ray(self.placement, target);
                    self.beam.opacity(
                        origin,
                        direction,
                        [self.view.near, self.view.distance_at_depth(depth)],
                    )
                } else {
                    0.0
                };
                lit |= opacity > 0.0;
                alpha += if opaque { 1.0 } else { opacity };
                if opaque {
                    covered += 1;
                }
                for c in 0..3 {
                    let encoded = (colors[i] >> (8 * c)) as u8;
                    let surface = if opaque {
                        legacy[c] += u32::from(encoded);
                        linear(encoded)
                    } else {
                        0.0
                    };
                    sum[c] += surface * (1.0 - opacity) + self.beam.color[c] * opacity;
                }
            }
        }
        let count = (ss * ss) as u32;
        if !lit {
            if covered == 0 {
                return [0; 4];
            }
            let [r, g, b] = legacy.map(|v| ((v + covered / 2) / covered) as u8);
            return [r, g, b, ((covered * 255 + count / 2) / count) as u8];
        }
        let [r, g, b] = sum.map(|v| encoded(v / alpha));
        [r, g, b, (alpha * 255.0 / f64::from(count) + 0.5) as u8]
    }
}

struct Blocker {
    world_to_local: [[f64; 3]; 3],
    center: [f64; 3],
    bounds: [[f64; 3]; 2],
    light: [f64; 3],
}

impl Blocker {
    fn new(placed: Affine, bounds: [[f32; 3]; 2], light: [f64; 3]) -> Option<Self> {
        let [x, y, z] = [0, 1, 2].map(|i| placed.axis(i));
        let cross_yz = cross(y, z);
        let determinant = dot(x, cross_yz);
        if determinant.abs() < 1e-12 {
            return None;
        }
        let world_to_local =
            [cross_yz, cross(z, x), cross(x, y)].map(|row| row.map(|value| value / determinant));
        Some(Self {
            world_to_local,
            center: placed.t,
            bounds: bounds.map(|p| p.map(f64::from)),
            light: world_to_local.map(|row| dot(row, sub(light, placed.t))),
        })
    }

    fn blocks(&self, point: [f64; 3]) -> bool {
        let direction = self
            .world_to_local
            .map(|row| dot(row, sub(point, self.center)));
        interval(
            self.light,
            sub(direction, self.light),
            self.bounds,
            [1e-6, 1.0],
        )
        .is_some()
    }
}

impl Prepared {
    pub fn new(
        beam: &Beam,
        placed: Affine,
        blocker: Option<(Affine, [[f32; 3]; 2])>,
    ) -> Option<Self> {
        let mut axis = placed.m.map(|row| dot(row, beam.direction));
        let norm = libm::sqrt(dot(axis, axis));
        if norm < 1e-12 || beam.density == 0.0 {
            return None;
        }
        axis = axis.map(|v| v / norm);
        let light = placed.t;
        let end = point_at(light, axis, beam.length);
        // Project the end circle onto each world axis to bound the cone with its light origin.
        let extent = axis.map(|v| beam.radius * libm::sqrt((1.0 - v * v).max(0.0)));
        let bounds = [
            std::array::from_fn(|i| light[i].min(end[i] - extent[i])),
            std::array::from_fn(|i| light[i].max(end[i] + extent[i])),
        ];
        Some(Self {
            light,
            axis,
            length: beam.length,
            radius: beam.radius,
            density: beam.density,
            color: [beam.color.r, beam.color.g, beam.color.b].map(linear),
            bounds,
            blocker: blocker.and_then(|(pose, bounds)| Blocker::new(pose, bounds, light)),
        })
    }

    /// Integrate illuminated density only. The cone has a soft radial edge and fades at its end.
    pub fn opacity(&self, origin: [f64; 3], direction: [f64; 3], depths: [f64; 2]) -> f64 {
        let Some([start, end]) = interval(origin, direction, self.bounds, depths) else {
            return 0.0;
        };
        const STEPS: usize = 20;
        let step = (end - start) / STEPS as f64;
        let mut density = 0.0;
        for i in 0..STEPS {
            let p = point_at(origin, direction, start + (i as f64 + 0.5) * step);
            let rel = sub(p, self.light);
            let along = dot(rel, self.axis);
            if along <= 0.0 || along >= self.length {
                continue;
            }
            let outer = self.radius * along / self.length;
            let radial_sq = (dot(rel, rel) - along * along).max(0.0) / (outer * outer);
            if radial_sq >= 1.0
                || self
                    .blocker
                    .as_ref()
                    .is_some_and(|blocker| blocker.blocks(p))
            {
                continue;
            }
            let radial = (1.0 - radial_sq) * (1.0 - radial_sq);
            let tail = ((self.length - along) / (0.25 * self.length)).clamp(0.0, 1.0);
            density += radial * tail;
        }
        let world_step = step * libm::sqrt(dot(direction, direction));
        -libm::expm1(-self.density * world_step * density)
    }
}

fn point_at(origin: [f64; 3], direction: [f64; 3], t: f64) -> [f64; 3] {
    std::array::from_fn(|i| origin[i] + direction[i] * t)
}

fn interval(
    origin: [f64; 3],
    direction: [f64; 3],
    [lo, hi]: [[f64; 3]; 2],
    [mut start, mut end]: [f64; 2],
) -> Option<[f64; 2]> {
    for i in 0..3 {
        if direction[i].abs() <= 1e-12 {
            if origin[i] < lo[i] || origin[i] > hi[i] {
                return None;
            }
        } else {
            let a = (lo[i] - origin[i]) / direction[i];
            let b = (hi[i] - origin[i]) / direction[i];
            start = start.max(a.min(b));
            end = end.min(a.max(b));
        }
    }
    (end > start).then_some([start, end])
}

pub(super) fn linear(encoded: u8) -> f64 {
    static TABLE: OnceLock<[f64; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let c = i as f64 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                libm::pow((c + 0.055) / 1.055, 2.4)
            }
        })
    })[encoded as usize]
}

pub(super) fn encoded(value: f64) -> u8 {
    let c = value.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * libm::pow(c, 1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}
