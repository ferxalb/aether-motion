//! The 3D human rig (`human-3d`): a hierarchy of `<hf-node>` joints in a `data-hf-3d` viewport, with
//! rigid smooth-shaded meshes and `data-hf-keyframes` sampled from the motion.
//!
//! Each limb segment is a sphere at each joint joined by a tube on the spheres' common tangent
//! cone (a custom mesh), so the segments stay solid and round at every joint angle. Joints turn with
//! the renderer's CSS rotation order (`rotateY rotateX rotate`), and the same order drives the
//! kinematics that put the soles on the floor and match the travel speed to the stride. The block
//! is a full scene (viewport, lights, floor, following camera) or just the figure's node tree.

use super::motion::View;
use super::options::{Flavor, Options, Travel};
use super::place::{placed, stride, Contact, Stride};
use super::skeleton::*;
use super::{fmt, keep, Out};
use crate::blocks::template::escape_html;
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Description shown in the block catalog.
pub const DESCRIPTION: &str = "Controllable 3D human for a native data-hf-3d scene, built from smooth spheres joined by tangent tubes, so joints stay round at every angle. It takes the motions, sequence, timeline, and joint offsets of human-2d, keeps the soles on the floor, and adds a following camera that can orbit. scene=figure prints only the figure node for your own viewport.";

type Vec3 = [f64; 3];
type Mat3 = [[f64; 3]; 3];

fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

fn apply(m: &Mat3, v: Vec3) -> Vec3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// The rotation of a node's `rotation="rx ry rz"`: CSS `rotateY(ry) rotateX(rx) rotate(rz)`.
fn rotation(r: Vec3) -> Mat3 {
    let (sx, cx) = r[0].to_radians().sin_cos();
    let (sy, cy) = r[1].to_radians().sin_cos();
    let (sz, cz) = r[2].to_radians().sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mul(&ry, &mul(&rx, &rz))
}

/// A joint node.
struct Joint {
    name: String,
    position: Vec3,
    rotate: Box<dyn Fn(&Pose) -> Vec3>,
    meshes: Vec<String>,
    children: Vec<Joint>,
    /// Sole points in this node's frame.
    contacts: Vec<(usize, Vec3)>,
}

impl Joint {
    fn new(name: &str, position: Vec3, rotate: impl Fn(&Pose) -> Vec3 + 'static) -> Self {
        Self {
            name: name.to_string(),
            position,
            rotate: Box::new(rotate),
            meshes: Vec::new(),
            children: Vec::new(),
            contacts: Vec::new(),
        }
    }

    fn mesh(mut self, markup: String) -> Self {
        self.meshes.push(markup);
        self
    }

    fn child(mut self, joint: Joint) -> Self {
        self.children.push(joint);
        self
    }

    fn contacts(&self, pose: &Pose, out: &mut Vec<(usize, Vec3)>) {
        let m = rotation((self.rotate)(pose));
        let place = |v: Vec3| -> Vec3 {
            let r = apply(&m, v);
            [
                r[0] + self.position[0],
                r[1] + self.position[1],
                r[2] + self.position[2],
            ]
        };
        for (foot, v) in &self.contacts {
            out.push((*foot, place(*v)));
        }
        for child in &self.children {
            let mut inner = Vec::new();
            child.contacts(pose, &mut inner);
            out.extend(inner.into_iter().map(|(foot, v)| (foot, place(v))));
        }
    }
}

fn color_attr(color: &str) -> String {
    escape_html(color)
}

/// A smooth sphere (`scale` turns it into an ellipsoid).
fn sphere(center: Vec3, radii: Vec3, color: &str) -> String {
    let r = radii[0].max(radii[1]).max(radii[2]);
    let detail = if r >= 14.0 { 3 } else { 2 };
    let scale = if radii[0] == radii[1] && radii[1] == radii[2] {
        String::new()
    } else {
        format!(
            " scale=\"{} {} {}\"",
            fmt(radii[0] / r),
            fmt(radii[1] / r),
            fmt(radii[2] / r)
        )
    };
    format!(
        "<hf-mesh shape=\"sphere\" radius=\"{}\" detail=\"{detail}\" smooth position=\"{} {} {}\"{scale} color=\"{}\"></hf-mesh>",
        fmt(r),
        fmt(center[0]),
        fmt(center[1]),
        fmt(center[2]),
        color_attr(color)
    )
}

