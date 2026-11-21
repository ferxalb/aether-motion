//! Binds camera rigs to their subjects once a composition is parsed: measures the resting layout
//! with the renderer's own placement and bakes each followed subject's path (see
//! `crate::camera::tracking`).
//!
//! A subject's point, in camera px from the camera's center, is its resting border-box center
//! relative to the camera's, plus the translations (keyframes, CSS animation, motion paths) of the
//! subject and of every ancestor below the camera at that instant. Scale and rotation of those
//! elements are not followed. The result depends on nothing but the composition, so every
//! renderer, thread, and part of a render frames the subject identically.
//!
//! For the camera of a 2.5D space (`data-hf-space`), the subject's point is the stage position of
//! its center through the full 3D transforms of its chain (depth and tilts included), and the
//! framing divides the anchor offset by the depth magnification of the subject at each instant
//! (baked with the path), so the subject stays on its anchor while it or the camera moves in depth.
//! A subject inside a nested `data-hf-space`, or below an element that flattens planes with 3D
//! values on the subject's chain into a nested space, is projected twice; it is rejected with an
//! error that names that element.

use crate::camera::tracking::SubjectTrack;
use crate::camera::Gain;
use crate::model::Composition;
use crate::render::geometry::Rect;
use crate::render::Renderer;
use crate::space::math::Affine;
use crate::space::Projection;
use anyhow::{Context, Result};
use std::sync::Arc;

fn center(rect: Rect) -> [f64; 2] {
    [
        f64::from(rect.x + rect.w / 2.0),
        f64::from(rect.y + rect.h / 2.0),
    ]
}

/// Bakes the subject path of every camera rig that follows a subject and is not bound yet.
pub fn bind_camera_rigs(composition: &mut Composition) -> Result<()> {
    let pending: Vec<usize> = composition
        .elements
        .iter()
        .enumerate()
        .filter(|(_, e)| e.motion.rig.as_ref().is_some_and(|rig| !rig.is_bound()))
        .map(|(idx, _)| idx)
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    let mut bound = Vec::with_capacity(pending.len());
    {
        let snapshot: &Composition = composition;
        let mut renderer = Renderer::new(snapshot, 1.0)?;
        for &camera in &pending {
            let element = &snapshot.elements[camera];
            let rig = element.motion.rig.as_ref().expect("filtered on rigs");
            let subject = rig.subject().expect("unbound rigs have a subject");
            let camera_rect = renderer.rest_rect(camera)?;
            let rest = center(renderer.rest_rect(subject.element)?);
            let origin = center(camera_rect);
            let mut chain = vec![subject.element];
            while let Some(parent) = snapshot.elements[*chain.last().unwrap()].parent {
                if parent == camera {
                    break;
                }
                chain.push(parent);
            }
            let start = element.start;
            let length = element.end - element.start;
            if let Some(stage) = &element.depth.stage {
                let label = |idx: usize| {
                    snapshot.elements[idx]
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("<{}> with no id", snapshot.elements[idx].tag))
                };
                for (i, &ancestor) in chain.iter().enumerate().skip(1) {
                    let node = &snapshot.elements[ancestor];
                    let nested = node.depth.stage.is_some()
                        || (crate::render::space::flattens(node)
                            && chain[..i]
                                .iter()
                                .any(|&below| snapshot.elements[below].depth.plane.is_some()));
                    if nested {
                        anyhow::bail!(
                            "camera {}: data-hf-camera-subject {} is inside {}, a nested space of the camera's space; follow an element of the camera's own space",
                            element.id.as_deref().unwrap_or("no id"),
                            label(subject.element),
                            label(ancestor)
                        );
                    }
                }
                // Rest centers of the chain, stage first, relative to their parents'.
                let mut offsets = Vec::with_capacity(chain.len());
                let mut parent_center = origin;
                for &idx in chain.iter().rev() {
                    let c = center(renderer.rest_rect(idx)?);
                    offsets.push((idx, [c[0] - parent_center[0], c[1] - parent_center[1]]));
                    parent_center = c;
                }
                let world = |t: f64| {
                    let time = start + t;
                    let mut world = Affine::IDENTITY;
                    for &(idx, offset) in &offsets {
                        let node = &snapshot.elements[idx];
                        let mut pose = node.motion.pose(time - node.start);
                        if node.depth.stage.as_ref().is_some_and(|s| s.camera) {
                            let base = node.motion.base;
                            (pose.x, pose.y, pose.scale) = (base.x, base.y, base.scale);
                        }
                        let depth = node.depth.at(time - node.start);
                        world = world.then(&Affine::pose(
                            [
                                offset[0] + f64::from(pose.x),
                                offset[1] + f64::from(pose.y),
                                f64::from(depth[0]),
                            ],
                            f64::from(depth[1]),
                            f64::from(depth[2]),
                            f64::from(pose.rotate),
                            f64::from(pose.scale),
                        ));
                    }
                    world.t
                };
                let path = SubjectTrack::bake(&subject.settings, length, |t| {
                    let p = world(t);
                    [p[0], p[1]]
                })
                .with_context(|| format!("camera {}", element.id.as_deref().unwrap_or("no id")))?;
                let gain = match stage.projection {
                    Projection::Perspective { distance } => {
                        let f = f64::from(distance);
                        let stage = stage.clone();
                        Some(Gain::bake(240.0, length, move |t| {
                            let dolly = f64::from(stage.camera_at(t)[0]);
                            f / (f - dolly - world(t)[2]).max(f * crate::space::NEAR_SHARE)
                        }))
                    }
                    Projection::Orthographic => None,
                };
                let mut rig = (**rig).clone();
                rig.bind(
                    path,
                    [f64::from(camera_rect.w), f64::from(camera_rect.h)],
                    gain,
                );
                bound.push((camera, rig));
                continue;
            }
            let point = |t: f64| {
                let time = start + t;
                let mut p = [rest[0] - origin[0], rest[1] - origin[1]];
                for &idx in &chain {
                    let node = &snapshot.elements[idx];
                    let pose = node.motion.pose(time - node.start);
                    p[0] += f64::from(pose.x);
                    p[1] += f64::from(pose.y);
                }
                p
            };
            let path = SubjectTrack::bake(&subject.settings, length, point)
                .with_context(|| format!("camera {}", element.id.as_deref().unwrap_or("no id")))?;
            let mut rig = (**rig).clone();
            rig.bind(
                path,
                [f64::from(camera_rect.w), f64::from(camera_rect.h)],
                None,
            );
            bound.push((camera, rig));
        }
    }
    for (camera, rig) in bound {
        composition.elements[camera].motion.rig = Some(Arc::new(rig));
    }
    Ok(())
}
