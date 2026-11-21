//! Native low-poly 3D scenes (`data-hf-3d`), independent of HTML and pixels.
//!
//! A [`Scene3d`] is a hierarchy of [`Node`]s (groups, meshes, lights, and one camera) whose
//! translation, rotation, and scale are curves of the clip time ([`Transform`]). World coordinates
//! follow CSS and 2.5D spaces: the origin is the center of the element's box, x points right, y
//! down, and z toward the viewer; rotations use the CSS `rotateX`/`rotateY`/`rotate` senses and
//! compose as `translate3d() rotateY() rotateX() rotate() scale3d()` ([`crate::space::math::Affine`]).
//! The default camera sits `perspective` px in front of the origin looking along -z, so the plane
//! z = 0 maps one to one onto the element's CSS px, exactly like a 2.5D space at rest.
//!
//! [`Scene3d::frame`] evaluates the scene at one instant into a [`frame::Frame`]: world-space
//! triangles that survived back-face culling, each with its flat-shaded color ([`shade`]), plus
//! the camera ([`camera::View`]). That packet is the shared representation every backend draws:
//! the deterministic CPU rasterizer ([`raster`]) and, with the `gpu` feature, the wgpu backend
//! ([`gpu`]). [`backend`] picks one and falls back to the CPU.

pub mod backend;
pub mod camera;
pub mod frame;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod mesh;
pub mod raster;
pub mod shade;

use crate::camera::curve::Curve;
use crate::model::Color;
use crate::space::math::Affine;
use std::sync::Arc;

/// Most nodes one scene may hold.
pub const MAX_NODES: usize = 4096;
/// Most triangles one scene may hold across all its meshes.
pub const MAX_TRIANGLES: usize = 100_000;
/// Most directional lights one scene may hold.
pub const MAX_DIRECTIONAL_LIGHTS: usize = 8;
/// Supersampling factors per axis (`data-hf-3d-antialias`).
pub const ANTIALIAS: std::ops::RangeInclusive<u32> = 1..=4;

/// One animated scalar: a curve of clip time and whether it ever changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Channel {
    curve: Curve,
    animated: bool,
}

impl Channel {
    pub fn constant(value: f64) -> Self {
        Self {
            curve: Curve::constant(value),
            animated: false,
        }
    }

    /// A channel through `points` (`(at, value, interp)`, strictly increasing `at`).
    pub fn keyed(points: &[(f64, f64, crate::camera::curve::Interp)]) -> Self {
        let animated = points.windows(2).any(|w| w[0].1 != w[1].1);
        Self {
            curve: Curve::new(points),
            animated,
        }
    }

    pub fn eval(&self, t: f64) -> f64 {
        if self.animated {
            self.curve.eval(t)
        } else {
            self.curve.eval(0.0)
        }
    }

    pub fn is_animated(&self) -> bool {
        self.animated
    }
}

/// Translation, rotation (degrees: `rotateX`, `rotateY`, `rotate`), and per-axis scale.
#[derive(Clone, Debug, PartialEq)]
pub struct Transform {
    pub position: [Channel; 3],
    pub rotation: [Channel; 3],
    pub scale: [Channel; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: std::array::from_fn(|_| Channel::constant(0.0)),
            rotation: std::array::from_fn(|_| Channel::constant(0.0)),
            scale: std::array::from_fn(|_| Channel::constant(1.0)),
        }
    }
}

impl Transform {
    /// The local transform at clip time `t`.
    pub fn at(&self, t: f64) -> Affine {
        let p = self.position.each_ref().map(|c| c.eval(t));
        let [rx, ry, rz] = self.rotation.each_ref().map(|c| c.eval(t));
        let s = self.scale.each_ref().map(|c| c.eval(t));
        let mut local = Affine::pose(p, rx, ry, rz, 1.0);
        for row in &mut local.m {
            for (value, factor) in row.iter_mut().zip(s) {
                *value *= factor;
            }
        }
        local
    }

    fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.position
            .iter()
            .chain(&self.rotation)
            .chain(&self.scale)
    }
}

/// Which faces are skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cull {
    /// Faces whose back is toward the camera are not drawn (closed solids).
    Back,
    /// Both sides are drawn; the back is lit as if its normal were flipped.
    None,
}

/// How a mesh is shaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shading {
    /// The base color, unaffected by lights.
    Unlit,
    /// Flat Lambert: one diffuse value per face from the ambient and directional lights.
    Lambert,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub shading: Shading,
    /// Opaque base color (alpha is ignored).
    pub color: Color,
    pub cull: Cull,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LightKind {
    /// Light from every direction: adds `color x intensity` to every lit face.
    Ambient,
    /// Parallel light travelling along `direction` (local; turned by the node's rotation).
    Directional { direction: [f64; 3] },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub color: Color,
    pub intensity: Channel,
}

/// How the camera maps depth to the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Pinhole camera with a focal length of `focal` CSS px: a point `focal` px in front of the
    /// camera is drawn at its own size.
    Perspective { focal: f64 },
    /// Pinhole camera whose vertical field of view spans `degrees` of the viewport's height.
    FieldOfView { degrees: f64 },
    /// Parallel projection: nothing scales with distance.
    Orthographic,
}

/// What the camera aims at, if anything.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A fixed world point.
    Point([f64; 3]),
    /// The world origin of another node (by index).
    Node(usize),
}

