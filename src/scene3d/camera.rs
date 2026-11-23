//! The 3D camera at one instant ([`View`]) and the projection shared by every backend
//! ([`ClipMatrix`]): world px to clip coordinates for one raster target.
//!
//! View coordinates are x right, y down, and z toward the viewer from the eye, which looks along
//! -z; the distance of a point is `-z`. Depth is stored reversed (1 at the near plane, 0 at the
//! far plane), linear in screen space, so the CPU and GPU depth buffers hold the same quantity.

use super::mesh::{cross, dot, sub};
use super::{CameraSpec, Projection, Scene3d, Target};
use crate::space::math::Affine;

/// The camera at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub eye: [f64; 3],
    /// Unit world directions of the view axes: right, down, and back (toward the viewer).
    pub axes: [[f64; 3]; 3],
    pub projection: Projection,
    pub zoom: f64,
    pub near: f64,
    pub far: f64,
}

fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(v, v).sqrt();
    (length > 1e-12 && length.is_finite()).then(|| v.map(|c| c / length))
}

impl View {
    /// World ray whose parameter is camera-axis distance from the eye plane.
    pub(super) fn ray(&self, placement: &Placement, target: [f64; 2]) -> ([f64; 3], [f64; 3]) {
        let [x, y] = [0, 1]
            .map(|i| (target[i] - placement.origin[i]) / placement.scale - placement.size[i] * 0.5);
        let lateral = |i| (x * self.axes[0][i] + y * self.axes[1][i]) / self.zoom;
        match self.projection.focal(placement.size[1]) {
            Some(focal) => (
                self.eye,
                std::array::from_fn(|i| lateral(i) / focal - self.axes[2][i]),
            ),
            None => (
                std::array::from_fn(|i| self.eye[i] + lateral(i)),
                self.axes[2].map(|v| -v),
            ),
        }
    }

    /// Camera-axis distance represented by one reversed-depth sample.
    pub(super) fn distance_at_depth(&self, reversed_depth: f32) -> f64 {
        if reversed_depth == 0.0 {
            return self.far;
        }
        let z = f64::from(reversed_depth);
        let (a, b) = self.depth_terms();
        let distance = match self.projection {
            Projection::Orthographic => (z - a) / b,
            _ => a / (z - b),
        };
        distance.clamp(self.near, self.far)
    }

    /// The scene's camera at clip time `t`, given the world transforms of the nodes at `t`.
    pub fn of(scene: &Scene3d, world: &[Affine], t: f64) -> Self {
        let (transform, spec, index) = scene.camera();
        let placed = match index {
            Some(i) => world[i],
            None => transform.at(t),
        };
        Self::place(spec, &placed, world, t)
    }

    fn place(spec: &CameraSpec, placed: &Affine, world: &[Affine], t: f64) -> Self {
        let eye = placed.t;
        let [mut yaw, mut pitch, roll] = spec.orientation.each_ref().map(|c| c.eval(t));
        let aim = spec.target.as_ref().map(|target| match target {
            Target::Point(p) => *p,
            Target::Node(i) => world[*i].t,
        });
        // Orthonormal base frame: the node's (scale and shear ignored), or level when aiming.
        let mut frame = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        match aim.and_then(|aim| normalize(sub(aim, eye))) {
            // The aim as yaw and pitch angles (forward = (sin Y cos P, -sin P, -cos Y cos P)), so
            // the yaw and pitch offsets turn around world-vertical and the horizon stays level.
            Some(forward) => {
                yaw += forward[0].atan2(-forward[2]).to_degrees();
                pitch += (-forward[1]).clamp(-1.0, 1.0).asin().to_degrees();
            }
            None => {
                if let (Some(x), Some(y)) = (normalize(placed.axis(0)), normalize(placed.axis(1))) {
                    if let Some(z) = normalize(cross(x, y)) {
                        frame = [x, cross(z, x), z];
                    }
                }
            }
        }
        // Yaw pans right, pitch tilts up, roll turns clockwise: the 2.5D space camera senses.
        let own = Affine::pose([0.0; 3], -pitch, -yaw, roll, 1.0);
        let axes = std::array::from_fn(|i| {
            let local = own.axis(i);
            std::array::from_fn(|r| {
                frame[0][r] * local[0] + frame[1][r] * local[1] + frame[2][r] * local[2]
            })
        });
        Self {
            eye,
            axes,
            projection: spec.projection,
            zoom: spec.zoom.eval(t),
            near: spec.near,
            far: spec.far,
        }
    }

    /// View coordinates of a world point.
    pub fn view(&self, p: [f64; 3]) -> [f64; 3] {
        let d = sub(p, self.eye);
        self.axes.map(|axis| dot(axis, d))
    }

    /// Unit world direction from a surface point toward the viewer.
    pub fn toward_viewer(&self, p: [f64; 3]) -> [f64; 3] {
        match self.projection {
            Projection::Orthographic => self.axes[2],
            _ => sub(self.eye, p),
        }
    }

