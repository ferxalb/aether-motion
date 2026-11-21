//! 3D transform and camera math for 2.5D spaces: affine plane transforms composed down the element
//! tree, the stage camera's projection, and the homography that maps a tilted plane onto the screen.
//!
//! All arithmetic is in `f64` and depends only on its inputs, so every frame, thread, and render
//! part projects identically.

use super::Projection;

/// `p' = m * p + t`: a plane's local px (from its center, z = 0 on the plane) to stage px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub m: [[f64; 3]; 3],
    pub t: [f64; 3],
}

impl Affine {
    pub const IDENTITY: Self = Self {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };

    /// A local transform equivalent to CSS
    /// `translate3d(x, y, z) rotateY(ry) rotateX(rx) rotate(roll) scale(s)` (degrees; rotations are
    /// about the element center, `roll` clockwise on screen).
    pub fn pose(offset: [f64; 3], rotate_x: f64, rotate_y: f64, roll: f64, scale: f64) -> Self {
        let rz = rotation_z(roll);
        let rx = rotation_x(rotate_x);
        let ry = rotation_y(rotate_y);
        let mut m = multiply(&ry, &multiply(&rx, &rz));
        for row in &mut m {
            for value in row {
                *value *= scale;
            }
        }
        Self { m, t: offset }
    }

    /// `self` applied after `local` (a child's transform inside this one).
    pub fn then(&self, local: &Self) -> Self {
        Self {
            m: multiply(&self.m, &local.m),
            t: self.apply(local.t),
        }
    }

    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let mut out = self.t;
        for (row, value) in out.iter_mut().enumerate() {
            *value += self.m[row][0] * p[0] + self.m[row][1] * p[1] + self.m[row][2] * p[2];
        }
        out
    }

    /// Direction of the plane's local axis `i` (0 = x, 1 = y, 2 = normal), scaled.
    pub fn axis(&self, i: usize) -> [f64; 3] {
        [self.m[0][i], self.m[1][i], self.m[2][i]]
    }
}

fn rotation_x(degrees: f64) -> [[f64; 3]; 3] {
    let (s, c) = degrees.to_radians().sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}

fn rotation_y(degrees: f64) -> [[f64; 3]; 3] {
    let (s, c) = degrees.to_radians().sin_cos();
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}

fn rotation_z(degrees: f64) -> [[f64; 3]; 3] {
    if degrees == 0.0 {
        return Affine::IDENTITY.m;
    }
    let (s, c) = degrees.to_radians().sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

fn multiply(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, value) in row.iter_mut().enumerate() {
            *value = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
        }
    }
    out
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The stage camera at one instant, mapping stage px to screen (composition px).
///
/// The camera sits at `eye` (perspective: `distance - z` px in front of the stage plane, over the
/// point it looks at; orthographic: on the stage plane) and turns by `orientation`: `rotateX`
/// (positive tilts up), `rotateY` (positive pans right), and `rotate` (positive rolls clockwise,
/// so the scene turns counterclockwise on screen). A point `p` is seen at view coordinates
/// `q = turn * (p - eye)`, looking along `-z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub projection: Projection,
    /// Screen position of the stage center (composition px).
    pub origin: [f64; 2],
    /// Screen px per stage px at depth 0: the stage's own scale times the camera zoom.
    pub scale: f64,
    /// Point the camera looks at (`x`, `y`, stage px) and its dolly toward the scene (`z`).
    pub camera: [f64; 3],
    /// World-to-view rotation (the inverse of the camera's orientation).
    turn: [[f64; 3]; 3],
    /// Whether the camera pans or tilts (planes then never face it exactly).
    tilted: bool,
    /// Camera roll in degrees (clockwise).
    roll: f64,
    /// Distance below which points are clipped (0: never).
    pub near: f64,
}