/// The scene camera: placed by its node transform, turned by yaw (positive pans right), pitch
/// (positive tilts up), and roll (positive clockwise), as the 2.5D space camera.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraSpec {
    pub projection: Projection,
    /// Lens multiplier (1 = neutral): scales the focal length or the orthographic scale.
    pub zoom: Channel,
    /// `[yaw, pitch, roll]` in degrees.
    pub orientation: [Channel; 3],
    pub near: f64,
    pub far: f64,
    /// Aim at a point or node; yaw, pitch, and roll then turn the camera away from that aim.
    pub target: Option<Target>,
}

impl CameraSpec {
    /// The default camera: `focal` px in front of the origin, looking along -z.
    pub fn default_for(focal: f64) -> (Transform, Self) {
        let mut transform = Transform::default();
        transform.position[2] = Channel::constant(focal);
        (
            transform,
            Self {
                projection: Projection::Perspective { focal },
                zoom: Channel::constant(1.0),
                orientation: std::array::from_fn(|_| Channel::constant(0.0)),
                near: DEFAULT_NEAR,
                far: DEFAULT_FAR,
                target: None,
            },
        )
    }
}

/// Default near clipping distance (px).
pub const DEFAULT_NEAR: f64 = 10.0;
/// Default far clipping distance (px).
pub const DEFAULT_FAR: f64 = 50_000.0;
/// Default focal length (px), as CSS `perspective: 1000px` and 2.5D spaces.
pub const DEFAULT_FOCAL: f64 = 1000.0;

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Group,
    Mesh {
        mesh: Arc<mesh::Mesh>,
        material: Material,
        /// What the mesh was made from (`custom` for explicit vertices), for reports.
        shape: &'static str,
    },
    Light(Light),
    Camera(Box<CameraSpec>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// The element `id`, for reports and `target`.
    pub id: Option<String>,
    /// Parent node (always earlier in the list).
    pub parent: Option<usize>,
    pub kind: NodeKind,
    pub transform: Transform,
}

/// A whole `data-hf-3d` scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene3d {
    /// Nodes in document order: every parent precedes its children.
    pub nodes: Vec<Node>,
    /// The camera node, if the scene declares one (otherwise [`Scene3d::default_camera`]).
    pub camera: Option<usize>,
    /// Transform and settings of the camera used when no `<hf-camera>` is given.
    pub default_camera: (Transform, CameraSpec),
    /// Supersampling factor per axis (1 to 4): each output pixel averages `antialias^2` samples.
    pub antialias: u32,
    /// Whether no light was declared, so lit meshes use the default rig ([`shade::DEFAULT_RIG`]).
    pub default_lights: bool,
}

impl Scene3d {
    /// The camera transform and settings in use.
    pub fn camera(&self) -> (&Transform, &CameraSpec, Option<usize>) {
        match self.camera {
            Some(index) => match &self.nodes[index].kind {
                NodeKind::Camera(spec) => (&self.nodes[index].transform, spec, self.camera),
                _ => unreachable!("camera index points at a camera node"),
            },
            None => (&self.default_camera.0, &self.default_camera.1, None),
        }
    }

    /// World transform of every node at clip time `t`.
    pub fn world(&self, t: f64) -> Vec<Affine> {
        let mut world: Vec<Affine> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let local = node.transform.at(t);
            let placed = match node.parent {
                Some(parent) => world[parent].then(&local),
                None => local,
            };
            world.push(placed);
        }
        world
    }

    /// Triangles drawn per frame at most (before culling).
    pub fn triangle_count(&self) -> usize {
        self.nodes
            .iter()
            .map(|n| match &n.kind {
                NodeKind::Mesh { mesh, .. } => mesh.triangles.len(),
                _ => 0,
            })
            .sum()
    }

    /// Whether anything in the scene changes over time.
    pub fn is_animated(&self) -> bool {
        let (camera_transform, camera, _) = self.camera();
        camera_transform.channels().any(Channel::is_animated)
            || camera.zoom.is_animated()
            || camera.orientation.iter().any(Channel::is_animated)
            || self.nodes.iter().any(|n| {
                n.transform.channels().any(Channel::is_animated)
                    || matches!(&n.kind, NodeKind::Light(l) if l.intensity.is_animated())
            })
    }

    /// A key that changes whenever the picture can: 0 for a scene that never moves, otherwise
    /// the bits of every channel value at `t` folded together.
    pub fn key(&self, t: f64) -> u32 {
        if !self.is_animated() {
            return 0;
        }
        let (camera_transform, camera, _) = self.camera();
        let values = camera_transform
            .channels()
            .chain(std::iter::once(&camera.zoom))
            .chain(&camera.orientation)
            .chain(self.nodes.iter().flat_map(|n| {
                n.transform.channels().chain(match &n.kind {
                    NodeKind::Light(l) => Some(&l.intensity),
                    _ => None,
                })
            }))
            .filter(|c| c.is_animated())
            .map(|c| c.eval(t).to_bits());
        let hash = values.fold(0xcbf2_9ce4_8422_2325_u64, |h, bits| {
            (h ^ bits).wrapping_mul(0x0000_0100_0000_01b3)
        });
        ((hash >> 32) as u32 ^ hash as u32) | 1
    }
}

impl Projection {
    /// Focal length in CSS px for a viewport `height` px tall (`None` for orthographic).
    pub fn focal(&self, height: f64) -> Option<f64> {
        match *self {
            Self::Perspective { focal } => Some(focal),
            Self::FieldOfView { degrees } => {
                Some(height / 2.0 / (degrees.to_radians() / 2.0).tan())
            }
            Self::Orthographic => None,
        }
    }
}
