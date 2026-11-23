//! Fixed-step rigid-body simulation, immutable pose tracks, and launch diagnostics.

use super::{NodeKind, Scene3d, Transform};
use crate::space::math::Affine;
use anyhow::{bail, ensure, Result};
use rapier3d::prelude::{ColliderBuilder, PhysicsWorld, Pose, RigidBodyBuilder, Rotation, Vector};

mod hinge;
pub use hinge::Hinge;

pub const TICK_RATE: u32 = 120;
pub const PX_PER_METER: f64 = 100.0;
pub const MAX_BODIES: usize = 128;
pub const MAX_DURATION: f64 = 60.0;
const MAX_POSE_BYTES: usize = 16 * 1024 * 1024;
const DEG_TO_RAD: f64 = std::f64::consts::PI / 180.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaunchVelocity {
    pub requested: [f64; 3],
    /// Solver velocity at the end of tick one, including gravity and any contacts.
    pub actual: [f64; 3],
    pub limit: f64,
    /// First solver step when the authored launch is subject to the speed cap.
    pub first_tick_affected: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch {
    pub linear: LaunchVelocity,
    pub angular: LaunchVelocity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collider {
    Box([f64; 3]),
    Sphere(f64),
    /// Y-axis cylinder, `[radius, height]`.
    Cylinder([f64; 2]),
    /// Y-axis cone with its base at the bottom, `[radius, height]`.
    Cone([f64; 2]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyKind {
    Fixed,
    Dynamic,
    Kinematic,
}

impl BodyKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Dynamic => "dynamic",
            Self::Kinematic => "kinematic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub collider: Collider,
    pub kind: BodyKind,
    pub friction: f64,
    pub restitution: f64,
    pub mass: f64,
    pub velocity: [f64; 3],
    pub angular_velocity: [f64; 3],
    pub ccd: bool,
    pub angular_damping: f64,
    pub linear_damping: f64,
}

/// Position in meters and quaternion in x, y, z, w order.
type Sample = [f32; 7];

#[derive(Clone, Debug, PartialEq)]
pub struct Bake {
    tracks: Vec<Option<Vec<Sample>>>,
    launches: Vec<(usize, Launch)>,
    ticks: usize,
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
        if bodies.is_empty()
            && !scene
                .nodes
                .iter()
                .any(|node| matches!(node.kind, NodeKind::Hinge(_)))
        {
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
        let moving_count = bodies
            .iter()
            .filter(|(_, _, body)| body.kind != BodyKind::Fixed)
            .count();
        let ticks = if moving_count == 0 {
            0
        } else {
            (duration * f64::from(TICK_RATE)).ceil() as usize
        };
        let pose_bytes = (bodies.len() + ticks * moving_count) * size_of::<Sample>();
        ensure!(
            pose_bytes <= MAX_POSE_BYTES,
            "rigid-body pose tracks need {pose_bytes} bytes, exceeding {MAX_POSE_BYTES}; shorten the clip or use fewer moving bodies"
        );

        let mut world = PhysicsWorld::new();
        world.gravity = Vector::new(0.0, 9.81, 0.0);
        world.integration_parameters.dt = 1.0 / TICK_RATE as f32;
        let max_linear = world.integration_parameters.max_linear_velocity();
        // Rapier 0.36.1 caps ordinary bodies at π/4 radians per full step.
        let max_angular = std::f32::consts::FRAC_PI_4 * world.integration_parameters.inv_dt();
        let mut tracks = vec![None; scene.nodes.len()];
        let mut launches = Vec::with_capacity(moving_count);
        let mut moving = Vec::with_capacity(moving_count);
        let mut handles = vec![None; scene.nodes.len()];
        for (index, node, body) in bodies {
            let start = authored_pose(&node.transform, 0.0);
            let builder = match body.kind {
                BodyKind::Dynamic => RigidBodyBuilder::dynamic()
                    .linvel(vector(body.velocity, 1.0 / PX_PER_METER))
                    .angvel(vector(body.angular_velocity, DEG_TO_RAD))
                    .ccd_enabled(body.ccd)
                    .angular_damping(body.angular_damping as f32)
                    .linear_damping(body.linear_damping as f32),
                BodyKind::Fixed => RigidBodyBuilder::fixed(),
                BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
            };
            let collider = match body.collider {
                Collider::Box([w, h, d]) => {
                    let half = |value: f64| (value / (2.0 * PX_PER_METER)) as f32;
                    ColliderBuilder::cuboid(half(w), half(h), half(d))
                }
                Collider::Sphere(radius) => ColliderBuilder::ball((radius / PX_PER_METER) as f32),
                Collider::Cylinder([radius, height]) => ColliderBuilder::cylinder(
                    (height / (2.0 * PX_PER_METER)) as f32,
                    (radius / PX_PER_METER) as f32,
                ),
                Collider::Cone([radius, height]) => ColliderBuilder::cone(
                    (height / (2.0 * PX_PER_METER)) as f32,
                    (radius / PX_PER_METER) as f32,
                ),
            };
            let (handle, _) = world.insert(
                builder.pose(start),
                collider
                    .friction(body.friction as f32)
                    .restitution(body.restitution as f32)
                    .mass(body.mass as f32),
            );
            handles[index] = Some(handle);
            let mut track = Vec::with_capacity(if body.kind == BodyKind::Fixed {
                1
            } else {
                ticks + 1
            });
            track.push(sample(start));
            tracks[index] = Some(track);
            if body.kind != BodyKind::Fixed {
                moving.push((index, handle, body));
            }
        }
        hinge::insert(scene, &mut world, &handles)?;
        for tick in 1..=ticks {
            for &(index, handle, body) in &moving {
                if body.kind == BodyKind::Kinematic {
                    let time = tick as f64 / f64::from(TICK_RATE);
                    let pose = authored_pose(&scene.nodes[index].transform, time);
                    world.bodies[handle].set_next_kinematic_position(pose);
                }
            }
            world.step();
            for &(index, handle, body) in &moving {
                let solved = &world.bodies[handle];
                let pose = sample(*solved.position());
                if !pose.iter().all(|value| value.is_finite()) {
                    let label = scene.nodes[index].id.as_deref().unwrap_or("<hf-body>");
                    bail!("rigid body {label} produced a non-finite pose at tick {tick}");
                }
                tracks[index]
                    .as_mut()
                    .expect("moving pose track")
                    .push(pose);
                if tick == 1 && body.kind == BodyKind::Dynamic {
                    launches.push((
                        index,
                        Launch {
                            linear: launch_velocity(
                                body.velocity,
                                solved.linvel(),
                                max_linear,
                                PX_PER_METER,
                            ),
                            angular: launch_velocity(
                                body.angular_velocity,
                                solved.angvel(),
                                max_angular,
                                1.0 / DEG_TO_RAD,
                            ),
                        },
                    ));
                }
            }
        }
        Ok(Some(Self {
            tracks,
            launches,
            ticks,
            pose_bytes,
        }))
    }

    pub fn ticks(&self) -> usize {
        self.ticks
    }

    pub fn pose_bytes(&self) -> usize {
        self.pose_bytes
    }

    pub fn dynamic_count(&self) -> usize {
        self.launches.len()
    }

    pub fn launch(&self, node: usize) -> Option<&Launch> {
        let index = self
            .launches
            .binary_search_by_key(&node, |(index, _)| *index)
            .ok()?;
        Some(&self.launches[index].1)
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

fn launch_velocity(
    requested: [f64; 3],
    actual: Vector,
    limit: f32,
    units_per_solver_unit: f64,
) -> LaunchVelocity {
    LaunchVelocity {
        requested,
        actual: actual
            .to_array()
            .map(|v| f64::from(v) * units_per_solver_unit),
        limit: f64::from(limit) * units_per_solver_unit,
        first_tick_affected: (vector(requested, 1.0 / units_per_solver_unit).length() > limit)
            .then_some(1),
    }
}

fn vector(values: [f64; 3], factor: f64) -> Vector {
    Vector::from_array(values.map(|v| (v * factor) as f32))
}

fn authored_pose(transform: &Transform, time: f64) -> Pose {
    let position = transform
        .position
        .each_ref()
        .map(|channel| channel.eval(time));
    let [rx, ry, rz] = transform
        .rotation
        .each_ref()
        .map(|channel| channel.eval(time));
    let axis_rotation = |degrees: f64, axis: usize| {
        let half = degrees * (std::f64::consts::PI / 360.0);
        let mut q = [0.0; 3];
        q[axis] = libm::sin(half) as f32;
        Rotation::from_xyzw(q[0], q[1], q[2], libm::cos(half) as f32)
    };
    let rotation = axis_rotation(ry, 1) * axis_rotation(rx, 0) * axis_rotation(rz, 2);
    Pose::from_parts(vector(position, 1.0 / PX_PER_METER), rotation)
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