impl View {
    /// A camera looking at `camera` (`[x, y, dolly]`) turned by `orientation`
    /// (`[rotateX, rotateY, roll]`, degrees).
    pub fn new(
        projection: Projection,
        origin: [f64; 2],
        scale: f64,
        camera: [f64; 3],
        orientation: [f64; 3],
        near: f64,
    ) -> Self {
        let [tilt, pan, roll] = orientation;
        // The camera's own orientation, then its inverse (transpose) maps the world into view.
        let own = multiply(
            &rotation_y(-pan),
            &multiply(&rotation_x(-tilt), &rotation_z(roll)),
        );
        let turn = std::array::from_fn(|r| std::array::from_fn(|c| own[c][r]));
        Self {
            projection,
            origin,
            scale,
            camera,
            turn,
            tilted: tilt != 0.0 || pan != 0.0,
            roll,
            near,
        }
    }

    /// The camera's position (perspective) or, for orthographic views, the point it turns around.
    fn eye(&self) -> [f64; 3] {
        match self.projection {
            Projection::Perspective { distance } => [
                self.camera[0],
                self.camera[1],
                f64::from(distance) - self.camera[2],
            ],
            Projection::Orthographic => [self.camera[0], self.camera[1], 0.0],
        }
    }

    /// View coordinates of a direction (no translation).
    fn rotate(&self, d: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|r| dot(self.turn[r], d))
    }

    /// View coordinates of `p`: right, down, and toward the viewer from the camera.
    pub fn view(&self, p: [f64; 3]) -> [f64; 3] {
        let eye = self.eye();
        self.rotate([p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]])
    }

    /// Whether the camera pans or tilts, so no plane faces it exactly.
    pub fn is_tilted(&self) -> bool {
        self.tilted
    }

    /// Camera roll (degrees, clockwise); a facing plane turns by its negative on screen.
    pub fn roll(&self) -> f64 {
        self.roll
    }

    /// Distance from the camera to `p` along the view axis (perspective), or the depth behind
    /// the stage (orthographic: only the order matters). Larger is farther.
    pub fn distance(&self, p: [f64; 3]) -> f64 {
        -self.view(p)[2]
    }

    /// Whether `p` is in front of the near clipping distance.
    pub fn visible(&self, p: [f64; 3]) -> bool {
        match self.projection {
            Projection::Perspective { .. } => self.distance(p) > self.near,
            Projection::Orthographic => true,
        }
    }

    /// Screen px per stage px at `p` (the depth magnification times [`Self::scale`]).
    pub fn magnification(&self, p: [f64; 3]) -> f64 {
        match self.projection {
            Projection::Perspective { distance } => {
                self.scale * f64::from(distance) / self.distance(p)
            }
            Projection::Orthographic => self.scale,
        }
    }

    /// Screen position (composition px) of a visible point.
    pub fn project(&self, p: [f64; 3]) -> [f64; 2] {
        let g = self.magnification(p);
        let q = self.view(p);
        [self.origin[0] + g * q[0], self.origin[1] + g * q[1]]
    }

    /// The eye as a point (`w = 1`, perspective) or the direction toward the viewer (`w = 0`).
    pub fn eye_point(&self) -> ([f64; 3], f64) {
        match self.projection {
            Projection::Perspective { .. } => (self.eye(), 1.0),
            Projection::Orthographic => {
                // The view's +z axis in world coordinates: the turn's third row.
                (self.turn[2], 0.0)
            }
        }
    }

    /// Whether the camera sees the front of the plane through `point` with normal `normal`
    /// (`> 0`), its back (`< 0`), or its edge (`0`).
    pub fn facing(&self, point: [f64; 3], normal: [f64; 3]) -> f64 {
        let (eye, w) = self.eye_point();
        let towards = [
            eye[0] - w * point[0],
            eye[1] - w * point[1],
            eye[2] - w * point[2],
        ];
        dot(towards, normal)
    }

    /// The homography that maps a plane's local px `(u, v, 1)` to homogeneous output pixels
    /// `(x w, y w, w)` at render scale `k`. For perspective, `w` is the camera distance of the
    /// point, so the inverse yields `1 / distance` as its third coordinate.
    pub fn homography(&self, plane: &Affine, k: f64) -> [[f64; 3]; 3] {
        let (a0, a1, t) = (
            self.rotate(plane.axis(0)),
            self.rotate(plane.axis(1)),
            self.view(plane.t),
        );
        match self.projection {
            Projection::Perspective { distance } => {
                let f = f64::from(distance);
                let w = [-a0[2], -a1[2], -t[2]];
                let row = |axis: usize| {
                    let o = self.origin[axis] * k;
                    let s = self.scale * f * k;
                    [
                        o * w[0] + s * a0[axis],
                        o * w[1] + s * a1[axis],
                        o * w[2] + s * t[axis],
                    ]
                };
                [row(0), row(1), w]
            }
            Projection::Orthographic => {
                let row = |axis: usize| {
                    let s = self.scale * k;
                    [
                        s * a0[axis],
                        s * a1[axis],
                        k * (self.origin[axis] + self.scale * t[axis]),
                    ]
                };
                [row(0), row(1), [0.0, 0.0, 1.0]]
            }
        }
    }
}

