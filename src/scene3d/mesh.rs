//! Indexed triangle meshes and the procedural low-poly primitives (plane, box, pyramid, cone,
//! cylinder, and icosphere).
//!
//! A mesh is a list of positions in local px (x right, y down, z toward the viewer, as in CSS and
//! 2.5D spaces) and triangles that index them. The front of a triangle `(a, b, c)` is the side its
//! right-handed normal `(b - a) x (c - a)` points to (with y down, that is clockwise winding as
//! seen on screen). Every primitive is centered on its bounding box and faces outward; "up" is -y.
//! Generation is pure `f64` arithmetic rounded once to `f32`, so meshes are identical everywhere.

use anyhow::{bail, Result};

/// Most vertices one mesh may hold.
pub const MAX_VERTICES: usize = 65_536;
/// Most triangles one mesh may hold.
pub const MAX_MESH_TRIANGLES: usize = 100_000;
/// Radial segments accepted by cones and cylinders.
pub const SEGMENTS: std::ops::RangeInclusive<u32> = 3..=128;
/// Subdivision levels accepted by icospheres (0 is the 20-face icosahedron, 4 has 5120 faces).
pub const DETAIL: std::ops::RangeInclusive<u32> = 0..=4;

/// An indexed triangle mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

/// A procedural primitive and its dimensions (px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    /// A `width` x `depth` rectangle in the x-z plane facing up (-y).
    Plane {
        width: f64,
        depth: f64,
    },
    Box {
        size: [f64; 3],
    },
    /// A square-based pyramid: base `size[0]` x `size[2]` at the bottom, apex at the top.
    Pyramid {
        size: [f64; 3],
    },
    Cone {
        radius: f64,
        height: f64,
        segments: u32,
    },
    Cylinder {
        radius: f64,
        height: f64,
        segments: u32,
    },
    /// An icosphere: the icosahedron subdivided `detail` times and pushed onto the sphere.
    Sphere {
        radius: f64,
        detail: u32,
    },
}

impl Primitive {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Plane { .. } => "plane",
            Self::Box { .. } => "box",
            Self::Pyramid { .. } => "pyramid",
            Self::Cone { .. } => "cone",
            Self::Cylinder { .. } => "cylinder",
            Self::Sphere { .. } => "sphere",
        }
    }

    pub fn mesh(&self) -> Mesh {
        let mut b = Builder::default();
        match *self {
            Self::Plane { width, depth } => {
                let (x, z) = (width / 2.0, depth / 2.0);
                b.face(
                    &[[-x, 0.0, -z], [x, 0.0, -z], [x, 0.0, z], [-x, 0.0, z]],
                    [0.0, -1.0, 0.0],
                );
            }
            Self::Box { size } => {
                let [x, y, z] = size.map(|v| v / 2.0);
                let c = |i: usize| {
                    [
                        if i & 1 == 0 { -x } else { x },
                        if i & 2 == 0 { -y } else { y },
                        if i & 4 == 0 { -z } else { z },
                    ]
                };
                for quad in [
                    [0, 1, 3, 2],
                    [4, 5, 7, 6],
                    [0, 1, 5, 4],
                    [2, 3, 7, 6],
                    [0, 2, 6, 4],
                    [1, 3, 7, 5],
                ] {
                    b.convex(&quad.map(c));
                }
            }
            Self::Pyramid { size } => {
                let [x, y, z] = size.map(|v| v / 2.0);
                let base = [[-x, y, -z], [x, y, -z], [x, y, z], [-x, y, z]];
                let apex = [0.0, -y, 0.0];
                b.convex(&base);
                for i in 0..4 {
                    b.convex(&[base[i], base[(i + 1) % 4], apex]);
                }
            }
            Self::Cone {
                radius,
                height,
                segments,
            } => {
                let ring = ring(radius, height / 2.0, segments);
                let apex = [0.0, -height / 2.0, 0.0];
                b.convex(&ring);
                for i in 0..ring.len() {
                    b.convex(&[ring[i], ring[(i + 1) % ring.len()], apex]);
                }
            }
            Self::Cylinder {
                radius,
                height,
                segments,
            } => {
                let bottom = ring(radius, height / 2.0, segments);
                let top = ring(radius, -height / 2.0, segments);
                b.convex(&bottom);
                b.convex(&top);
                for i in 0..bottom.len() {
                    let j = (i + 1) % bottom.len();
                    b.convex(&[bottom[i], bottom[j], top[j], top[i]]);
                }
            }
            Self::Sphere { radius, detail } => {
                let (points, faces) = icosphere(detail);
                for [i, j, k] in faces {
                    let p = |n: usize| points[n].map(|v| v * radius);
                    b.convex(&[p(i), p(j), p(k)]);
                }
            }
        }
        b.mesh
    }
}