    /// Screen position (CSS px from the top-left of a `size` viewport) of a world point in front
    /// of the near plane.
    pub fn project_css(&self, p: [f64; 3], size: [f64; 2]) -> Option<[f64; 2]> {
        let q = self.view(p);
        let distance = -q[2];
        if distance < self.near {
            return None;
        }
        let g = match self.projection.focal(size[1]) {
            Some(focal) => focal * self.zoom / distance,
            None => self.zoom,
        };
        Some([size[0] / 2.0 + g * q[0], size[1] / 2.0 + g * q[1]])
    }

    /// Reversed depth `(a, b)`: `z_clip = a + b * distance`, so `z_clip / w` is 1 at the near plane
    /// and 0 at the far plane (with `w` the distance for perspective, 1 for orthographic).
    fn depth_terms(&self) -> (f64, f64) {
        let (n, f) = (self.near, self.far);
        match self.projection {
            Projection::Orthographic => (f / (f - n), -1.0 / (f - n)),
            _ => (n * f / (f - n), -n / (f - n)),
        }
    }
}

/// Where a viewport sits on a raster target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Target px of the viewport's top-left corner.
    pub origin: [f64; 2],
    /// Target px per CSS px.
    pub scale: f64,
    /// The viewport's CSS size.
    pub size: [f64; 2],
}

/// World px to clip coordinates `(x, y, z, w)` for a raster target whose NDC square covers the
/// target px rectangle `[x0, x0 + w) x [y0, y0 + h)` (x right, y up in NDC, as WebGPU expects).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipMatrix(pub [[f64; 4]; 4]);

impl ClipMatrix {
    pub fn new(view: &View, placement: &Placement, rect: [f64; 4]) -> Self {
        let [x0, y0, w, h] = rect;
        let k = placement.scale;
        // Target px of the viewport center.
        let cx = placement.origin[0] + k * placement.size[0] / 2.0 - x0;
        let cy = placement.origin[1] + k * placement.size[1] / 2.0 - y0;
        let (a, b) = view.depth_terms();
        // Rows in view coordinates (q_x, q_y, q_z, 1).
        let rows: [[f64; 4]; 4] = match view.projection.focal(placement.size[1]) {
            Some(focal) => {
                let g = k * focal * view.zoom;
                // w = distance = -q_z; ndc_x = 2 (cx + g q_x / w) / W - 1.
                [
                    [2.0 * g / w, 0.0, -(2.0 * cx / w - 1.0), 0.0],
                    [0.0, -2.0 * g / h, 2.0 * cy / h - 1.0, 0.0],
                    [0.0, 0.0, -b, a],
                    [0.0, 0.0, -1.0, 0.0],
                ]
            }
            None => {
                let g = k * view.zoom;
                [
                    [2.0 * g / w, 0.0, 0.0, 2.0 * cx / w - 1.0],
                    [0.0, -2.0 * g / h, 0.0, 1.0 - 2.0 * cy / h],
                    [0.0, 0.0, -b, a],
                    [0.0, 0.0, 0.0, 1.0],
                ]
            }
        };
        // Compose with world -> view: q = axes * (p - eye).
        Self(rows.map(|row| {
            let shift: f64 = (0..3).map(|i| row[i] * dot(view.axes[i], view.eye)).sum();
            std::array::from_fn(|c| {
                if c < 3 {
                    (0..3).map(|i| row[i] * view.axes[i][c]).sum()
                } else {
                    row[3] - shift
                }
            })
        }))
    }