/// Inverse of a 3x3 matrix, or `None` when it is (nearly) singular: a plane seen exactly edge-on.
pub fn invert(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let cofactor =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adj = [
        [
            cofactor(1, 2, 1, 2),
            -cofactor(0, 2, 1, 2),
            cofactor(0, 1, 1, 2),
        ],
        [
            -cofactor(1, 2, 0, 2),
            cofactor(0, 2, 0, 2),
            -cofactor(0, 1, 0, 2),
        ],
        [
            cofactor(1, 2, 0, 1),
            -cofactor(0, 2, 0, 1),
            cofactor(0, 1, 0, 1),
        ],
    ];
    let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
    let scale = m
        .iter()
        .flatten()
        .fold(0.0_f64, |high, value| high.max(value.abs()));
    if !det.is_finite() || det.abs() <= 1e-12 * scale * scale * scale {
        return None;
    }
    Some(adj.map(|row| row.map(|value| value / det)))
}

/// The part of a convex polygon (plane-local `(u, v)`) where `distance(u, v) >= near`, with
/// `distance` affine: `d0 + du * u + dv * v`.
pub fn clip_near(polygon: &[[f64; 2]], d: [f64; 3], near: f64) -> Vec<[f64; 2]> {
    clip_half_plane(polygon, [d[0] - near, d[1], d[2]])
}