/// A closed polygon of `segments` points of radius `radius` at height `y`, starting at +x.
fn ring(radius: f64, y: f64, segments: u32) -> Vec<[f64; 3]> {
    (0..segments)
        .map(|i| {
            let (s, c) = (std::f64::consts::TAU * f64::from(i) / f64::from(segments)).sin_cos();
            [radius * c, y, radius * s]
        })
        .collect()
}

/// Unit icosphere: vertices and faces (unoriented; [`Builder::convex`] orients them).
fn icosphere(detail: u32) -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
    let t = (1.0 + 5f64.sqrt()) / 2.0;
    let mut points: Vec<[f64; 3]> = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ]
    .iter()
    .map(|&p| normalize(p))
    .collect();
    let mut faces: Vec<[usize; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..detail {
        let mut midpoints = std::collections::BTreeMap::new();
        let mut midpoint = |a: usize, b: usize, points: &mut Vec<[f64; 3]>| {
            *midpoints.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let (p, q) = (points[a], points[b]);
                points.push(normalize([
                    (p[0] + q[0]) / 2.0,
                    (p[1] + q[1]) / 2.0,
                    (p[2] + q[2]) / 2.0,
                ]));
                points.len() - 1
            })
        };
        let mut next = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let ab = midpoint(a, b, &mut points);
            let bc = midpoint(b, c, &mut points);
            let ca = midpoint(c, a, &mut points);
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        faces = next;
    }
    (points, faces)
}

fn normalize(p: [f64; 3]) -> [f64; 3] {
    let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    p.map(|v| v / length)
}

pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Accumulates faces as fans of triangles; each face gets its own vertices (flat shading never
/// shares normals across faces, so nothing is gained by welding).
#[derive(Default)]
struct Builder {
    mesh: Mesh,
}

impl Builder {
    /// Adds a convex planar face of a solid centered on the origin, oriented away from it.
    fn convex(&mut self, points: &[[f64; 3]]) {
        let n = points.len() as f64;
        let centroid = points.iter().fold([0.0; 3], |acc, p| {
            [acc[0] + p[0] / n, acc[1] + p[1] / n, acc[2] + p[2] / n]
        });
        self.face(points, centroid);
    }

    /// Adds a convex planar face whose front looks along `outward`.
    fn face(&mut self, points: &[[f64; 3]], outward: [f64; 3]) {
        let normal = cross(sub(points[1], points[0]), sub(points[2], points[0]));
        let flip = dot(normal, outward) < 0.0;
        let base = self.mesh.positions.len() as u32;
        self.mesh
            .positions
            .extend(points.iter().map(|p| p.map(|v| v as f32)));
        for i in 1..points.len() as u32 - 1 {
            let (b, c) = if flip { (i + 1, i) } else { (i, i + 1) };
            self.mesh.triangles.push([base, base + b, base + c]);
        }
    }
}

impl Mesh {
    /// A mesh from explicit positions and triangles, validated.
    pub fn custom(positions: Vec<[f32; 3]>, triangles: Vec<[u32; 3]>) -> Result<Self> {
        if positions.is_empty() || triangles.is_empty() {
            bail!("a custom mesh needs at least 3 vertices and one triangle");
        }
        if positions.len() > MAX_VERTICES {
            bail!("a mesh may have at most {MAX_VERTICES} vertices");
        }
        if triangles.len() > MAX_MESH_TRIANGLES {
            bail!("a mesh may have at most {MAX_MESH_TRIANGLES} triangles");
        }
        if let Some(p) = positions
            .iter()
            .find(|p| p.iter().any(|v| !v.is_finite() || v.abs() > 1e6))
        {
            bail!("vertex {p:?} must be finite and within 1000000 px");
        }
        if let Some(t) = triangles
            .iter()
            .find(|t| t.iter().any(|&i| i as usize >= positions.len()))
        {
            bail!(
                "triangle {t:?} indexes a vertex that does not exist ({} vertices)",
                positions.len()
            );
        }
        Ok(Self {
            positions,
            triangles,
        })
    }

    /// Local bounding box `[min, max]`.
    pub fn bounds(&self) -> [[f32; 3]; 2] {
        self.positions.iter().fold(
            [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]],
            |[lo, hi], p| {
                [
                    std::array::from_fn(|i| lo[i].min(p[i])),
                    std::array::from_fn(|i| hi[i].max(p[i])),
                ]
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outward(mesh: &Mesh) -> bool {
        mesh.triangles.iter().all(|t| {
            let p = t.map(|i| mesh.positions[i as usize].map(f64::from));
            let normal = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            let centroid = [0, 1, 2].map(|a| (p[0][a] + p[1][a] + p[2][a]) / 3.0);
            dot(normal, centroid) > 0.0
        })
    }

    #[test]
    fn primitives_face_outward_with_expected_counts() {
        let cases = [
            (
                Primitive::Box {
                    size: [2.0, 4.0, 6.0],
                },
                12,
            ),
            (
                Primitive::Pyramid {
                    size: [2.0, 2.0, 2.0],
                },
                6,
            ),
            (
                Primitive::Cone {
                    radius: 1.0,
                    height: 2.0,
                    segments: 8,
                },
                6 + 8,
            ),
            (
                Primitive::Cylinder {
                    radius: 1.0,
                    height: 2.0,
                    segments: 8,
                },
                12 + 16,
            ),
            (
                Primitive::Sphere {
                    radius: 3.0,
                    detail: 0,
                },
                20,
            ),
            (
                Primitive::Sphere {
                    radius: 3.0,
                    detail: 2,
                },
                320,
            ),
        ];
        for (primitive, count) in cases {
            let mesh = primitive.mesh();
            assert_eq!(mesh.triangles.len(), count, "{}", primitive.name());
            assert!(outward(&mesh), "{} faces inward", primitive.name());
        }
        let b = Primitive::Box {
            size: [2.0, 4.0, 6.0],
        }
        .mesh()
        .bounds();
        assert_eq!(b, [[-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]]);
    }

    #[test]
    fn plane_faces_up() {
        let mesh = Primitive::Plane {
            width: 4.0,
            depth: 2.0,
        }
        .mesh();
        assert_eq!(mesh.triangles.len(), 2);
        for t in &mesh.triangles {
            let p = t.map(|i| mesh.positions[i as usize].map(f64::from));
            assert!(cross(sub(p[1], p[0]), sub(p[2], p[0]))[1] < 0.0);
        }
    }

    #[test]
    fn custom_meshes_are_validated() {
        assert!(Mesh::custom(vec![[0.0; 3]; 3], vec![[0, 1, 3]]).is_err());
        assert!(Mesh::custom(vec![[f32::NAN, 0.0, 0.0]; 3], vec![[0, 1, 2]]).is_err());
        assert!(Mesh::custom(vec![[0.0; 3]; 3], vec![[0, 1, 2]]).is_ok());
    }
}
