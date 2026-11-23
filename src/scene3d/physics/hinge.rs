//! Validates revolute joint endpoints and connects their bodies in the pose bake.

use super::{vector, BodyKind, DEG_TO_RAD, PX_PER_METER};
use crate::scene3d::{NodeKind, Scene3d};
use anyhow::{anyhow, ensure, Result};
use rapier3d::prelude::{PhysicsWorld, RevoluteJointBuilder, RigidBodyHandle};
use std::collections::HashMap;

pub const MAX_HINGES: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct Hinge {
    pub bodies: [String; 2],
    pub anchors: [[f64; 3]; 2],
    pub axis: [f64; 3],
    pub limits: Option<[f64; 2]>,
}

pub(super) fn insert(
    scene: &Scene3d,
    world: &mut PhysicsWorld,
    handles: &[Option<RigidBodyHandle>],
) -> Result<()> {
    let ids: HashMap<_, _> = scene
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let NodeKind::Body(body) = node.kind else {
                return None;
            };
            Some((node.id.as_deref()?, (handles[index]?, body.kind)))
        })
        .collect();
    let mut count = 0;
    for node in &scene.nodes {
        let NodeKind::Hinge(hinge) = &node.kind else {
            continue;
        };
        count += 1;
        ensure!(
            count <= MAX_HINGES,
            "a viewport can have at most {MAX_HINGES} hinges"
        );
        let label = node.id.as_deref().unwrap_or("<hf-hinge>");
        let find = |id: &str| {
            ids.get(id).copied().ok_or_else(|| {
                anyhow!(
                    "hinge {label} references #{id}, which is not a rigid body in this viewport"
                )
            })
        };
        let (a, kind_a) = find(&hinge.bodies[0])?;
        let (b, kind_b) = find(&hinge.bodies[1])?;
        ensure!(a != b, "hinge {label} needs two different bodies");
        ensure!(
            kind_a == BodyKind::Dynamic || kind_b == BodyKind::Dynamic,
            "hinge {label} needs at least one dynamic body"
        );
        let mut joint = RevoluteJointBuilder::new(vector(hinge.axis, 1.0).normalize())
            .local_anchor1(vector(hinge.anchors[0], 1.0 / PX_PER_METER))
            .local_anchor2(vector(hinge.anchors[1], 1.0 / PX_PER_METER))
            .contacts_enabled(false);
        if let Some(limits) = hinge.limits {
            joint = joint.limits(limits.map(|degrees| (degrees * DEG_TO_RAD) as f32));
        }
        world.insert_impulse_joint(a, b, joint);
    }
    Ok(())
}
