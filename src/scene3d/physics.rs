//! Fixed-step native rigid-body simulation and immutable pose tracks for 3D scene evaluation.

use super::{NodeKind, Scene3d};
use crate::space::math::Affine;
use anyhow::{bail, ensure, Result};
use rapier3d::prelude::{ColliderBuilder, PhysicsWorld, Pose, RigidBodyBuilder, Rotation, Vector};

pub const TICK_RATE: u32 = 120;
pub const PX_PER_METER: f64 = 100.0;
pub const MAX_BODIES: usize = 128;
pub const MAX_DURATION: f64 = 60.0;
const MAX_POSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collider {
    Box([f64; 3]),
    Sphere(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub collider: Collider,
    pub dynamic: bool,
    pub friction: f64,
    pub restitution: f64,
    pub mass: f64,
    pub velocity: [f64; 3],
    pub angular_velocity: [f64; 3],
    pub ccd: bool,
}

/// Position in meters and quaternion in x, y, z, w order.
type Sample = [f32; 7];

#[derive(Clone, Debug, PartialEq)]
pub struct Bake {
    tracks: Vec<Option<Vec<Sample>>>,
    ticks: usize,
    dynamic_count: usize,
    pose_bytes: usize,
}

impl Bake {
    /// Advances a compile-local solver in document order. Renderers only see the resulting poses.
    pub fn compile(scene: &Scene3d, duration: f64) -> Result<Option<Self>> {
        let bodies: Vec<_> = scene
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| match &node.kind {
                NodeKind::Body(body) => Some((index, node, body)),
                _ => None,
            })
            .collect();
        if bodies.is_empty() {
            return Ok(None);
        }
        ensure!(
            bodies.len() <= MAX_BODIES,
            "a viewport can have at most {MAX_BODIES} rigid bodies"
        );
        ensure!(
            duration.is_finite() && duration > 0.0 && duration <= MAX_DURATION,
            "a rigid-body viewport must last more than 0 and at most {MAX_DURATION} seconds"
        );
        let dynamic_count = bodies.iter().filter(|(_, _, body)| body.dynamic).count();
        let ticks = if dynamic_count == 0 {
            0
        } else {
            (duration * f64::from(TICK_RATE)).ceil() as usize
        };
        let pose_bytes = (bodies.len() + ticks * dynamic_count) * size_of::<Sample>();
        ensure!(
            pose_bytes <= MAX_POSE_BYTES,
            "rigid-body pose tracks need {pose_bytes} bytes, exceeding {MAX_POSE_BYTES}; shorten the clip or use fewer dynamic bodies"
        );

        let mut world = PhysicsWorld::new();
        world.gravity = Vector::new(0.0, 9.81, 0.0);
        world.integration_parameters.dt = 1.0 / TICK_RATE as f32;
        let mut tracks = vec![None; scene.nodes.len()];
        let mut handles = Vec::with_capacity(dynamic_count);
        for (index, node, body) in bodies {
            let position = node
                .transform
                .position
                .each_ref()
                .map(|channel| channel.eval(0.0));
            let [rx, ry, rz] = node
                .transform
                .rotation
                .each_ref()
                .map(|channel| channel.eval(0.0));
            let axis_rotation = |degrees: f64, axis: usize| {
                let half = degrees * (std::f64::consts::PI / 360.0);
                let mut q = [0.0; 3];
                q[axis] = libm::sin(half) as f32;
                Rotation::from_xyzw(q[0], q[1], q[2], libm::cos(half) as f32)
            };
            let rotation = axis_rotation(ry, 1) * axis_rotation(rx, 0) * axis_rotation(rz, 2);
            let start = Pose::from_parts(vector(position, 1.0 / PX_PER_METER), rotation);
            let builder = if body.dynamic {
                RigidBodyBuilder::dynamic()
                    .linvel(vector(body.velocity, 1.0 / PX_PER_METER))
                    .angvel(vector(body.angular_velocity, std::f64::consts::PI / 180.0))
                    .ccd_enabled(body.ccd)
            } else {
                RigidBodyBuilder::fixed()
            };
            let collider = match body.collider {
                Collider::Box([w, h, d]) => {
                    let half = |value: f64| (value / (2.0 * PX_PER_METER)) as f32;
                    ColliderBuilder::cuboid(half(w), half(h), half(d))
                }
                Collider::Sphere(radius) => ColliderBuilder::ball((radius / PX_PER_METER) as f32),
            };
            let (handle, _) = world.insert(
                builder.pose(start),
                collider
                    .friction(body.friction as f32)
                    .restitution(body.restitution as f32)
                    .mass(body.mass as f32),
            );
            let mut track = Vec::with_capacity(if body.dynamic { ticks + 1 } else { 1 });
            track.push(sample(start));
            tracks[index] = Some(track);
            if body.dynamic {
                handles.push((index, handle));
            }
        }
        for tick in 1..=ticks {
            world.step();
            for &(index, handle) in &handles {
                let pose = sample(*world.bodies[handle].position());
                if !pose.iter().all(|value| value.is_finite()) {
                    let label = scene.nodes[index].id.as_deref().unwrap_or("<hf-body>");
                    bail!("rigid body {label} produced a non-finite pose at tick {tick}");
                }
                tracks[index]
                    .as_mut()
                    .expect("dynamic pose track")
                    .push(pose);
            }
        }
        Ok(Some(Self {
            tracks,
            ticks,
            dynamic_count,
            pose_bytes,
        }))
    }

    pub fn dynamic_count(&self) -> usize {
        self.dynamic_count
    }

    pub fn ticks(&self) -> usize {
        self.ticks
    }

    pub fn pose_bytes(&self) -> usize {
        self.pose_bytes
    }

    pub(super) fn transform(&self, node: usize, t: f64) -> Option<Affine> {
        let track = self.tracks.get(node)?.as_deref()?;
        let [x, y, z, qx, qy, qz, w] = evaluate(track, t);
        Some(Affine {
            t: [x, y, z].map(|v| v * PX_PER_METER),
            m: [
                [
                    1.0 - 2.0 * (qy * qy + qz * qz),
                    2.0 * (qx * qy - w * qz),
                    2.0 * (qx * qz + w * qy),
                ],
                [
                    2.0 * (qx * qy + w * qz),
                    1.0 - 2.0 * (qx * qx + qz * qz),
                    2.0 * (qy * qz - w * qx),
                ],
                [
                    2.0 * (qx * qz - w * qy),
                    2.0 * (qy * qz + w * qx),
                    1.0 - 2.0 * (qx * qx + qy * qy),
                ],
            ],
        })
    }

    pub(super) fn hash_at(&self, t: f64, mut hash: u64) -> u64 {
        for track in self.tracks.iter().flatten().filter(|track| track.len() > 1) {
            for value in evaluate(track, t) {
                hash = (hash ^ value.to_bits()).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }
}

fn vector(values: [f64; 3], factor: f64) -> Vector {
    Vector::from_array(values.map(|v| (v * factor) as f32))
}

fn sample(pose: Pose) -> Sample {
    let [x, y, z] = pose.translation.to_array();
    let [qx, qy, qz, w] = pose.rotation.to_array();
    [x, y, z, qx, qy, qz, w]
}

fn evaluate(track: &[Sample], time: f64) -> [f64; 7] {
    let last = track.len() - 1;
    let tick = (time * f64::from(TICK_RATE)).clamp(0.0, last as f64);
    let index = tick as usize;
    let mut a = track[index].map(f64::from);
    if index < last && tick != index as f64 {
        let b = track[index + 1].map(f64::from);
        let alpha = tick - index as f64;
        for axis in 0..3 {
            a[axis] += (b[axis] - a[axis]) * alpha;
        }
        let dot: f64 = a[3..].iter().zip(&b[3..]).map(|(a, b)| a * b).sum();
        let sign = if dot < 0.0 { -1.0 } else { 1.0 };
        for axis in 3..7 {
            a[axis] += (sign * b[axis] - a[axis]) * alpha;
        }
    }
    let length = libm::sqrt(a[3..].iter().map(|v| v * v).sum::<f64>());
    for value in &mut a[3..] {
        *value /= length;
    }
    a
}
