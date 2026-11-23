//! The shared per-frame representation of a scene: the camera and the world-space triangles that
//! face it, each with its flat-shaded color. Built once per drawn instant on the CPU in `f64`, it is
//! what the CPU rasterizer and the GPU backend both draw, so they agree on geometry, culling, and
//! color and differ only in how they cover pixels. Also estimates shutter travel from scene poses.

use super::camera::View;
use super::mesh::{cross, dot, sub};
use super::shade::Lighting;
use super::{Cull, NodeKind, Scene3d};

/// A triangle to draw: world px corners and its opaque sRGB color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub corners: [[f64; 3]; 3],
    pub color: [u8; 3],
}

/// One instant of a scene, ready to draw.
#[derive(Clone, Debug)]
pub struct Frame {
    pub view: View,
    pub triangles: Vec<Triangle>,
}

impl Scene3d {
    /// Evaluates the scene at clip time `t` into `triangles` (cleared first, so its allocation is
    /// reused across frames) and returns the camera.
    pub fn frame_into(&self, t: f64, triangles: &mut Vec<Triangle>) -> View {
        triangles.clear();
        let world = self.world(t);
        let view = View::of(self, &world, t);
        let mut lighting = if self.default_lights {
            Lighting::default_rig()
        } else {
            Lighting::default()
        };
        for (node, placed) in self.nodes.iter().zip(&world) {
            if let NodeKind::Light(light) = &node.kind {
                let turn = |d: [f64; 3]| {
                    std::array::from_fn(|r| {
                        placed.m[r][0] * d[0] + placed.m[r][1] * d[1] + placed.m[r][2] * d[2]
                    })
                };
                lighting.add(
                    &light.kind,
                    light.color,
                    light.intensity.eval(t).max(0.0),
                    turn,
                );
            }
        }
        for (node, placed) in self.nodes.iter().zip(&world) {
            let NodeKind::Mesh { mesh, material, .. } = &node.kind else {
                continue;
            };
            let m = placed.m;
            let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
            if det == 0.0 || !det.is_finite() {
                continue;
            }
            // A mirroring transform turns the winding inside out; flip it back.
            let mirrored = det < 0.0;
            for tri in &mesh.triangles {
                let mut corners =
                    tri.map(|i| placed.apply(mesh.positions[i as usize].map(f64::from)));
                if mirrored {
                    corners.swap(1, 2);
                }
                let normal = cross(sub(corners[1], corners[0]), sub(corners[2], corners[0]));
                let length = dot(normal, normal).sqrt();
                if length <= 1e-12 || !length.is_finite() {
                    continue;
                }
                let mut unit = normal.map(|v| v / length);
                let facing = dot(unit, view.toward_viewer(corners[0]));
                if facing <= 0.0 {
                    if material.cull == Cull::Back || facing == 0.0 {
                        continue;
                    }
                    unit = unit.map(|v| -v);
                }
                triangles.push(Triangle {
                    corners,
                    color: lighting.shade(material, unit),
                });
            }
        }
        view
    }

    /// Evaluates the scene at clip time `t`.
    pub fn frame(&self, t: f64) -> Frame {
        let mut triangles = Vec::new();
        let view = self.frame_into(t, &mut triangles);
        Frame { view, triangles }
    }

    /// Estimate on-screen travel (CSS px) for motion-blur sample counts.
    pub fn screen_travel(&self, a: f64, b: f64, size: [f64; 2]) -> f64 {
        let mut world = self.world(a);
        let mut view = View::of(self, &world, a);
        let mut travel = 0.0;
        if let Some(bake) = &self.physics {
            let rate = f64::from(super::physics::TICK_RATE);
            let start = ((a * rate).floor().max(0.0) as usize).saturating_add(1);
            let end = (b * rate).ceil().min(bake.ticks() as f64 + 1.0) as usize;
            // An impact can reverse a body's motion between the shutter endpoints.
            for tick in start..end {
                let time = tick as f64 / rate;
                let next = self.world(time);
                let next_view = View::of(self, &next, time);
                travel += self.projected_travel(&world, &view, &next, &next_view, size);
                world = next;
                view = next_view;
            }
        }
        let next = self.world(b);
        let next_view = View::of(self, &next, b);
        travel += self.projected_travel(&world, &view, &next, &next_view, size);
        travel.min(size[0].hypot(size[1]))
    }

    fn projected_travel(
        &self,
        wa: &[crate::space::math::Affine],
        va: &View,
        wb: &[crate::space::math::Affine],
        vb: &View,
        size: [f64; 2],
    ) -> f64 {
        let mut travel: f64 = 0.0;
        for (i, node) in self.nodes.iter().enumerate() {
            let NodeKind::Mesh { mesh, .. } = &node.kind else {
                continue;
            };
            let [lo, hi] = mesh.bounds();
            for corner in 0..8 {
                let p = [0, 1, 2].map(|axis| {
                    f64::from(if corner >> axis & 1 == 0 {
                        lo[axis]
                    } else {
                        hi[axis]
                    })
                });
                let (Some(pa), Some(pb)) = (
                    va.project_css(wa[i].apply(p), size),
                    vb.project_css(wb[i].apply(p), size),
                ) else {
                    continue;
                };
                travel = travel.max((pa[0] - pb[0]).hypot(pa[1] - pb[1]));
            }
        }
        travel
    }
}