/// The open tube on the cone tangent to the spheres (`a`, `ra`) and (`b`, `rb`), outward facing.
fn tube(a: Vec3, ra: f64, b: Vec3, rb: f64, color: &str) -> String {
    const SEGMENTS: usize = 28;
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let u = [d[0] / len, d[1] / len, d[2] / len];
    // Any unit vector not parallel to u, then an orthonormal frame around u.
    let helper = if u[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let cross = |p: Vec3, q: Vec3| -> Vec3 {
        [
            p[1] * q[2] - p[2] * q[1],
            p[2] * q[0] - p[0] * q[2],
            p[0] * q[1] - p[1] * q[0],
        ]
    };
    let norm = |p: Vec3| -> Vec3 {
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        [p[0] / l, p[1] / l, p[2] / l]
    };
    let e1 = norm(cross(u, helper));
    let e2 = cross(u, e1);
    let sin = ((ra - rb) / len).clamp(-0.999, 0.999);
    let cos = (1.0 - sin * sin).sqrt();
    let mut vertices = Vec::with_capacity(SEGMENTS * 2);
    for (c, r) in [(a, ra), (b, rb)] {
        let center = [
            c[0] + r * sin * u[0],
            c[1] + r * sin * u[1],
            c[2] + r * sin * u[2],
        ];
        let radius = r * cos;
        for i in 0..SEGMENTS {
            let (s, co) = (std::f64::consts::TAU * i as f64 / SEGMENTS as f64).sin_cos();
            vertices.push([
                center[0] + radius * (co * e1[0] + s * e2[0]),
                center[1] + radius * (co * e1[1] + s * e2[1]),
                center[2] + radius * (co * e1[2] + s * e2[2]),
            ]);
        }
    }
    let mut indices = Vec::with_capacity(SEGMENTS * 2);
    for i in 0..SEGMENTS {
        let j = (i + 1) % SEGMENTS;
        for tri in [[i, j, SEGMENTS + i], [j, SEGMENTS + j, SEGMENTS + i]] {
            // Front faces follow (b - a) x (c - a); keep it pointing away from the axis.
            let (p0, p1, p2) = (vertices[tri[0]], vertices[tri[1]], vertices[tri[2]]);
            let n = cross(
                [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]],
                [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]],
            );
            let centroid = [
                (p0[0] + p1[0] + p2[0]) / 3.0 - a[0],
                (p0[1] + p1[1] + p2[1]) / 3.0 - a[1],
                (p0[2] + p1[2] + p2[2]) / 3.0 - a[2],
            ];
            let along = centroid[0] * u[0] + centroid[1] * u[1] + centroid[2] * u[2];
            let out = [
                centroid[0] - along * u[0],
                centroid[1] - along * u[1],
                centroid[2] - along * u[2],
            ];
            if n[0] * out[0] + n[1] * out[1] + n[2] * out[2] >= 0.0 {
                indices.push(tri);
            } else {
                indices.push([tri[0], tri[2], tri[1]]);
            }
        }
    }
    let vertex_text: Vec<String> = vertices
        .iter()
        .map(|v| format!("{} {} {}", fmt(v[0]), fmt(v[1]), fmt(v[2])))
        .collect();
    let index_text: Vec<String> = indices
        .iter()
        .map(|t| format!("{} {} {}", t[0], t[1], t[2]))
        .collect();
    format!(
        "<hf-mesh shape=\"custom\" smooth vertices=\"{}\" indices=\"{}\" color=\"{}\"></hf-mesh>",
        vertex_text.join(", "),
        index_text.join(", "),
        color_attr(color)
    )
}

