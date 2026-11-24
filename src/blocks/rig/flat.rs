//! The 2D human rig (`human-2d`): rigid segments drawn as inline SVG and animated with CSS
//! `@keyframes`, posed by forward kinematics computed here.
//!
//! The skeleton is a joint tree (for kinematics and paint order); the output is flat: one `<svg>`
//! per segment, a square centered on its joint, moved and turned to the joint's world frame at each
//! sample. Flat siblings keep every rotated layer small (nested rotated boxes would have to hold
//! whole limbs at any angle) and make paint order explicit, so a side view's far limbs pass behind
//! the body. Limb segments are the exact tangent hull of their two joint circles, so a bent elbow
//! or knee is always a clean round joint, never a kink or a stretched shape. Samples are refined
//! until no joint turns more than 2 degrees between keyframes, and the body is placed on the ground
//! by the same kinematics that place the drawn feet.

use super::motion::View;
use super::options::{Flavor, Options, Travel};
use super::place::{placed, stride, Contact, Stride};
use super::skeleton::*;
use super::{fmt, keep, Out};
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Description shown in the block catalog.
pub const DESCRIPTION: &str = "Controllable 2D human, side or front view, with rigid jointed limbs that never stretch or kink. Pick a motion (idle, walk, run, wave, cheer, jump, sit, point), chain motions with sequence, or key joint angles with timeline; offset any joint by name. Feet stay on the ground, and travel=auto walks without foot sliding. x and y are the ground point under the figure; size is its height in px.";

/// A drawable piece: SVG markup in joint-local coordinates (facing right, y down) and its box.
struct Shape {
    /// Markup; `{G}` stands for a gradient id unique in the document.
    svg: String,
    bbox: [f64; 4],
}

/// A joint's angle (CSS degrees, facing right) as a function of the pose.
type Turn = Box<dyn Fn(&Pose) -> f64>;

/// A joint: placed at `offset` in its parent's frame, turned by `angle` (CSS degrees, facing
/// right), drawing `items` in order.
struct Joint {
    name: String,
    offset: (f64, f64),
    angle: Turn,
    items: Vec<Item>,
    /// Sole points in this joint's frame: (foot id, x, y).
    contacts: Vec<(usize, f64, f64)>,
}

enum Item {
    Shape(Shape),
    Joint(Joint),
}

impl Joint {
    fn new(name: &str, offset: (f64, f64), angle: impl Fn(&Pose) -> f64 + 'static) -> Self {
        Self {
            name: name.to_string(),
            offset,
            angle: Box::new(angle),
            items: Vec::new(),
            contacts: Vec::new(),
        }
    }

    fn shape(mut self, shape: Shape) -> Self {
        self.items.push(Item::Shape(shape));
        self
    }

    fn child(mut self, joint: Joint) -> Self {
        self.items.push(Item::Joint(joint));
        self
    }

    /// Contact points under `pose`, in the frame of this joint's parent.
    fn contacts(&self, pose: &Pose, out: &mut Vec<(usize, f64, f64)>) {
        let (sin, cos) = (self.angle)(pose).to_radians().sin_cos();
        let place = |x: f64, y: f64| -> (f64, f64) {
            (
                self.offset.0 + x * cos - y * sin,
                self.offset.1 + x * sin + y * cos,
            )
        };
        for (foot, x, y) in &self.contacts {
            let (px, py) = place(*x, *y);
            out.push((*foot, px, py));
        }
        for item in &self.items {
            if let Item::Joint(j) = item {
                let mut inner = Vec::new();
                j.contacts(pose, &mut inner);
                for (foot, x, y) in inner {
                    let (px, py) = place(x, y);
                    out.push((foot, px, py));
                }
            }
        }
    }
}

const SHADE_STOPS: &str = r##"<stop offset="0" stop-color="#000" stop-opacity="0.28"/><stop offset="0.34" stop-color="#000" stop-opacity="0"/><stop offset="0.6" stop-color="#fff" stop-opacity="0.16"/><stop offset="0.78" stop-color="#fff" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.2"/>"##;