/// The part of a convex polygon where `c0 + cu * u + cv * v >= 0` (Sutherland–Hodgman against
/// one edge).
pub fn clip_half_plane(polygon: &[[f64; 2]], [c0, cu, cv]: [f64; 3]) -> Vec<[f64; 2]> {
    let value = |p: [f64; 2]| c0 + cu * p[0] + cv * p[1];
    let mut out = Vec::with_capacity(polygon.len() + 1);
    for (i, &current) in polygon.iter().enumerate() {
        let previous = polygon[(i + polygon.len() - 1) % polygon.len()];
        let (vc, vp) = (value(current), value(previous));
        if (vc >= 0.0) != (vp >= 0.0) {
            let t = vp / (vp - vc);
            out.push([
                previous[0] + (current[0] - previous[0]) * t,
                previous[1] + (current[1] - previous[1]) * t,
            ]);
        }
        if vc >= 0.0 {
            out.push(current);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective(camera: [f64; 3]) -> View {
        View::new(
            Projection::Perspective { distance: 1000.0 },
            [960.0, 540.0],
            1.0,
            camera,
            [0.0; 3],
            100.0,
        )
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn depth_scales_like_css_perspective() {
        let view = perspective([0.0; 3]);
        assert!(close(view.magnification([0.0, 0.0, 0.0]), 1.0));
        assert!(close(view.magnification([0.0, 0.0, -1000.0]), 0.5));
        assert!(close(view.magnification([0.0, 0.0, 500.0]), 2.0));
        let p = view.project([100.0, 0.0, -1000.0]);
        assert!(close(p[0], 1010.0) && close(p[1], 540.0));
    }

    #[test]
    fn camera_at_depth_zero_matches_a_classic_camera() {
        // A classic camera puts stage point p at origin + zoom * (p - focus).
        let mut view = perspective([120.0, -40.0, 0.0]);
        view.scale = 1.6;
        let p = view.project([300.0, 200.0, 0.0]);
        assert!(close(p[0], 960.0 + 1.6 * 180.0));
        assert!(close(p[1], 540.0 + 1.6 * 240.0));
    }

    #[test]
    fn dolly_magnifies_and_trucking_gives_parallax() {
        let view = perspective([0.0, 0.0, 500.0]);
        assert!(close(view.magnification([0.0; 3]), 2.0));
        let truck = perspective([100.0, 0.0, 0.0]);
        let near = truck.project([0.0, 0.0, 500.0])[0] - 960.0;
        let far = truck.project([0.0, 0.0, -1000.0])[0] - 960.0;
        assert!(close(near, -200.0) && close(far, -50.0));
        assert!(!perspective([0.0; 3]).visible([0.0, 0.0, 950.0]));
    }

    #[test]
    fn orthographic_ignores_depth_for_scale() {
        let view = View {
            projection: Projection::Orthographic,
            ..perspective([0.0; 3])
        };
        assert!(close(view.magnification([0.0, 0.0, -5000.0]), 1.0));
        assert!(view.distance([0.0, 0.0, -10.0]) > view.distance([0.0, 0.0, 10.0]));
    }

    #[test]
    fn transforms_compose_down_the_tree() {
        let parent = Affine::pose([100.0, 0.0, -200.0], 0.0, 90.0, 0.0, 2.0);
        let child = Affine::pose([10.0, 0.0, 0.0], 0.0, 0.0, 0.0, 1.0);
        let world = parent.then(&child);
        // A quarter turn about y sends the child's +x offset (scaled by 2) toward -z.
        let c = world.t;
        assert!(close(c[0], 100.0) && close(c[1], 0.0) && close(c[2], -220.0));
        let rolled = Affine::pose([0.0; 3], 0.0, 0.0, 90.0, 1.0).apply([1.0, 0.0, 0.0]);
        assert!(close(rolled[0], 0.0) && close(rolled[1], 1.0));
    }

    #[test]
    fn homography_matches_projection_and_inverts() {
        let view = perspective([30.0, 10.0, 200.0]);
        let plane = Affine::pose([50.0, -20.0, -300.0], 25.0, -40.0, 10.0, 1.5);
        let k = 2.0;
        let h = homography_of(&view, &plane, k);
        let inverse = invert(&h).unwrap();
        for (u, v) in [(0.0, 0.0), (120.0, -80.0), (-200.0, 60.0)] {
            let world = plane.apply([u, v, 0.0]);
            let expected = view.project(world);
            let x = h[0][0] * u + h[0][1] * v + h[0][2];
            let y = h[1][0] * u + h[1][1] * v + h[1][2];
            let w = h[2][0] * u + h[2][1] * v + h[2][2];
            assert!(close(x / w, expected[0] * k) && close(y / w, expected[1] * k));
            assert!(close(w, view.distance(world)));
            let back = apply(&inverse, [x / w, y / w]);
            assert!((back[0] / back[2] - u).abs() < 1e-6 && (back[1] / back[2] - v).abs() < 1e-6);
            assert!((1.0 / back[2] - w).abs() < 1e-6);
        }
        assert!(invert(&[[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]]).is_none());
    }

    fn homography_of(view: &View, plane: &Affine, k: f64) -> [[f64; 3]; 3] {
        view.homography(plane, k)
    }

    fn apply(m: &[[f64; 3]; 3], p: [f64; 2]) -> [f64; 3] {
        [0, 1, 2].map(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2])
    }

    #[test]
    fn back_faces_and_near_clipping() {
        let view = perspective([0.0; 3]);
        let front = Affine::pose([0.0; 3], 0.0, 30.0, 0.0, 1.0);
        let back = Affine::pose([0.0; 3], 0.0, 180.0, 0.0, 1.0);
        assert!(view.facing(front.t, front.axis(2)) > 0.0);
        assert!(view.facing(back.t, back.axis(2)) < 0.0);
        // A floor reaching toward the camera: the part past the near distance is cut.
        let square = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let clipped = clip_near(&square, [0.0, 0.0, 1.0], 0.0);
        assert_eq!(clipped.len(), 4);
        assert!(clipped.iter().all(|p| p[1] >= 0.0));
        assert!(clip_near(&square, [-5.0, 0.0, 1.0], 0.0).is_empty());
    }
}
