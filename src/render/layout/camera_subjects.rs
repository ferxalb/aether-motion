//! Binds camera rigs to their subjects once a composition is parsed: measures the resting layout
//! with the renderer's own placement and bakes each followed subject's path (see
//! `crate::camera::tracking`).
//!
//! A subject's point, in camera px from the camera's center, is its resting border-box center
//! relative to the camera's, plus the translations (keyframes, CSS animation, motion paths) of the
//! subject and of every ancestor below the camera at that instant. Scale and rotation of those
//! elements are not followed. The result depends on nothing but the composition, so every
//! renderer, thread, and part of a render frames the subject identically.

use crate::camera::tracking::SubjectTrack;
use crate::model::Composition;
use crate::render::geometry::Rect;
use crate::render::Renderer;
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
            rig.bind(path, [f64::from(camera_rect.w), f64::from(camera_rect.h)]);
            bound.push((camera, rig));
        }
    }
    for (camera, rig) in bound {
        composition.elements[camera].motion.rig = Some(Arc::new(rig));
    }
    Ok(())
}