/// A filled outline with a cylindrical shade across `(x1, y1) -> (x2, y2)`; far pieces are darker.
fn filled(d: &str, bbox: [f64; 4], color: &str, far: bool, shade: [f64; 4]) -> Shape {
    let mut svg = format!(
        r#"<defs><linearGradient id="{{G}}" gradientUnits="userSpaceOnUse" x1="{}" y1="{}" x2="{}" y2="{}">{SHADE_STOPS}</linearGradient></defs><path d="{d}" fill="{color}"/><path d="{d}" fill="url(#{{G}})"/>"#,
        fmt(shade[0]),
        fmt(shade[1]),
        fmt(shade[2]),
        fmt(shade[3]),
    );
    if far {
        let _ = write!(svg, r##"<path d="{d}" fill="#000" fill-opacity="0.28"/>"##);
    }
    Shape { svg, bbox }
}

/// A flat filled outline without shading.
fn flat(d: &str, bbox: [f64; 4], color: &str, opacity: f64) -> Shape {
    let svg = if opacity < 1.0 {
        format!(
            r#"<path d="{d}" fill="{color}" fill-opacity="{}"/>"#,
            fmt(opacity)
        )
    } else {
        format!(r#"<path d="{d}" fill="{color}"/>"#)
    };
    Shape { svg, bbox }
}

fn ellipse_d(cx: f64, cy: f64, rx: f64, ry: f64) -> String {
    format!(
        "M{} {}A{} {} 0 1 0 {} {}A{} {} 0 1 0 {} {}Z",
        fmt(cx - rx),
        fmt(cy),
        fmt(rx),
        fmt(ry),
        fmt(cx + rx),
        fmt(cy),
        fmt(rx),
        fmt(ry),
        fmt(cx - rx),
        fmt(cy)
    )
}

fn ellipse_box(cx: f64, cy: f64, rx: f64, ry: f64) -> [f64; 4] {
    [cx - rx, cy - ry, cx + rx, cy + ry]
}

/// The outline of two circles joined by their outer tangents: a tapered capsule from `a` (radius
/// `ra`) to `b` (radius `rb`). It contains both circles exactly, so a child segment's circle at the
/// shared joint lies on this one's and the joint stays round at every angle.
pub fn hull(a: (f64, f64), ra: f64, b: (f64, f64), rb: f64) -> String {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let sin = ((ra - rb) / len).clamp(-0.999, 0.999);
    let cos = (1.0 - sin * sin).sqrt();
    let point = |c: (f64, f64), r: f64, side: f64| -> (f64, f64) {
        (
            c.0 + r * (sin * ux + side * cos * nx),
            c.1 + r * (sin * uy + side * cos * ny),
        )
    };
    let (a1, b1) = (point(a, ra, 1.0), point(b, rb, 1.0));
    let (b2, a2) = (point(b, rb, -1.0), point(a, ra, -1.0));
    // The far arc around b spans less than half a turn when the limb narrows toward b.
    let large_b = u8::from(rb > ra);
    let large_a = u8::from(ra >= rb);
    format!(
        "M{} {}L{} {}A{} {} 0 {large_b} 0 {} {}L{} {}A{} {} 0 {large_a} 0 {} {}Z",
        fmt(a1.0),
        fmt(a1.1),
        fmt(b1.0),
        fmt(b1.1),
        fmt(rb),
        fmt(rb),
        fmt(b2.0),
        fmt(b2.1),
        fmt(a2.0),
        fmt(a2.1),
        fmt(ra),
        fmt(ra),
        fmt(a1.0),
        fmt(a1.1)
    )
}

/// A limb segment from `a` to `b` with joint radii `ra` and `rb`.
fn limb(a: (f64, f64), ra: f64, b: (f64, f64), rb: f64, color: &str, far: bool) -> Shape {
    let r = ra.max(rb);
    let bbox = [
        (a.0 - ra).min(b.0 - rb),
        (a.1 - ra).min(b.1 - rb),
        (a.0 + ra).max(b.0 + rb),
        (a.1 + ra).max(b.1 + rb),
    ];
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    let (nx, ny) = (-dy / len, dx / len);
    let shade = [a.0 + r * nx, a.1 + r * ny, a.0 - r * nx, a.1 - r * ny];
    filled(&hull(a, ra, b, rb), bbox, color, far, shade)
}

/// A point of a cubic Bézier curve.
fn cubic(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// A point of a quadratic Bézier curve.
fn quad(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    (
        u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
        u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
    )
}

/// Contact points of one foot with ids unique to it: the first two are the flat sole's ends (the
/// heel and toe in a side view).
fn sole_points(foot: usize, points: &[(f64, f64)]) -> Vec<(usize, f64, f64)> {
    points
        .iter()
        .enumerate()
        .map(|(i, (x, y))| (foot * 64 + i, *x, *y))
        .collect()
}

/// Builds the joint tree of the figure.
fn skeleton(view: View, o: &Options) -> Joint {
    let p = &o.palette;
    let side = view == View::Side;
    // In a side view (facing right) the right side is near; the left one is drawn behind the body.
    let near = |s: Side| side && s.tag == "r";

    let arm = |s: Side, offset: (f64, f64)| -> Joint {
        let far = side && !near(s);
        let sign = s.sign;
        let (shoulder, elbow, wrist): (Turn, Turn, Turn) = if side {
            (
                Box::new(move |q: &Pose| -q[s.shoulder]),
                Box::new(move |q: &Pose| -q[s.elbow]),
                Box::new(move |q: &Pose| -q[s.wrist]),
            )
        } else {
            (
                Box::new(move |q: &Pose| sign * q[s.shoulder_side]),
                Box::new(move |q: &Pose| sign * q[s.elbow]),
                Box::new(move |q: &Pose| sign * q[s.wrist]),
            )
        };
        let hand = Joint {
            name: format!("{}-wrist", s.tag),
            offset: (0.0, FOREARM),
            angle: wrist,
            items: Vec::new(),
            contacts: Vec::new(),
        }
        .shape(limb(
            (0.0, 0.0),
            R_WRIST,
            (0.0, HAND),
            R_FINGERS,
            &p.skin,
            far,
        ));
        let forearm = Joint {
            name: format!("{}-elbow", s.tag),
            offset: (0.0, UPPER_ARM),
            angle: elbow,
            items: Vec::new(),
            contacts: Vec::new(),
        }
        .shape(limb(
            (0.0, 0.0),
            R_ELBOW,
            (0.0, FOREARM),
            R_WRIST,
            p.forearm(),
            far,
        ))
        .child(hand);
        Joint {
            name: format!("{}-shoulder", s.tag),
            offset,
            angle: shoulder,
            items: Vec::new(),
            contacts: Vec::new(),
        }
        .shape(limb(
            (0.0, 0.0),
            R_SHOULDER,
            (0.0, UPPER_ARM),
            R_ELBOW,
            p.upper_arm(),
            far,
        ))
        .child(forearm)
    };

    let leg = |s: Side, offset: (f64, f64), foot: usize| -> Joint {
        let far = side && !near(s);
        let sign = s.sign;
        let (hip, knee, ankle): (Turn, Turn, Turn) = if side {
            (
                Box::new(move |q: &Pose| -q[s.hip]),
                Box::new(move |q: &Pose| q[s.knee]),
                Box::new(move |q: &Pose| -q[s.ankle]),
            )
        } else {
            (
                Box::new(move |q: &Pose| sign * q[s.hip_side]),
                Box::new(|_: &Pose| 0.0),
                Box::new(|_: &Pose| 0.0),
            )
        };
        let mut shoe = Joint {
            name: format!("{}-ankle", s.tag),
            offset: (0.0, SHIN),
            angle: ankle,
            items: Vec::new(),
            contacts: Vec::new(),
        };
        if side {
            let d = format!(
                "M-15 -6C-22 -4 -23 {g} {h} {g}L{t} {g}C60 {g} 61 9 50 4C40 0 24 -4 12 -10C4 -14 -8 -10 -15 -6Z",
                g = fmt(ANKLE_HEIGHT),
                h = fmt(HEEL),
                t = fmt(TOE)
            );
            shoe = shoe.shape(filled(
                &d,
                [-23.0, -15.0, 61.0, 23.0],
                &p.shoes,
                far,
                [0.0, -14.0, 0.0, 24.0],
            ));
            // The flat sole's ends, then points along the rounded heel and toe, so a tilted
            // foot rests on whichever part of its outline is lowest.
            let g = ANKLE_HEIGHT;
            let mut points = vec![(HEEL, g), (TOE, g)];
            for k in 1..12 {
                let t = f64::from(k) / 12.0;
                points.push(cubic(
                    (-15.0, -6.0),
                    (-22.0, -4.0),
                    (-23.0, g),
                    (HEEL, g),
                    t,
                ));
                points.push(cubic((TOE, g), (60.0, g), (61.0, 9.0), (50.0, 4.0), t));
            }
            shoe.contacts = sole_points(foot, &points);
        } else {
            let out = -sign * 3.0;
            let d = format!(
                "M{} 0C{} -9 {} -9 {} 0L{} 15Q{} {g} {} {g}L{} {g}Q{} {g} {} 15Z",
                fmt(out - 15.0),
                fmt(out - 15.0),
                fmt(out + 15.0),
                fmt(out + 15.0),
                fmt(out + 17.0),
                fmt(out + 17.0),
                fmt(out + FRONT_SOLE_HALF),
                fmt(out - FRONT_SOLE_HALF),
                fmt(out - 17.0),
                fmt(out - 17.0),
                g = fmt(ANKLE_HEIGHT)
            );
            shoe = shoe.shape(filled(
                &d,
                [out - 18.0, -9.0, out + 18.0, 23.0],
                &p.shoes,
                false,
                [out - 17.0, 0.0, out + 17.0, 0.0],
            ));
            // The flat sole's ends, points along both rounded corners, and the corners' tops.
            let g = ANKLE_HEIGHT;
            let (a, b) = (out - FRONT_SOLE_HALF, out + FRONT_SOLE_HALF);
            let mut points = vec![(a, g), (b, g), (out - 17.0, 15.0), (out + 17.0, 15.0)];
            for k in 1..12 {
                let t = f64::from(k) / 12.0;
                points.push(quad((out + 17.0, 15.0), (out + 17.0, g), (b, g), t));
                points.push(quad((a, g), (out - 17.0, g), (out - 17.0, 15.0), t));
            }
            shoe.contacts = sole_points(foot, &points);
        }
        let shin = Joint {
            name: format!("{}-knee", s.tag),
            offset: (0.0, THIGH),
            angle: knee,
            items: Vec::new(),
            contacts: Vec::new(),
        }
        .shape(limb(
            (0.0, 0.0),
            R_KNEE,
            (0.0, SHIN),
            R_ANKLE,
            &p.pants,
            far,
        ))
        .child(shoe);
        Joint {
            name: format!("{}-hip", s.tag),
            offset,
            angle: hip,
            items: Vec::new(),
            contacts: Vec::new(),
        }
        .shape(limb((0.0, 0.0), R_HIP, (0.0, THIGH), R_KNEE, &p.pants, far))
        .child(shin)
    };

    let head = |offset: (f64, f64)| -> Joint {
        let angle: Turn = if side {
            Box::new(|q: &Pose| q[HEAD])
        } else {
            Box::new(|q: &Pose| -q[HEAD_SIDE])
        };
        let mut j = Joint {
            name: "neck".into(),
            offset,
            angle,
            items: Vec::new(),
            contacts: Vec::new(),
        };
        let eye = "#1c1b22";
        if side {
            j = j
                .shape(limb((0.0, 0.0), R_NECK, (2.0, -30.0), 12.0, &p.skin, false))
                .shape(filled(&ellipse_d(42.0, -53.0, 6.5, 7.5), ellipse_box(42.0, -53.0, 6.5, 7.5), &p.skin, false, [36.0, -60.0, 48.0, -46.0]))
                .shape(filled(&ellipse_d(6.0, -62.0, 36.0, 42.0), ellipse_box(6.0, -62.0, 36.0, 42.0), &p.skin, false, [-30.0, -62.0, 42.0, -62.0]))
                .shape(filled(&ellipse_d(-3.0, -58.0, 7.0, 10.0), ellipse_box(-3.0, -58.0, 7.0, 10.0), &p.skin, true, [-10.0, -58.0, 4.0, -58.0]))
                .shape(flat(&ellipse_d(27.0, -65.0, 3.4, 4.2), ellipse_box(27.0, -65.0, 3.4, 4.2), eye, 1.0))
                .shape(filled(
                    "M-31 -36C-46 -66 -32 -106 6 -105C34 -104 47 -88 43 -72C33 -82 17 -84 4 -78C-2 -68 -6 -58 -9 -46C-15 -40 -23 -34 -31 -36Z",
                    [-44.0, -106.0, 45.0, -33.0],
                    &p.hair,
                    false,
                    [-40.0, -70.0, 44.0, -70.0],
                ));
        } else {
            j = j
                .shape(limb((0.0, 0.0), R_NECK, (0.0, -30.0), 12.0, &p.skin, false))
                .shape(filled(&ellipse_d(-33.0, -58.0, 6.0, 10.0), ellipse_box(-33.0, -58.0, 6.0, 10.0), &p.skin, true, [-39.0, -58.0, -27.0, -58.0]))
                .shape(filled(&ellipse_d(33.0, -58.0, 6.0, 10.0), ellipse_box(33.0, -58.0, 6.0, 10.0), &p.skin, true, [27.0, -58.0, 39.0, -58.0]))
                .shape(filled(&ellipse_d(0.0, -62.0, 34.0, 42.0), ellipse_box(0.0, -62.0, 34.0, 42.0), &p.skin, false, [-34.0, -62.0, 34.0, -62.0]))
                .shape(flat(&ellipse_d(-12.5, -63.0, 3.6, 4.4), ellipse_box(-12.5, -63.0, 3.6, 4.4), eye, 1.0))
                .shape(flat(&ellipse_d(12.5, -63.0, 3.6, 4.4), ellipse_box(12.5, -63.0, 3.6, 4.4), eye, 1.0))
                .shape(flat("M-7 -40Q0 -36 7 -40Q0 -38 -7 -40Z", [-8.0, -41.0, 8.0, -36.0], "#000", 0.35))
                .shape(filled(
                    "M-37 -56C-41 -96 -18 -106 0 -106C18 -106 41 -96 37 -56C34 -70 27 -80 15 -83C5 -77 -12 -78 -23 -83C-31 -77 -35 -68 -37 -56Z",
                    [-42.0, -107.0, 42.0, -55.0],
                    &p.hair,
                    false,
                    [-38.0, -80.0, 38.0, -80.0],
                ));
        }
        j
    };

    let torso_angle = move |q: &Pose| if side { q[TORSO] } else { -q[TORSO_SIDE] };
    if side {
        let torso_shape = filled(
            "M-30 12C-34 -40 -38 -120 -30 -170Q-24 -196 2 -196Q26 -196 32 -170C42 -130 40 -104 34 -80C28 -50 28 -20 30 12Z",
            [-39.0, -197.0, 42.0, 13.0],
            &p.shirt,
            false,
            [-38.0, -90.0, 41.0, -90.0],
        );
        let pelvis_shape = filled(
            &ellipse_d(1.0, -8.0, 33.0, 34.0),
            ellipse_box(1.0, -8.0, 33.0, 34.0),
            &p.pants,
            false,
            [-32.0, -8.0, 34.0, -8.0],
        );
        Joint::new("pelvis", (0.0, -GROUND), |q: &Pose| q[BODY])
            .child(
                Joint::new("torso-far", (0.0, -WAIST), torso_angle)
                    .child(arm(LEFT, (0.0, -SHOULDER_RISE))),
            )
            .shape(pelvis_shape)
            .child(leg(LEFT, (0.0, 0.0), 1))
            .child(leg(RIGHT, (0.0, 0.0), 0))
            .child(
                Joint::new("torso", (0.0, -WAIST), torso_angle)
                    .shape(torso_shape)
                    .child(head((4.0, -NECK_RISE)))
                    .child(arm(RIGHT, (0.0, -SHOULDER_RISE))),
            )
    } else {
        let torso_shape = filled(
            "M-46 12C-46 -30 -50 -60 -50 -100C-50 -140 -54 -162 -62 -176Q-56 -194 -20 -194L20 -194Q56 -194 62 -176C54 -162 50 -140 50 -100C50 -60 46 -30 46 12Z",
            [-63.0, -195.0, 63.0, 13.0],
            &p.shirt,
            false,
            [-62.0, -90.0, 62.0, -90.0],
        );
        let pelvis_shape = filled(
            "M-46 -30C-54 -12 -50 22 -26 34L26 34C50 22 54 -12 46 -30C16 -35 -16 -35 -46 -30Z",
            [-54.0, -36.0, 54.0, 35.0],
            &p.pants,
            false,
            [-54.0, 0.0, 54.0, 0.0],
        );
        Joint::new("pelvis", (0.0, -GROUND), |_: &Pose| 0.0)
            .shape(pelvis_shape)
            .child(leg(RIGHT, (-HIP_HALF, 0.0), 0))
            .child(leg(LEFT, (HIP_HALF, 0.0), 1))
            .child(
                Joint::new("torso", (0.0, -WAIST), torso_angle)
                    .shape(torso_shape)
                    .child(head((0.0, -NECK_RISE)))
                    .child(arm(RIGHT, (-SHOULDER_HALF, -SHOULDER_RISE)))
                    .child(arm(LEFT, (SHOULDER_HALF, -SHOULDER_RISE))),
            )
    }
}

/// Contact points of the posed figure relative to the ground point (rest hips at `-GROUND`).
fn contacts(root: &Joint, pose: &Pose) -> Vec<Contact> {
    let mut points = Vec::new();
    root.contacts(pose, &mut points);
    points
        .into_iter()
        .map(|(foot, x, y)| Contact {
            foot,
            forward: x,
            down: y,
        })
        .collect()
}

/// A drawn piece of the figure: consecutive shapes of one joint, in paint order.
struct Segment<'a> {
    joint: usize,
    name: String,
    shapes: Vec<&'a Shape>,
}

/// The joints in preorder (with each one's parent) and the segments in paint order: a joint's
/// shapes paint before the joints listed after them, so children cover their parents at the shared
/// joint circle, and a side view's far limbs paint behind the body.
struct Flat<'a> {
    joints: Vec<&'a Joint>,
    parents: Vec<Option<usize>>,
    segments: Vec<Segment<'a>>,
}

impl<'a> Flat<'a> {
    fn new(root: &'a Joint) -> Self {
        let mut flat = Self {
            joints: Vec::new(),
            parents: Vec::new(),
            segments: Vec::new(),
        };
        flat.visit(root, None);
        flat
    }

    fn visit(&mut self, joint: &'a Joint, parent: Option<usize>) {
        let index = self.joints.len();
        self.joints.push(joint);
        self.parents.push(parent);
        let mut runs = 0;
        let mut open = false;
        for item in &joint.items {
            match item {
                Item::Shape(shape) => {
                    if !open {
                        let name = if runs == 0 {
                            joint.name.clone()
                        } else {
                            format!("{}-{runs}", joint.name)
                        };
                        runs += 1;
                        self.segments.push(Segment {
                            joint: index,
                            name,
                            shapes: Vec::new(),
                        });
                        open = true;
                    }
                    if let Some(segment) = self.segments.last_mut() {
                        segment.shapes.push(shape);
                    }
                }
                Item::Joint(child) => {
                    open = false;
                    self.visit(child, Some(index));
                }
            }
        }
    }

    /// Every joint's frame (x, y, clockwise degrees) relative to the ground point, facing right.
    fn frames(&self, pose: &Pose) -> Vec<(f64, f64, f64)> {
        let mut out: Vec<(f64, f64, f64)> = Vec::with_capacity(self.joints.len());
        for (i, joint) in self.joints.iter().enumerate() {
            let angle = (joint.angle)(pose);
            let frame = match self.parents[i] {
                None => {
                    // The root carries the body offsets (body-x is forward in a side view, and
                    // the facing mirror turns it toward the facing direction).
                    (
                        joint.offset.0 + pose[BODY_X],
                        joint.offset.1 + pose[BODY_Y],
                        angle,
                    )
                }
                Some(p) => {
                    let (px, py, pa) = out[p];
                    let (sin, cos) = pa.to_radians().sin_cos();
                    let (ox, oy) = joint.offset;
                    (
                        px + ox * cos - oy * sin,
                        py + ox * sin + oy * cos,
                        pa + angle,
                    )
                }
            };
            out.push(frame);
        }
        out
    }
}

/// Adds sample times between neighbors whose joints turn more than `MAX_TURN` degrees, so the
/// straight-line interpolation of each piece's position between keyframes stays on the arc its
/// joint travels (the error is under 0.1 px for a 300 px limb).
fn refine(times: &mut Vec<f64>, turn_at: &dyn Fn(f64) -> Vec<f64>) {
    const MAX_TURN: f64 = 2.0;
    let mut i = 0;
    while i + 1 < times.len() && times.len() < 6000 {
        let (t0, t1) = (times[i], times[i + 1]);
        let (a, b) = (turn_at(t0), turn_at(t1));
        let worst = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max);
        if worst > MAX_TURN && t1 - t0 > 0.002 {
            times.insert(i + 1, (t0 + t1) / 2.0);
        } else {
            i += 1;
        }
    }
}