/// A smooth closed body through elliptical rings `(y, rx, rz, z)` (top to bottom), capped with a
/// point at `top` above the first ring and `bottom` below the last one.
fn loft(rings: &[(f64, f64, f64, f64)], top: f64, bottom: f64, color: &str) -> String {
    const SEGMENTS: usize = 32;
    let mut vertices: Vec<Vec3> = Vec::new();
    for (y, rx, rz, z) in rings {
        for i in 0..SEGMENTS {
            let (s, c) = (std::f64::consts::TAU * i as f64 / SEGMENTS as f64).sin_cos();
            vertices.push([rx * c, *y, z + rz * s]);
        }
    }
    let first_z = rings[0].3;
    let last = rings[rings.len() - 1];
    let apex = vertices.len();
    vertices.push([0.0, top, first_z]);
    vertices.push([0.0, bottom, last.3]);
    let mut indices: Vec<[usize; 3]> = Vec::new();
    // Each triangle with a point inside the body next to it, to orient it outward.
    let mut add = |tri: [usize; 3], inside: Vec3, vertices: &[Vec3]| {
        let (p0, p1, p2) = (vertices[tri[0]], vertices[tri[1]], vertices[tri[2]]);
        let n = [
            (p1[1] - p0[1]) * (p2[2] - p0[2]) - (p1[2] - p0[2]) * (p2[1] - p0[1]),
            (p1[2] - p0[2]) * (p2[0] - p0[0]) - (p1[0] - p0[0]) * (p2[2] - p0[2]),
            (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p1[1] - p0[1]) * (p2[0] - p0[0]),
        ];
        let c = [
            (p0[0] + p1[0] + p2[0]) / 3.0 - inside[0],
            (p0[1] + p1[1] + p2[1]) / 3.0 - inside[1],
            (p0[2] + p1[2] + p2[2]) / 3.0 - inside[2],
        ];
        if n[0] * c[0] + n[1] * c[1] + n[2] * c[2] >= 0.0 {
            indices.push(tri);
        } else {
            indices.push([tri[0], tri[2], tri[1]]);
        }
    };
    for r in 0..rings.len() - 1 {
        let (a, b) = (r * SEGMENTS, (r + 1) * SEGMENTS);
        let inside = [
            0.0,
            (rings[r].0 + rings[r + 1].0) / 2.0,
            (rings[r].3 + rings[r + 1].3) / 2.0,
        ];
        for i in 0..SEGMENTS {
            let j = (i + 1) % SEGMENTS;
            add([a + i, a + j, b + i], inside, &vertices);
            add([a + j, b + j, b + i], inside, &vertices);
        }
    }
    let base = (rings.len() - 1) * SEGMENTS;
    let below_top = [
        0.0,
        (top + rings[0].0) / 2.0 + (rings[0].0 - top).abs(),
        first_z,
    ];
    let above_bottom = [
        0.0,
        (bottom + last.0) / 2.0 - (bottom - last.0).abs(),
        last.3,
    ];
    for i in 0..SEGMENTS {
        let j = (i + 1) % SEGMENTS;
        add([apex, i, j], below_top, &vertices);
        add([apex + 1, base + j, base + i], above_bottom, &vertices);
    }
    let vertex_text: Vec<String> = vertices
        .iter()
        .map(|v| format!("{} {} {}", fmt(v[0]), fmt(v[1]), fmt(v[2])))
        .collect();
    let index_text: Vec<String> = indices
        .iter()
        .map(|t| format!("{} {} {}", t[0], t[1], t[2]))
        .collect();
    format!(
        "<hf-mesh shape=\"custom\" smooth vertices=\"{}\" indices=\"{}\" color=\"{}\"></hf-mesh>",
        vertex_text.join(", "),
        index_text.join(", "),
        color_attr(color)
    )
}

/// A segment from this joint (radius `ra` at the origin) to `b` (radius `rb`): both spheres and
/// the tube between them.
fn segment(ra: f64, b: Vec3, rb: f64, color: &str) -> Vec<String> {
    vec![
        sphere([0.0; 3], [ra; 3], color),
        tube([0.0; 3], ra, b, rb, color),
        sphere(b, [rb; 3], color),
    ]
}