    pub fn apply(&self, p: [f64; 3]) -> [f64; 4] {
        self.0
            .map(|row| row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d::{Channel, Transform};

    fn scene_with(transform: Transform, spec: CameraSpec) -> Scene3d {
        Scene3d {
            nodes: vec![],
            camera: None,
            default_camera: (transform, spec),
            antialias: 1,
            default_lights: true,
            physics: None,
        }
    }

    #[test]
    fn default_camera_maps_the_origin_plane_one_to_one() {
        let (transform, spec) = CameraSpec::default_for(1000.0);
        let scene = scene_with(transform, spec);
        let view = View::of(&scene, &[], 0.0);
        let size = [800.0, 600.0];
        let p = view.project_css([100.0, -50.0, 0.0], size).unwrap();
        assert!((p[0] - 500.0).abs() < 1e-9 && (p[1] - 250.0).abs() < 1e-9);
        // Halfway to the camera, things look twice as large.
        let q = view.project_css([100.0, 0.0, 500.0], size).unwrap();
        assert!((q[0] - 600.0).abs() < 1e-9);
        // The clip matrix agrees with the CSS projection on a target at 2x.
        let placement = Placement {
            origin: [10.0, 20.0],
            scale: 2.0,
            size,
        };
        let m = ClipMatrix::new(&view, &placement, [0.0, 0.0, 2000.0, 1500.0]);
        let c = m.apply([100.0, -50.0, 0.0]);
        let px = (c[0] / c[3] + 1.0) / 2.0 * 2000.0;
        let py = (1.0 - c[1] / c[3]) / 2.0 * 1500.0;
        assert!((px - (10.0 + 2.0 * 500.0)).abs() < 1e-9);
        assert!((py - (20.0 + 2.0 * 250.0)).abs() < 1e-9);
        assert!((c[3] - 1000.0).abs() < 1e-9);
        // Reversed depth: 1 at near, 0 at far.
        let near = m.apply([0.0, 0.0, 1000.0 - view.near]);
        let far = m.apply([0.0, 0.0, 1000.0 - view.far]);
        assert!((near[2] / near[3] - 1.0).abs() < 1e-9);
        assert!((far[2] / far[3]).abs() < 1e-9);
    }

    #[test]
    fn yaw_pans_right_and_pitch_tilts_up() {
        let (transform, mut spec) = CameraSpec::default_for(1000.0);
        spec.orientation[0] = Channel::constant(10.0);
        let view = View::of(&scene_with(transform.clone(), spec.clone()), &[], 0.0);
        // Panning right moves a point straight ahead to the left of the screen.
        assert!(view.project_css([0.0, 0.0, 0.0], [100.0, 100.0]).unwrap()[0] < 50.0);
        spec.orientation = std::array::from_fn(|_| Channel::constant(0.0));
        spec.orientation[1] = Channel::constant(10.0);
        let view = View::of(&scene_with(transform, spec), &[], 0.0);
        assert!(view.project_css([0.0, 0.0, 0.0], [100.0, 100.0]).unwrap()[1] > 50.0);
    }

    #[test]
    fn sample_ray_and_reversed_depth_reconstruct_the_same_world_point() {
        let placement = Placement {
            origin: [-31.25, 15.75],
            scale: 1.7,
            size: [440.0, 260.0],
        };
        let target = [130.375, 181.625];
        for projection in [
            Projection::Orthographic,
            Projection::FieldOfView { degrees: 53.0 },
        ] {
            let (transform, mut spec) = CameraSpec::default_for(1000.0);
            spec.projection = projection;
            spec.zoom = Channel::constant(1.4);
            spec.orientation[0] = Channel::constant(11.0);
            spec.orientation[2] = Channel::constant(-7.0);
            let view = View::of(&scene_with(transform, spec), &[], 0.0);
            let rect = [23.0, 48.0, 395.0, 240.0];
            let m = ClipMatrix::new(&view, &placement, rect);
            let (origin, direction) = view.ray(&placement, target);
            let p = std::array::from_fn(|i| origin[i] + direction[i] * 1413.0);
            let [x, y, z, w] = m.apply(p);
            assert!((rect[0] + rect[2] * (x / w + 1.0) * 0.5 - target[0]).abs() < 1e-9);
            assert!((rect[1] + rect[3] * (1.0 - y / w) * 0.5 - target[1]).abs() < 1e-9);
            assert!((view.distance_at_depth((z / w) as f32) - 1413.0).abs() < 0.002);
            assert_eq!(view.distance_at_depth(0.0), view.far);
        }
    }

    #[test]
    fn target_keeps_the_aim_centered() {
        let (mut transform, mut spec) = CameraSpec::default_for(1000.0);
        transform.position = [
            Channel::constant(600.0),
            Channel::constant(-400.0),
            Channel::constant(700.0),
        ];
        spec.target = Some(Target::Point([50.0, 20.0, -100.0]));
        let view = View::of(&scene_with(transform, spec), &[], 0.0);
        let p = view
            .project_css([50.0, 20.0, -100.0], [640.0, 360.0])
            .unwrap();
        assert!((p[0] - 320.0).abs() < 1e-9 && (p[1] - 180.0).abs() < 1e-9);
        // Up stays up: a point above the target is drawn above the center.
        assert!(
            view.project_css([50.0, -80.0, -100.0], [640.0, 360.0])
                .unwrap()[1]
                < 180.0
        );
    }

    #[test]
    fn yaw_away_from_a_target_keeps_the_horizon_level() {
        let (mut transform, mut spec) = CameraSpec::default_for(1000.0);
        transform.position = [
            Channel::constant(700.0),
            Channel::constant(-500.0),
            Channel::constant(900.0),
        ];
        spec.target = Some(Target::Point([0.0, 0.0, 0.0]));
        spec.orientation[0] = Channel::constant(-15.0);
        let view = View::of(&scene_with(transform, spec), &[], 0.0);
        // The camera's right axis has no vertical component, so nothing rolls: far points at one
        // height project to one screen row.
        let size = [1920.0, 1080.0];
        let a = view
            .project_css([-60_000.0, -500.0, -30_000.0], size)
            .unwrap();
        let b = view
            .project_css([-30_000.0, -500.0, -60_000.0], size)
            .unwrap();
        assert!((a[1] - b[1]).abs() < 0.5, "{a:?} {b:?}");
        assert!(view.axes[0][1].abs() < 1e-12);
    }
}