/// Ends an element's open CSS rule with its transform: static when it never changes, otherwise an
/// animation through the samples that a straight line between kept samples cannot reproduce
/// within 0.12 units. Offsets carry six decimals of a percent; if two kept samples still share an
/// offset, the later one wins, so the true endpoint of a pass is never lost.
fn animate(
    css: &mut String,
    sel: &str,
    times: &[f64],
    values: &[String],
    series: &[Vec<f64>],
    period: Option<f64>,
    o: &Options,
) {
    let moving = values.iter().any(|v| *v != values[0]);
    let Some(period) = period.filter(|_| moving) else {
        let _ = writeln!(css, " transform: {}; }}", values[0]);
        return;
    };
    let iterations = if o.source.repeats() { "infinite" } else { "1" };
    let _ = writeln!(
        css,
        " animation: {sel} {}s linear {}s {iterations} both; }}",
        fmt(period),
        fmt(o.delay)
    );
    let mut stops: Vec<(String, &str)> = Vec::new();
    for i in keep(times, series, 0.12) {
        let at = percent(times[i] / period * 100.0);
        match stops.last_mut() {
            Some(last) if last.0 == at => last.1 = &values[i],
            _ => stops.push((at, &values[i])),
        }
    }
    let _ = write!(css, "  @keyframes {sel} {{");
    for (at, value) in stops {
        let _ = write!(css, " {at}% {{ transform: {value} }}");
    }
    let _ = writeln!(css, " }}");
}

