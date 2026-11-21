//! 2.5D spaces: flat planes placed, turned, and scaled in 3D and seen through one camera,
//! independent of HTML and pixels.
//!
//! A space (`data-hf-space` on a container) is a stage whose origin is the container's center, with
//! x to the right, y down, and z toward the viewer, as in CSS. Every element inside it is a 2D plane
//! with a hierarchical 3D transform: its resting layout offset and 2D pose (translation, scale, and
//! roll) plus a depth `z` and the tilts `rotateX` and `rotateY` ([`PlaneMotion`]), composed with its
//! parent's transform ([`math::Affine`]). The stage's camera ([`math::View`]) looks along -z from
//! `perspective` px in front of the stage plane (or orthographically), and its `data-hf-camera`
//! keyframes or `data-hf-shots` move it: `x`/`y` (the point it looks at, exactly as a classic
//! camera's focus), `zoom` (the lens), `z` (a dolly toward the scene, [`Stage::dolly`]), and
//! `rotateX`/`rotateY`/`rotate` (tilt, pan, and roll, [`Stage::orientation`]). Planes are ordered
//! exactly, splitting intersecting ones, by [`bsp`]. Everything is a pure function of time; the
//! renderer (`render/space.rs`) only orders and draws the planes it describes.

pub mod bsp;
pub mod math;

use crate::camera::curve::Curve;
use crate::model::Track;
use std::sync::Arc;

/// Default distance (px) from the camera to the stage plane, as CSS `perspective: 1000px`.
pub const DEFAULT_PERSPECTIVE: f32 = 1000.0;
/// Planes and pixels closer to the camera than this share of the perspective distance are clipped,
/// so nothing is magnified more than ten times by depth alone.
pub const NEAR_SHARE: f64 = 0.1;

/// How a space maps depth to the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// A pinhole camera `distance` px in front of the stage plane: a point at depth `z` is scaled by
    /// `distance / (distance - z)` around the point the camera looks at.
    Perspective { distance: f32 },
    /// Parallel projection: depth orders the planes and tilts foreshorten them, but nothing scales
    /// with distance (camera `z` has no effect).
    Orthographic,
}

/// `data-hf-space` on a container: its projection and its camera's dolly and orientation.
#[derive(Clone, Debug)]
pub struct Stage {
    pub projection: Projection,
    /// Whether the container also has `data-hf-camera` or `data-hf-shots`: its zoom and pan then
    /// move the 3D camera instead of scaling the container.
    pub camera: bool,
    /// Camera `z` over clip time (px toward the scene); `None` when it never dollies.
    pub dolly: Option<Curve>,
    /// Camera `[rotateX, rotateY, rotate]` over clip time (degrees); `None` when it never turns.
    pub orientation: Option<[Curve; 3]>,
}

impl Stage {
    /// Camera `[dolly, rotateX, rotateY, rotate]` `elapsed` seconds after the clip start (the ends
    /// hold).
    pub fn camera_at(&self, elapsed: f64) -> [f32; 4] {
        let dolly = self.dolly.as_ref().map_or(0.0, |c| c.eval(elapsed) as f32);
        let [x, y, roll] = self
            .orientation
            .as_ref()
            .map_or([0.0; 3], |o| o.each_ref().map(|c| c.eval(elapsed) as f32));
        [dolly, x, y, roll]
    }

    /// Whether the camera dollies or turns over time.
    pub fn is_animated(&self) -> bool {
        self.dolly.is_some() || self.orientation.is_some()
    }

    /// Distance from the camera below which planes are clipped (0 for orthographic spaces).
    pub fn near(&self) -> f64 {
        match self.projection {
            Projection::Perspective { distance } => f64::from(distance) * NEAR_SHARE,
            Projection::Orthographic => 0.0,
        }
    }
}

/// The 3D part of a plane's pose: depth and tilts, static (`data-hf-z`, `data-hf-rotate-x`,
/// `data-hf-rotate-y`) or keyframed (`z`, `rotateX`, `rotateY` in `data-hf-keyframes`).
#[derive(Clone, Debug, Default)]
pub struct PlaneMotion {
    /// Resting `[z, rotateX, rotateY]` (px, degrees, degrees).
    pub base: [f32; 3],
    /// Keyframed values of the same channels; an empty track keeps its resting value.
    pub tracks: [Track; 3],
    /// `data-hf-backface="hidden"`: the plane is not drawn while its back faces the camera.
    pub backface_hidden: bool,
}

impl PlaneMotion {
    /// `[z, rotateX, rotateY]` `elapsed` seconds after the clip start (the ends hold).
    pub fn eval(&self, elapsed: f64) -> [f32; 3] {
        [0, 1, 2].map(|i| self.tracks[i].eval(elapsed, self.base[i], true, true))
    }

    pub fn is_animated(&self) -> bool {
        self.tracks.iter().any(|track| !track.0.is_empty())
    }

    /// Whether the plane ever leaves the camera-facing orientation (it then needs a projective
    /// warp instead of a scaled, rolled draw).
    pub fn tilts(&self) -> bool {
        self.base[1] != 0.0
            || self.base[2] != 0.0
            || self.tracks[1..].iter().any(|track| !track.0.is_empty())
    }
}

/// The 2.5D role of one element: a stage, a plane with its own depth or tilt, both (a space
/// nested in another one), or neither (an element inside a space without 3D values of its own is
/// a plane at its parent's depth).
#[derive(Clone, Debug, Default)]
pub struct Depth {
    pub stage: Option<Arc<Stage>>,
    pub plane: Option<Arc<PlaneMotion>>,
}

impl Depth {
    /// A plane's `[z, rotateX, rotateY]`; zero for every other element.
    pub fn at(&self, elapsed: f64) -> [f32; 3] {
        self.plane
            .as_ref()
            .map_or([0.0; 3], |plane| plane.eval(elapsed))
    }

    /// A stage camera's `[dolly, rotateX, rotateY, rotate]`; zero for every other element.
    pub fn camera_at(&self, elapsed: f64) -> [f32; 4] {
        self.stage
            .as_ref()
            .map_or([0.0; 4], |stage| stage.camera_at(elapsed))
    }

    /// Whether any value of [`Self::at`] or [`Self::camera_at`] changes over time.
    pub fn is_animated(&self) -> bool {
        self.plane.as_ref().is_some_and(|plane| plane.is_animated())
            || self.stage.as_ref().is_some_and(|stage| stage.is_animated())
    }

    /// Keyframe times of the plane channels.
    pub fn times(&self) -> impl Iterator<Item = f64> + '_ {
        self.plane
            .iter()
            .flat_map(|plane| plane.tracks.iter())
            .flat_map(|track| track.0.iter().map(|point| point.at))
    }
}