fn skeleton(o: &Options) -> Joint {
    let p = &o.palette;
    let arm = |s: Side| -> Joint {
        let sign = s.sign;
        let mut hand = Joint::new(
            &format!("{}-wrist", s.tag),
            [0.0, FOREARM, 0.0],
            move |q: &Pose| [q[s.wrist], 0.0, 0.0],
        );
        hand = hand
            .mesh(sphere([0.0; 3], [R_WRIST; 3], &p.skin))
            .mesh(sphere([0.0, 22.0, 2.0], [9.0, 24.0, 13.0], &p.skin))
            .mesh(sphere([sign * 4.0, 12.0, 11.0], [5.5, 10.0, 5.5], &p.skin));
        let mut forearm = Joint::new(
            &format!("{}-elbow", s.tag),
            [0.0, UPPER_ARM, 0.0],
            move |q: &Pose| [q[s.elbow], 0.0, 0.0],
        );
        for m in segment(R_ELBOW, [0.0, FOREARM, 0.0], R_WRIST, p.forearm()) {
            forearm = forearm.mesh(m);
        }
        let mut upper = Joint::new(
            &format!("{}-shoulder", s.tag),
            [-sign * SHOULDER_HALF, -SHOULDER_RISE, 0.0],
            move |q: &Pose| [q[s.shoulder], 0.0, sign * q[s.shoulder_side]],
        );
        for m in segment(R_SHOULDER, [0.0, UPPER_ARM, 0.0], R_ELBOW, p.upper_arm()) {
            upper = upper.mesh(m);
        }
        upper.child(forearm.child(hand))
    };
    let leg = |s: Side, foot: usize| -> Joint {
        let sign = s.sign;
        let mut shoe = Joint::new(
            &format!("{}-ankle", s.tag),
            [0.0, SHIN, 0.0],
            move |q: &Pose| [q[s.ankle], 0.0, 0.0],
        )
        .mesh(sphere([0.0, 8.0, 18.0], [15.0, 13.0, 34.0], &p.shoes))
        .mesh(format!(
            "<hf-mesh shape=\"box\" size=\"{} 6 {}\" position=\"0 {} {}\" color=\"{}\"></hf-mesh>",
            fmt(SOLE_HALF * 2.0),
            fmt(TOE - HEEL),
            fmt(ANKLE_HEIGHT - 3.0),
            fmt((TOE + HEEL) / 2.0),
            color_attr(&p.shoes)
        ));
        shoe.contacts = vec![
            (foot * 4, [-SOLE_HALF, ANKLE_HEIGHT, HEEL]),
            (foot * 4 + 1, [SOLE_HALF, ANKLE_HEIGHT, HEEL]),
            (foot * 4 + 2, [-SOLE_HALF, ANKLE_HEIGHT, TOE]),
            (foot * 4 + 3, [SOLE_HALF, ANKLE_HEIGHT, TOE]),
        ];
        let mut shin = Joint::new(
            &format!("{}-knee", s.tag),
            [0.0, THIGH, 0.0],
            move |q: &Pose| [-q[s.knee], 0.0, 0.0],
        );
        for m in segment(R_KNEE, [0.0, SHIN, 0.0], R_ANKLE, &p.pants) {
            shin = shin.mesh(m);
        }
        let mut thigh = Joint::new(
            &format!("{}-hip", s.tag),
            [-sign * HIP_HALF, 0.0, 0.0],
            move |q: &Pose| [q[s.hip], 0.0, sign * q[s.hip_side]],
        );
        for m in segment(R_HIP, [0.0, THIGH, 0.0], R_KNEE, &p.pants) {
            thigh = thigh.mesh(m);
        }
        thigh.child(shin.child(shoe))
    };
    let eye = "#1c1b22";
    let neck = Joint::new("neck", [0.0, -NECK_RISE, 0.0], |q: &Pose| {
        [-q[HEAD], -q[HEAD_TURN], -q[HEAD_SIDE]]
    })
    .mesh(sphere([0.0; 3], [R_NECK; 3], &p.skin))
    .mesh(tube([0.0; 3], R_NECK, [0.0, -32.0, 2.0], 12.0, &p.skin))
    .mesh(sphere([0.0, -62.0, 4.0], [34.0, 42.0, 37.0], &p.skin))
    .mesh(sphere([-33.0, -59.0, 2.0], [5.0, 9.5, 7.0], &p.skin))
    .mesh(sphere([33.0, -59.0, 2.0], [5.0, 9.5, 7.0], &p.skin))
    .mesh(sphere([0.0, -52.0, 40.0], [5.0, 8.0, 6.0], &p.skin))
    .mesh(sphere([-12.5, -63.0, 36.0], [4.2; 3], eye))
    .mesh(sphere([12.5, -63.0, 36.0], [4.2; 3], eye))
    .mesh(sphere([0.0, -74.0, -4.0], [37.0, 38.0, 39.0], &p.hair));
    let torso = Joint::new("torso", [0.0, -WAIST, 0.0], |q: &Pose| {
        [-q[TORSO], -q[TORSO_TWIST], -q[TORSO_SIDE]]
    })
    .mesh(loft(
        &[
            (-192.0, 30.0, 19.0, 0.0),
            (-184.0, 47.0, 25.0, 0.0),
            (-170.0, 55.0, 30.0, 1.0),
            (-140.0, 54.0, 33.0, 3.0),
            (-105.0, 49.0, 31.0, 2.0),
            (-65.0, 45.0, 29.0, 1.0),
            (-25.0, 46.0, 30.0, 0.0),
            (12.0, 47.0, 30.0, 0.0),
        ],
        -199.0,
        20.0,
        &p.shirt,
    ))
    .child(neck)
    .child(arm(RIGHT))
    .child(arm(LEFT));
    Joint::new("hips", [0.0, -GROUND, 0.0], |q: &Pose| [-q[BODY], 0.0, 0.0])
        .mesh(loft(
            &[
                (-38.0, 50.0, 32.5, 0.0),
                (-14.0, 53.0, 33.0, 0.0),
                (8.0, 52.0, 32.0, -1.0),
                (24.0, 40.0, 27.0, -1.0),
                (33.0, 22.0, 19.0, 0.0),
            ],
            -43.0,
            38.0,
            &p.pants,
        ))
        .child(leg(RIGHT, 0))
        .child(leg(LEFT, 1))
        .child(torso)
}