/// A keyframe offset with six decimals and no trailing zeros.
fn percent(v: f64) -> String {
    let text = format!("{:.6}", v.clamp(0.0, 100.0));
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Renders the 2D rig.
pub fn render(values: &BTreeMap<String, String>) -> Result<String> {
    let view_of = |raw: &BTreeMap<String, String>| -> Result<View> {
        Ok(match raw["view"].as_str() {
            "side" => View::Side,
            "front" => View::Front,
            other => bail!("view must be side or front, got '{other}'"),
        })
    };
    let o = Options::parse(Flavor::Flat, values, view_of)?;
    let view = o.view;
    let side = view == View::Side;
    let facing = match o.text("facing") {
        "right" => 1.0,
        "left" => -1.0,
        other => bail!("facing must be right or left, got '{other}'"),
    };
    let f = if side { facing } else { 1.0 };
    let x = o.number("x", -100_000.0, 100_000.0)?;
    let y = o.number("y", -100_000.0, 100_000.0)?;
    let shadow = match o.text("shadow") {
        "on" | "true" => true,
        "off" | "false" => false,
        other => bail!("shadow must be on or off, got '{other}'"),
    };
    let id = o.id.clone();
    let k = o.scale();
    let root = skeleton(view, &o);
    let flat = Flat::new(&root);
    let fk = |pose: &Pose| contacts(&root, pose);

    let period = o.source.period();
    let gait = match o.travel {
        Travel::Auto => Some(stride(
            &o,
            period.unwrap_or(1.0),
            &fk,
            o.source.repeats(),
            &o.source.pass_times(48, 20.0),
        )?),
        Travel::Speed(_) => None,
    };
    let frames_at = |t: f64| flat.frames(&placed(&o, t, &fk, gait.as_ref()));
    let mut times = o.source.pass_times(48, 20.0);
    refine(&mut times, &|t| {
        frames_at(t).iter().map(|fr| fr.2).collect()
    });
    let frames: Vec<Vec<(f64, f64, f64)>> = times.iter().map(|t| frames_at(*t)).collect();

    let mut css = String::new();
    let mut body = String::new();
    let _ = writeln!(
        css,
        "  #{id} {{ position: absolute; left: {}px; top: {}px; width: 1px; height: 1px;{} }}",
        fmt(x),
        fmt(y),
        if (k - 1.0).abs() > 1e-9 {
            format!(" transform: scale({}); transform-origin: 0px 0px;", fmt(k))
        } else {
            String::new()
        }
    );

    // Travel along the facing direction, in rig units (the root scale turns them into px).
    let speed = match o.travel {
        Travel::Auto => gait.as_ref().map_or(0.0, |g: &Stride| g.speed),
        Travel::Speed(px) => px / k,
    };
    let travel_time = o.duration - o.delay;
    let mut travel = format!(
        "  #{id}-travel {{ position: absolute; left: 0px; top: 0px; width: 1px; height: 1px;"
    );
    if speed != 0.0 && travel_time > 0.0 {
        let _ = writeln!(
            css,
            "  @keyframes {id}-travel {{ from {{ transform: translateX(0px) }} to {{ transform: translateX({}px) }} }}",
            fmt(speed * travel_time * f)
        );
        let _ = write!(
            travel,
            " animation: {id}-travel {}s linear {}s 1 both;",
            fmt(travel_time),
            fmt(o.delay)
        );
    }
    travel.push_str(" }\n");
    css.push_str(&travel);

    let _ = writeln!(body, "<div id=\"{id}\">");
    let _ = writeln!(body, "  <div id=\"{id}-travel\">");
    if shadow {
        let half = if side { 82.0 } else { 92.0 };
        let _ = writeln!(
            css,
            "  #{id}-shadow {{ position: absolute; left: -{h}px; top: -12px; width: {w}px; height: 24px; background: radial-gradient(ellipse farthest-side, rgba(0,0,0,0.34), rgba(0,0,0,0));",
            h = fmt(half),
            w = fmt(half * 2.0)
        );
        // The shadow stays under the hips (a sequence's travel and a gait's surge move them).
        let values: Vec<String> = frames
            .iter()
            .map(|fs| format!("translateX({}px)", fmt(fs[0].0 * f)))
            .collect();
        let series: Vec<Vec<f64>> = frames.iter().map(|fs| vec![fs[0].0]).collect();
        animate(
            &mut css,
            &format!("{id}-shadow"),
            &times,
            &values,
            &series,
            period,
            &o,
        );
        let _ = writeln!(body, "    <div id=\"{id}-shadow\"></div>");
    }

    let mut gradient = 0usize;
    for segment in &flat.segments {
        let sel = format!("{id}-{}", segment.name);
        // A square centered on the joint, so the piece turns around its joint.
        let r = segment
            .shapes
            .iter()
            .flat_map(|s| {
                let [x0, y0, x1, y1] = s.bbox;
                [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
            })
            .map(|(px, py)| px.hypot(py))
            .fold(0.0, f64::max)
            .ceil()
            + 2.0;
        let mut content = String::new();
        for shape in &segment.shapes {
            gradient += 1;
            content.push_str(&shape.svg.replace("{G}", &format!("{id}-g{gradient}")));
        }
        if f < 0.0 {
            content = format!("<g transform=\"scale(-1 1)\">{content}</g>");
        }
        let _ = writeln!(
            body,
            "    <svg id=\"{sel}\" width=\"{d}\" height=\"{d}\" viewBox=\"{n} {n} {d} {d}\">{content}</svg>",
            n = fmt(-r),
            d = fmt(2.0 * r)
        );
        let transform = |fr: &(f64, f64, f64)| -> String {
            format!(
                "translate({}px, {}px) rotate({}deg)",
                fmt(fr.0 * f),
                fmt(fr.1),
                fmt(fr.2 * f)
            )
        };
        let values: Vec<String> = frames
            .iter()
            .map(|fs| transform(&fs[segment.joint]))
            .collect();
        let _ = write!(
            css,
            "  #{sel} {{ position: absolute; left: {n}px; top: {n}px; width: {d}px; height: {d}px;",
            n = fmt(-r),
            d = fmt(2.0 * r)
        );
        let series: Vec<Vec<f64>> = frames
            .iter()
            .map(|fs| {
                let fr = fs[segment.joint];
                vec![fr.0, fr.1, fr.2]
            })
            .collect();
        animate(&mut css, &sel, &times, &values, &series, period, &o);
    }
    let _ = writeln!(body, "  </div>");
    let _ = writeln!(body, "</div>");

    let mut out = Out::new();
    out.comment(&format!(
        "human-2d \"{id}\": {} view, {}. Regenerate with `aether-motion block human-2d` to change the pose; edit colors freely.",
        if side { "side" } else { "front" },
        o.describe_motion()
    ));
    out.push(&format!("<style>\n{css}</style>\n{body}"));
    Ok(out.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn colliding_offsets_keep_the_later_sample() {
        let o = Options::parse(Flavor::Flat, &BTreeMap::new(), |_| Ok(View::Side)).unwrap();
        let mut css = String::from("  #a {");
        let times = [0.0, 0.999_999_999, 1.0];
        let values: Vec<String> = ["rotate(0deg)", "rotate(90deg)", "rotate(0deg)"]
            .iter()
            .map(|v| v.to_string())
            .collect();
        let series = vec![vec![0.0], vec![90.0], vec![0.0]];
        animate(&mut css, "a", &times, &values, &series, Some(1.0), &o);
        assert!(css.contains("100% { transform: rotate(0deg) }"), "{css}");
        assert!(!css.contains("rotate(90deg)"));
    }

    #[test]
    fn tilted_front_soles_rest_on_their_lowest_drawn_point() {
        let mut v = BTreeMap::new();
        v.insert("motion".to_string(), "none".to_string());
        v.insert("r-hip-side".to_string(), "15".to_string());
        v.insert("l-hip-side".to_string(), "15".to_string());
        let o = Options::parse(Flavor::Flat, &v, |_| Ok(View::Front)).unwrap();
        let root = skeleton(View::Front, &o);
        let fk = |pose: &Pose| contacts(&root, pose);
        let pose = placed(&o, 0.0, &fk, None);
        // Densely sample the drawn corner curves of the left shoe under the placed pose: none of
        // them may dip below the ground by more than a hundredth of a unit.
        let mut lowest = f64::MIN;
        let lift = pose[BODY_Y];
        let out = 3.0;
        let g = ANKLE_HEIGHT;
        let mut pts = Vec::new();
        for i in 0..=100 {
            let t = i as f64 / 100.0;
            pts.push(quad(
                (out + 17.0, 15.0),
                (out + 17.0, g),
                (out + 11.0, g),
                t,
            ));
            pts.push(quad(
                (out - 11.0, g),
                (out - 17.0, g),
                (out - 17.0, 15.0),
                t,
            ));
        }
        // The ankle frame of the left foot: hip (30, 0) turned by -15, then the shin straight down.
        let a = (-15.0f64).to_radians();
        let (sin, cos) = a.sin_cos();
        for (x, y) in pts {
            let (lx, ly) = (x, y + THIGH + SHIN);
            let wy = -GROUND + lx * sin + ly * cos + lift;
            lowest = lowest.max(wy);
        }
        assert!(
            lowest < 0.01,
            "a drawn sole point is {lowest} units below the ground"
        );
    }

    #[test]
    fn hull_contains_both_circles() {
        let d = hull((0.0, 0.0), 20.0, (0.0, 100.0), 10.0);
        assert!(d.starts_with('M') && d.ends_with('Z'));
        // Tangent points sit outside the line of centers by r·cos and toward b by r·sin.
        let sin: f64 = 0.1;
        let x = 20.0 * (1.0 - sin * sin).sqrt();
        assert!(d.contains(&format!("M{} {}", fmt(-x), fmt(2.0))));
    }

    #[test]
    fn rest_feet_touch_the_ground() {
        for view in [View::Side, View::Front] {
            let o =
                Options::parse(Flavor::Flat, &values(&[("motion", "none")]), |_| Ok(view)).unwrap();
            let root = skeleton(view, &o);
            let pose = rest();
            let lowest = contacts(&root, &pose)
                .iter()
                .map(|c| c.down)
                .fold(f64::MIN, f64::max);
            assert!(lowest.abs() < 1e-9, "{view:?}: {lowest}");
        }
    }

    #[test]
    fn ground_lock_keeps_a_foot_planted_while_walking() {
        let o = Options::parse(Flavor::Flat, &values(&[("motion", "walk")]), |_| {
            Ok(View::Side)
        })
        .unwrap();
        let root = skeleton(View::Side, &o);
        let fk = |pose: &Pose| contacts(&root, pose);
        for i in 0..40 {
            let mut pose = placed(&o, i as f64 * 0.0275, &fk, None);
            let lift = pose[BODY_Y];
            pose[BODY_Y] = 0.0;
            let lowest = fk(&pose).iter().map(|c| c.down).fold(f64::MIN, f64::max);
            assert!((lowest + lift).abs() < 1e-9);
        }
        let gait = stride(&o, 1.1, &fk, true, &[]).unwrap();
        let speed = gait.speed;
        // The planted point stays still: hips(t) = speed * t + offset(t) cancels its motion.
        let world = |t: f64| -> f64 {
            let mut pose = placed(&o, t, &fk, Some(&gait));
            let hips = speed * t + pose[BODY_X];
            pose[BODY_X] = 0.0;
            // The right heel (point 0), flat on the ground through mid-stance.
            let heel = fk(&pose).into_iter().find(|c| c.foot == 0).unwrap();
            hips + heel.forward
        };
        // Mid-stance of the right foot (phase 0.15 to 0.35).
        let (a, b) = (world(0.17), world(0.38));
        assert!((a - b).abs() < 1.0, "planted foot slid {} units", a - b);
        assert!(speed > 80.0 && speed < 400.0, "walk speed {speed}");
    }

    #[test]
    fn renders_side_and_front_views() {
        for pairs in [
            vec![("motion", "walk"), ("travel", "auto")],
            vec![("view", "front"), ("motion", "wave"), ("facing", "left")],
            vec![("facing", "left"), ("motion", "jump"), ("size", "300")],
            vec![(
                "timeline",
                r#"[{"at":0},{"at":1,"r-shoulder":90,"easing":"smooth"}]"#,
            )],
            vec![("motion", "none"), ("r-elbow", "45")],
        ] {
            let html = render(&values(&pairs)).unwrap();
            assert!(html.contains("<div id=\"human\">"), "{pairs:?}");
            assert!(!html.contains("{G}"));
        }
        let still = render(&values(&[("motion", "none")])).unwrap();
        assert!(!still.contains("@keyframes"));
        assert!(render(&values(&[("view", "top")])).is_err());
    }
}
