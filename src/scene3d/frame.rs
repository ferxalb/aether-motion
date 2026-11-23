//! The shared per-frame representation of a scene: the camera and the world-space triangles that
//! face it, each with its flat-shaded color. Built once per drawn instant on the CPU in `f64`, it is
//! what the CPU rasterizer and the GPU backend both draw, so they agree on geometry, culling, and
//! color and differ only in how they cover pixels. Triangles of meshes with a [`super::look::Look`]
//! carry no color of their own: they point at a [`ShadedTri`] that the CPU rasterizer shades per
//! sample. Also estimates shutter travel from scene poses.

use super::camera::View;
use super::look::sample::{Shaded, ShadedTri};
use super::mesh::{cross, dot, sub};
use super::shade::{Caster, Lighting};
use super::{Cull, NodeKind, Scene3d};

/// A triangle to draw: world px corners and its opaque sRGB color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub corners: [[f64; 3]; 3],
    pub color: [u8; 3],
    /// Index into [`Shaded::tris`] for per-sample shading, or [`Triangle::FLAT`] to fill with
    /// `color`.
    pub shaded: u32,
}

impl Triangle {
    /// Marks a triangle filled with its flat color.
    pub const FLAT: u32 = u32::MAX;

    /// A triangle filled with one color.
    pub fn flat(corners: [[f64; 3]; 3], color: [u8; 3]) -> Self {
        Self {
            corners,
            color,
            shaded: Self::FLAT,
        }
    }
}

/// One instant of a scene, ready to draw.
#[derive(Debug)]
pub struct Frame {
    pub view: View,
    pub triangles: Vec<Triangle>,
    pub shaded: Shaded,
}

/// The cofactor matrix of the linear part of `m`, signed so it keeps pointing away from the
/// surface under mirroring: it turns a local normal into a world normal under any scale.
fn normal_matrix(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let c =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let cof = [
        [c(1, 2, 1, 2), -c(1, 2, 0, 2), c(1, 2, 0, 1)],
        [-c(0, 2, 1, 2), c(0, 2, 0, 2), -c(0, 2, 0, 1)],
        [c(0, 1, 1, 2), -c(0, 1, 0, 2), c(0, 1, 0, 1)],
    ];
    let det = m[0][0] * cof[0][0] + m[0][1] * cof[0][1] + m[0][2] * cof[0][2];
    let sign = if det < 0.0 { -1.0 } else { 1.0 };
    cof.map(|row| row.map(|v| v * sign))
}

impl Scene3d {
    /// Evaluates the scene at clip time `t` into `triangles` (cleared first, so its allocation is
    /// reused across frames) and returns the camera.
    pub fn frame_into(&self, t: f64, triangles: &mut Vec<Triangle>, shaded: &mut Shaded) -> View {
        triangles.clear();
        shaded.tris.clear();
        shaded.time = t;
        shaded.progress.clear();
        shaded
            .progress
            .extend(self.looks.iter().map(|look| look.progress.eval(t)));
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
                    light.shadows,
                    placed.t,
                    turn,
                );
            }
        }
        let mut casters: Vec<Caster> = Vec::new();
        if lighting.directional_shadows.iter().any(|&s| s)
            || lighting.points.iter().any(|p| p.shadows)
        {
            for (node, placed) in self.nodes.iter().zip(&world) {
                if let NodeKind::Mesh { mesh, .. } = &node.kind {
                    casters.extend(mesh.triangles.iter().map(|tri| {
                        tri.map(|i| placed.apply(mesh.positions[i as usize].map(f64::from)))
                    }));
                }
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
            let look = material.look.map(|i| (i, &self.looks[i as usize]));
            let normals = normal_matrix(&m);
            let turn = |n: [f32; 3]| {
                let n = n.map(f64::from);
                let w: [f64; 3] = std::array::from_fn(|r| {
                    normals[r][0] * n[0] + normals[r][1] * n[1] + normals[r][2] * n[2]
                });
                let length = dot(w, w).sqrt();
                if length > 1e-12 {
                    w.map(|v| v / length)
                } else {
                    n
                }
            };
            for tri in &mesh.triangles {
                let mut index = *tri;
                if mirrored {
                    index.swap(1, 2);
                }
                let corners =
                    index.map(|i| placed.apply(mesh.positions[i as usize].map(f64::from)));
                let normal = cross(sub(corners[1], corners[0]), sub(corners[2], corners[0]));
                let length = dot(normal, normal).sqrt();
                if length <= 1e-12 || !length.is_finite() {
                    continue;
                }
                let mut unit = normal.map(|v| v / length);
                let facing = dot(unit, view.toward_viewer(corners[0]));
                let mut back = false;
                if facing <= 0.0 {
                    if material.cull == Cull::Back || facing == 0.0 {
                        continue;
                    }
                    unit = unit.map(|v| -v);
                    back = true;
                }
                let centroid =
                    std::array::from_fn(|i| (corners[0][i] + corners[1][i] + corners[2][i]) / 3.0);
                if let Some((look_index, look)) = look {
                    let uv: [[f64; 2]; 3] = index.map(|i| {
                        mesh.uvs
                            .get(i as usize)
                            .map_or([0.0; 2], |uv| uv.map(f64::from))
                    });
                    let corner_normals: [[f64; 3]; 3] = if look.smooth && !mesh.normals.is_empty() {
                        index.map(|i| {
                            let n = turn(mesh.normals[i as usize]);
                            if back {
                                n.map(|v| -v)
                            } else {
                                n
                            }
                        })
                    } else {
                        [unit; 3]
                    };
                    let (tangent, bitangent) = ShadedTri::frame(&corners, &uv);
                    triangles.push(Triangle {
                        corners,
                        color: [0; 3],
                        shaded: shaded.tris.len() as u32,
                    });
                    shaded.tris.push(ShadedTri {
                        look: look_index,
                        material: *material,
                        tint: [material.color.r, material.color.g, material.color.b]
                            .map(super::shade::to_linear),
                        uv,
                        normal: corner_normals,
                        tangent,
                        bitangent,
                        shadowed: lighting.shadow_mask(unit, centroid, &casters),
                    });
                    continue;
                }
                let to_eye = view.toward_viewer(centroid);
                let to_eye_len = dot(to_eye, to_eye).sqrt();
                let to_eye = if to_eye_len > 1e-12 {
                    to_eye.map(|v| v / to_eye_len)
                } else {
                    unit
                };
                triangles.push(Triangle::flat(
                    corners,
                    lighting.shade(material, unit, centroid, to_eye, &casters),
                ));
            }
        }
        shaded.lighting = lighting;
        view
    }

    /// Evaluates the scene at clip time `t`.
    pub fn frame(&self, t: f64) -> Frame {
        let mut triangles = Vec::new();
        let mut shaded = Shaded::default();
        let view = self.frame_into(t, &mut triangles, &mut shaded);
        Frame {
            view,
            triangles,
            shaded,
        }
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
            if let NodeKind::Beam(beam) = &node.kind {
                for (a, b) in beam.endpoints(wa[i]).into_iter().zip(beam.endpoints(wb[i])) {
                    if let (Some(pa), Some(pb)) = (va.project_css(a, size), vb.project_css(b, size))
                    {
                        travel = travel.max((pa[0] - pb[0]).hypot(pa[1] - pb[1]));
                    }
                }
                continue;
            }
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