fn contacts(root: &Joint, pose: &Pose) -> Vec<Contact> {
    let mut points = Vec::new();
    root.contacts(pose, &mut points);
    points
        .into_iter()
        .map(|(foot, v)| Contact {
            foot,
            forward: v[2],
            down: v[1],
        })
        .collect()
}

/// Darkens a `#rrggbb` color toward black by `amount` (0..1).
fn darken(hex: &str, amount: f64) -> Option<String> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    let scale = |c: u8| ((c as f64) * (1.0 - amount)).round() as u8;
    Some(format!("#{:02x}{:02x}{:02x}", scale(r), scale(g), scale(b)))
}

/// Keyframe times (seconds from the clip start) and the matching motion times.
fn schedule(o: &Options) -> Result<Vec<(f64, f64)>> {
    const LIMIT: usize = 20_000;
    let pass = o.source.pass_times(32, 20.0);
    let Some(period) = o.source.period() else {
        return Ok(vec![(0.0, 0.0)]);
    };
    if period <= 0.0 || pass.len() < 2 {
        bail!("the motion's period is too short to animate; space keys at least 1 ms apart");
    }
    // Estimate before generating, so a long clip of short cycles fails fast.
    let laps = if o.source.repeats() {
        ((o.duration - o.delay).max(0.0) / period).ceil() + 1.0
    } else {
        1.0
    };
    let estimate = laps * (pass.len() - 1) as f64;
    if estimate > LIMIT as f64 {
        bail!(
            "this motion needs about {} keyframes over duration={} (limit {LIMIT}); shorten duration or lengthen cycle",
            estimate.round(),
            fmt(o.duration)
        );
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut lap = 0.0;
    'laps: loop {
        for t in &pass {
            if lap > 0.0 && *t == 0.0 {
                continue;
            }
            let motion = lap + t;
            let at = o.delay + motion;
            out.push((at, motion));
            if at >= o.duration {
                break 'laps;
            }
        }
        if !o.source.repeats() {
            break;
        }
        lap += period;
    }
    if out.len() > LIMIT {
        bail!(
            "this motion needs {} keyframes over duration={}; shorten duration or lengthen cycle",
            out.len(),
            fmt(o.duration)
        );
    }
    if o.delay > 0.0 {
        out.insert(0, (0.0, 0.0));
    }
    Ok(out)
}

/// `data-hf-keyframes` for the channels that change, or nothing.
fn keyframes(times: &[f64], channels: &[(&str, Vec<f64>)]) -> String {
    let moving: Vec<&(&str, Vec<f64>)> = channels
        .iter()
        .filter(|(_, v)| v.iter().any(|x| (x - v[0]).abs() > 1e-6))
        .collect();
    if moving.is_empty() || times.len() < 2 {
        return String::new();
    }
    let series: Vec<Vec<f64>> = (0..times.len())
        .map(|i| moving.iter().map(|(_, v)| v[i]).collect())
        .collect();
    let keys: Vec<String> = keep(times, &series, 0.05)
        .into_iter()
        .map(|i| {
            let t = &times[i];
            let mut key = format!("{{\"at\":{}", fmt(*t));
            for (name, values) in &moving {
                let _ = write!(key, ",\"{name}\":{}", fmt(values[i]));
            }
            key.push('}');
            key
        })
        .collect();
    format!(" data-hf-keyframes='[{}]'", keys.join(","))
}

fn emit(joint: &Joint, id: &str, times: &[f64], poses: &[Pose], out: &mut String, depth: usize) {
    let pad = "  ".repeat(depth);
    let rotations: Vec<Vec3> = poses.iter().map(|q| (joint.rotate)(q)).collect();
    let r0 = rotations[0];
    let channels = [
        (
            "rotateX",
            rotations.iter().map(|r| r[0]).collect::<Vec<_>>(),
        ),
        ("rotateY", rotations.iter().map(|r| r[1]).collect()),
        ("rotate", rotations.iter().map(|r| r[2]).collect()),
    ];
    let rotation = if r0.iter().all(|a| a.abs() < 1e-9) {
        String::new()
    } else {
        format!(" rotation=\"{} {} {}\"", fmt(r0[0]), fmt(r0[1]), fmt(r0[2]))
    };
    let mut position = joint.position;
    let mut extra = Vec::new();
    if joint.name == "hips" {
        // The hips carry the body offsets: forward is +z, and body-y lifts or drops them.
        let ys: Vec<f64> = poses.iter().map(|q| -GROUND + q[BODY_Y]).collect();
        let zs: Vec<f64> = poses.iter().map(|q| q[BODY_X]).collect();
        position[1] = ys[0];
        position[2] = zs[0];
        extra.push(("y", ys));
        extra.push(("z", zs));
    }
    let mut all: Vec<(&str, Vec<f64>)> = channels.to_vec();
    all.extend(extra);
    let _ = writeln!(
        out,
        "{pad}<hf-node id=\"{id}-{}\" position=\"{} {} {}\"{rotation}{}>",
        joint.name,
        fmt(position[0]),
        fmt(position[1]),
        fmt(position[2]),
        keyframes(times, &all)
    );
    for mesh in &joint.meshes {
        let _ = writeln!(out, "{pad}  {mesh}");
    }
    for child in &joint.children {
        emit(child, id, times, poses, out, depth + 1);
    }
    let _ = writeln!(out, "{pad}</hf-node>");
}

/// Renders the 3D rig.
pub fn render(values: &BTreeMap<String, String>) -> Result<String> {
    let o = Options::parse(Flavor::Solid, values, |_| Ok(View::Space))?;
    let full = match o.text("scene") {
        "full" => true,
        "figure" => false,
        other => bail!("scene must be full or figure, got '{other}'"),
    };
    let id = o.id.clone();
    let k = o.scale();
    let x = o.number("x", -1_000_000.0, 1_000_000.0)?;
    let z = o.number("z", -1_000_000.0, 1_000_000.0)?;
    let turn = o.number("turn", -3600.0, 3600.0)?;
    let root = skeleton(&o);
    let fk = |pose: &Pose| contacts(&root, pose);

    let plan = schedule(&o)?;
    let times: Vec<f64> = plan.iter().map(|(at, _)| *at).collect();
    let gait = match o.travel {
        Travel::Auto => Some(stride(
            &o,
            o.source.period().unwrap_or(1.0),
            &fk,
            o.source.repeats(),
        )),
        Travel::Speed(_) => None,
    };
    let mut poses: Vec<Pose> = plan
        .iter()
        .map(|(_, t)| placed(&o, *t, &fk, gait.as_ref()))
        .collect();
    // A motion played once carries its whole path in the stride; the track node takes it (in
    // scene px along the facing direction) so the camera and the floor shadow follow.
    let path: Option<Vec<f64>> = gait.as_ref().filter(|_| !o.source.repeats()).map(|g| {
        plan.iter()
            .zip(poses.iter_mut())
            .map(|((_, t), pose)| {
                let d = g.offset(*t);
                pose[BODY_X] -= d;
                d * k
            })
            .collect()
    });

    // Travel along the facing direction (turn), in scene px.
    let speed = match o.travel {
        Travel::Auto => gait.as_ref().map_or(0.0, |g: &Stride| g.speed) * k,
        Travel::Speed(v) => v,
    };
    let (dir_x, dir_z) = {
        let (s, c) = turn.to_radians().sin_cos();
        (s, c)
    };
    let travel_time = (o.duration - o.delay).max(0.0);
    let track_keys = if let Some(path) = &path {
        let series: Vec<Vec<f64>> = path.iter().map(|d| vec![*d]).collect();
        let keys: Vec<String> = keep(&times, &series, 0.05)
            .into_iter()
            .map(|i| {
                format!(
                    "{{\"at\":{},\"x\":{},\"z\":{}}}",
                    fmt(times[i]),
                    fmt(x + path[i] * dir_x),
                    fmt(z + path[i] * dir_z)
                )
            })
            .collect();
        if keys.len() > 1 {
            format!(" data-hf-keyframes='[{}]'", keys.join(","))
        } else {
            String::new()
        }
    } else if speed != 0.0 && travel_time > 0.0 {
        let d = speed * travel_time;
        format!(
            " data-hf-keyframes='[{{\"at\":{},\"x\":{},\"z\":{}}},{{\"at\":{},\"x\":{},\"z\":{}}}]'",
            fmt(o.delay),
            fmt(x),
            fmt(z),
            fmt(o.duration),
            fmt(x + d * dir_x),
            fmt(z + d * dir_z)
        )
    } else {
        String::new()
    };

    let mut figure = String::new();
    let depth = if full { 1 } else { 0 };
    let pad = "  ".repeat(depth);
    let _ = writeln!(
        figure,
        "{pad}<hf-node id=\"{id}-track\" position=\"{} 0 {}\"{track_keys}>",
        fmt(x),
        fmt(z)
    );
    if full {
        let floor = o.text("floor");
        let shades: Vec<String> = [0.16, 0.28, 0.4]
            .iter()
            .map(|a| darken(floor, *a).unwrap_or_else(|| "#151821".to_string()))
            .collect();
        for (i, (radius, shade)) in [(105.0, &shades[0]), (78.0, &shades[1]), (52.0, &shades[2])]
            .iter()
            .enumerate()
        {
            let _ = writeln!(
                figure,
                "{pad}  <hf-mesh shape=\"cylinder\" radius=\"{}\" height=\"0.5\" segments=\"48\" position=\"0 {} 0\" scale=\"1 1 0.75\" color=\"{}\"></hf-mesh>",
                fmt(radius * k),
                fmt(-1.6 - 0.5 * i as f64),
                color_attr(shade)
            );
        }
        let angle = o.number("camera-angle", -3600.0, 3600.0)?;
        let orbit = o.number("orbit", -3600.0, 3600.0)?;
        let height = o.number("camera-height", -100_000.0, 100_000.0)? * k;
        let distance = o.number("distance", 1.0, 1_000_000.0)? * k;
        let fov = o.number("fov", 1.0, 170.0)?;
        let orbit_keys = if orbit != 0.0 {
            format!(
                " data-hf-keyframes='[{{\"at\":0,\"rotateY\":{}}},{{\"at\":{},\"rotateY\":{},\"easing\":\"smooth\"}}]'",
                fmt(angle),
                fmt(o.duration),
                fmt(angle + orbit)
            )
        } else {
            String::new()
        };
        let _ = writeln!(
            figure,
            "{pad}  <hf-node id=\"{id}-aim\" position=\"0 {} 0\"></hf-node>",
            fmt(-0.5 * o.size)
        );
        let _ = writeln!(
            figure,
            "{pad}  <hf-node id=\"{id}-orbit\" rotation=\"0 {} 0\"{orbit_keys}>",
            fmt(angle)
        );
        let _ = writeln!(
            figure,
            "{pad}    <hf-camera position=\"0 {} {}\" fov=\"{}\" target=\"#{id}-aim\" far=\"{}\"></hf-camera>",
            fmt(-height),
            fmt(distance),
            fmt(fov),
            fmt((distance * 8.0).max(50_000.0))
        );
        let _ = writeln!(figure, "{pad}  </hf-node>");
    }
    let _ = writeln!(
        figure,
        "{pad}  <hf-node id=\"{id}-figure\"{}{}>",
        if turn != 0.0 {
            format!(" rotation=\"0 {} 0\"", fmt(turn))
        } else {
            String::new()
        },
        if (k - 1.0).abs() > 1e-9 {
            format!(" scale=\"{}\"", fmt(k))
        } else {
            String::new()
        }
    );
    emit(&root, &id, &times, &poses, &mut figure, depth + 2);
    let _ = writeln!(figure, "{pad}  </hf-node>");
    let _ = writeln!(figure, "{pad}</hf-node>");

    let mut out = Out::new();
    out.comment(&format!(
        "human-3d \"{id}\": {}. Regenerate with `aether-motion block human-3d` to change the pose; edit colors freely.",
        o.describe_motion()
    ));
    if !full {
        out.push(&figure);
        return Ok(out.finish());
    }
    let start = o.number("start", 0.0, 3600.0)?;
    let track = o.number("track", 0.0, 1000.0)?;
    if track.fract() != 0.0 {
        bail!("track must be a whole number (it becomes data-track-index)");
    }
    let left = o.number("left", -100_000.0, 100_000.0)?;
    let top = o.number("top", -100_000.0, 100_000.0)?;
    let width = o.number("width", 1.0, 100_000.0)?;
    let height = o.number("height", 1.0, 100_000.0)?;
    let antialias = o.number("antialias", 1.0, 4.0)?;
    if antialias.fract() != 0.0 {
        bail!("antialias must be a whole number from 1 to 4");
    }
    let floor = o.text("floor");
    let grid = o.text("grid");
    for (name, color) in [("floor", floor), ("grid", grid)] {
        if color.contains(['<', '>', '"', '\'']) || color.is_empty() {
            bail!("{name} must be a CSS color");
        }
    }
    let mut html = String::new();
    let _ = writeln!(
        html,
        "<div id=\"{id}\" class=\"clip\" data-start=\"{}\" data-duration=\"{}\" data-track-index=\"{}\" data-hf-3d data-hf-3d-antialias=\"{}\" style=\"position: absolute; left: {}px; top: {}px; width: {}px; height: {}px\">",
        fmt(start),
        fmt(o.duration),
        fmt(track),
        fmt(antialias),
        fmt(left),
        fmt(top),
        fmt(width),
        fmt(height)
    );
    html.push_str(
        "  <hf-light type=\"ambient\" color=\"#aab6d8\" intensity=\"0.55\"></hf-light>\n",
    );
    html.push_str("  <hf-light type=\"directional\" color=\"#fff1df\" intensity=\"1.1\" direction=\"-0.45 0.8 -0.55\"></hf-light>\n");
    html.push_str("  <hf-light type=\"directional\" color=\"#9cb6ff\" intensity=\"0.45\" direction=\"0.6 0.25 0.75\"></hf-light>\n");
    let _ = writeln!(
        html,
        "  <hf-mesh shape=\"plane\" size=\"12000 12000\" color=\"{}\"></hf-mesh>",
        color_attr(floor)
    );
    // A world-fixed grid, so travel reads against the floor.
    let step = 200.0;
    let lines = 30;
    let span = step * (lines * 2) as f64;
    for i in -lines..=lines {
        let at = step * i as f64;
        let _ = writeln!(
            html,
            "  <hf-mesh shape=\"box\" size=\"{} 1 3\" position=\"{} -0.6 {}\" color=\"{}\"></hf-mesh>",
            fmt(span),
            fmt(x),
            fmt(z + at),
            color_attr(grid)
        );
        let _ = writeln!(
            html,
            "  <hf-mesh shape=\"box\" size=\"3 1 {}\" position=\"{} -0.6 {}\" color=\"{}\"></hf-mesh>",
            fmt(span),
            fmt(x + at),
            fmt(z),
            color_attr(grid)
        );
    }
    html.push_str(&figure);
    html.push_str("</div>\n");
    out.push(&html);
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
    fn rotation_matches_css_senses() {
        // rotateX turns +y (down) toward +z (the viewer).
        let v = apply(&rotation([90.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);
        assert!((v[2] - 1.0).abs() < 1e-9);
        // rotate turns +y toward -x (clockwise on screen).
        let v = apply(&rotation([0.0, 0.0, 90.0]), [0.0, 1.0, 0.0]);
        assert!((v[0] + 1.0).abs() < 1e-9);
        // rotateY turns +z toward +x.
        let v = apply(&rotation([0.0, 90.0, 0.0]), [0.0, 0.0, 1.0]);
        assert!((v[0] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn tube_faces_outward() {
        let mesh = tube([0.0; 3], 20.0, [0.0, 100.0, 0.0], 10.0, "#fff");
        assert!(mesh.contains("shape=\"custom\""));
        assert_eq!(mesh.matches(", ").count() + 2, 28 * 2 + 28 * 2);
    }

    #[test]
    fn rest_soles_touch_the_floor_and_walk_stays_planted() {
        let o = Options::parse(Flavor::Solid, &values(&[("motion", "walk")]), |_| {
            Ok(View::Space)
        })
        .unwrap();
        let root = skeleton(&o);
        let fk = |pose: &Pose| contacts(&root, pose);
        let lowest = fk(&rest()).iter().map(|c| c.down).fold(f64::MIN, f64::max);
        assert!(lowest.abs() < 1e-9, "{lowest}");
        let speed = stride(&o, 1.1, &fk, true).speed;
        assert!(speed > 80.0 && speed < 400.0, "walk speed {speed}");
    }

    #[test]
    fn renders_full_scenes_and_figures() {
        let html = render(&values(&[
            ("motion", "walk"),
            ("travel", "auto"),
            ("orbit", "40"),
        ]))
        .unwrap();
        assert!(html.contains("data-hf-3d"));
        assert!(html.contains("<hf-camera"));
        let figure = render(&values(&[("scene", "figure"), ("motion", "none")])).unwrap();
        assert!(!figure.contains("data-hf-3d"));
        assert!(!figure.contains("data-hf-keyframes"));
        assert!(render(&values(&[("scene", "half")])).is_err());
        assert_eq!(darken("#808080", 0.5).as_deref(), Some("#404040"));
        assert_eq!(darken("#aé000", 0.5), None);
        assert!(render(&values(&[("track", "1.5")])).is_err());
        assert!(render(&values(&[
            ("motion", "run"),
            ("cycle", "0.1"),
            ("duration", "3600")
        ]))
        .is_err());
    }
}
