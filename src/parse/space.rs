//! 2.5D attributes: `data-hf-space` stages (`data-hf-perspective`), the static 3D values of their
//! planes (`data-hf-z`, `data-hf-rotate-x`, `data-hf-rotate-y`, `data-hf-backface`), the keyframed
//! `z`/`rotateX`/`rotateY` channels, the stage camera's dolly and orientation (from
//! `data-hf-camera` keyframes or `data-hf-shots`), and the checks that every plane has a space and
//! that no blend mode sits on a plane that is always projected.
//! Produces [`crate::space::Depth`].

use crate::camera::curve::{Curve, Interp};
use crate::model::{Easing, Element, Keyframe, Track, TrackPoint};
use crate::parse::animation::shots::ShotSpec;
use crate::parse::html::{attr, number, signed_number};
use crate::parse::tree::CameraDecl;
use crate::space::{Depth, PlaneMotion, Projection, Stage, DEFAULT_PERSPECTIVE};
use anyhow::{bail, Result};
use kuchiki::NodeRef;
use std::sync::Arc;

/// Largest depth (px) or tilt (degrees) a plane accepts, statically or in a keyframe.
pub(in crate::parse) const MAX_DEPTH_VALUE: f64 = 100_000.0;

const PLANE_ATTRIBUTES: [&str; 4] = [
    "data-hf-z",
    "data-hf-rotate-x",
    "data-hf-rotate-y",
    "data-hf-backface",
];

/// Reads the static 2.5D attributes of one element.
pub(in crate::parse) fn parse_depth(node: &NodeRef) -> Result<Depth> {
    let stage = match attr(node, "data-hf-space") {
        None => {
            if attr(node, "data-hf-perspective").is_some() {
                bail!("data-hf-perspective needs data-hf-space on the same element");
            }
            None
        }
        Some(kind) => {
            let projection = match kind.trim() {
                "" | "perspective" => {
                    let distance = attr(node, "data-hf-perspective")
                        .map(|raw| number(&raw, "data-hf-perspective"))
                        .transpose()?
                        .unwrap_or(f64::from(DEFAULT_PERSPECTIVE));
                    if !(50.0..=MAX_DEPTH_VALUE).contains(&distance) {
                        bail!("data-hf-perspective must be between 50 and 100000 px");
                    }
                    Projection::Perspective {
                        distance: distance as f32,
                    }
                }
                "orthographic" => {
                    if attr(node, "data-hf-perspective").is_some() {
                        bail!("data-hf-perspective only applies to data-hf-space=\"perspective\"");
                    }
                    Projection::Orthographic
                }
                other => {
                    bail!("data-hf-space must be perspective or orthographic: {other}")
                }
            };
            Some(Arc::new(Stage {
                projection,
                camera: false,
                dolly: None,
                orientation: None,
            }))
        }
    };
    let value = |name: &str| -> Result<f32> {
        let Some(raw) = attr(node, name) else {
            return Ok(0.0);
        };
        let n = signed_number(raw.trim(), name)?;
        if !n.is_finite() || n.abs() > MAX_DEPTH_VALUE {
            bail!("{name} must be a number of at most 100000");
        }
        Ok(n as f32)
    };
    let backface_hidden = match attr(node, "data-hf-backface").as_deref() {
        None | Some("visible") => false,
        Some("hidden") => true,
        Some(other) => bail!("data-hf-backface must be visible or hidden: {other}"),
    };
    let has_plane = PLANE_ATTRIBUTES
        .iter()
        .any(|name| attr(node, name).is_some());
    let plane = has_plane.then(|| {
        Ok::<_, anyhow::Error>(Arc::new(PlaneMotion {
            base: [
                value("data-hf-z")?,
                value("data-hf-rotate-x")?,
                value("data-hf-rotate-y")?,
            ],
            tracks: Default::default(),
            backface_hidden,
        }))
    });
    Ok(Depth {
        stage,
        plane: plane.transpose()?,
    })
}

/// Adds keyframed plane channels and the stage camera to an element whose motion is compiled.
/// Adds keyframed plane channels and the stage camera to an element whose motion is compiled.
pub(in crate::parse) fn finish_depth(
    element: &mut Element,
    keyframes: &[Keyframe],
    camera: &CameraDecl,
    root: bool,
    clip_len: f64,
) -> Result<()> {
    let keyed = keyframes
        .iter()
        .any(|k| k.depth.iter().any(Option::is_some));
    let camera_depth = camera
        .frames
        .iter()
        .any(|f| f.given_depth.iter().any(|&g| g))
        || ShotSpec::has_depth(&camera.shots);
    if element.depth.stage.is_none() && element.depth.plane.is_none() && !keyed && !camera_depth {
        return Ok(());
    }
    if root {
        bail!("data-hf-space and 3D plane values cannot be on the root; add a container inside it");
    }
    if let Some(stage) = &element.depth.stage {
        let mut stage = (**stage).clone();
        stage.camera = !camera.is_empty();
        let channels = camera_channels(camera, clip_len);
        stage.dolly = channels[0].clone();
        if channels[1..].iter().any(Option::is_some) {
            let [x, y, roll] =
                [1, 2, 3].map(|i| channels[i].clone().unwrap_or_else(|| Curve::constant(0.0)));
            stage.orientation = Some([x, y, roll]);
        }
        if stage.orientation.is_some()
            && camera
                .options
                .as_ref()
                .is_some_and(|options| options.subject.is_some())
        {
            bail!("a data-hf-camera-subject cannot be framed by a camera that also turns (rotateX, rotateY, rotate); follow without turning, or turn without a subject");
        }
        element.depth.stage = Some(Arc::new(stage));
    } else if camera_depth {
        bail!("camera z, rotateX, rotateY, and rotate need data-hf-space on the camera");
    }
    if keyed {
        let mut plane = element.depth.plane.as_deref().cloned().unwrap_or_default();
        for (i, track) in plane.tracks.iter_mut().enumerate() {
            if keyframes.iter().all(|k| k.depth[i].is_none()) {
                continue;
            }
            let mut value = plane.base[i];
            *track = Track(
                keyframes
                    .iter()
                    .map(|k| {
                        value = k.depth[i].unwrap_or(value);
                        TrackPoint {
                            at: k.at,
                            value,
                            easing: k.easing,
                        }
                    })
                    .collect(),
            );
        }
        element.depth.plane = Some(Arc::new(plane));
    }
    Ok(())
}

/// Curves of the 3D camera channels (`z`, `rotateX`, `rotateY`, `rotate`) of a space camera, from
/// its `data-hf-camera` keyframes (classic or rig easings, `smooth` included) or its shots; `None`
/// for a channel that is never set.
fn camera_channels(camera: &CameraDecl, clip_len: f64) -> [Option<Curve>; 4] {
    let options_easing = camera.options.as_ref().and_then(|options| options.easing);
    if !camera.shots.is_empty() {
        let default = options_easing.unwrap_or(Interp::Smooth);
        return ShotSpec::depth_keys(&camera.shots, default, clip_len)
            .map(|keys| keys.map(|keys| Curve::new(&keys)));
    }
    let default = options_easing.unwrap_or(Interp::Eased(Easing::Linear));
    std::array::from_fn(|i| {
        if camera.frames.iter().all(|f| !f.given_depth[i]) {
            return None;
        }
        let keys: Vec<(f64, f64, Interp)> = camera
            .frames
            .iter()
            .map(|f| {
                (
                    f.at,
                    f64::from(f.depth[i]),
                    f.interp.unwrap_or(if camera.is_rig() {
                        default
                    } else {
                        Interp::Eased(f.easing)
                    }),
                )
            })
            .collect();
        Some(Curve::new(&keys))
    })
}

/// Checks the tree once every element (imported scenes included) is in place: every plane with
/// 3D values of its own, and every nested space, sits inside a space.
pub(in crate::parse) fn check_spaces(elements: &[Element]) -> Result<()> {
    if !elements
        .iter()
        .any(|e| e.depth.stage.is_some() || e.depth.plane.is_some())
    {
        return Ok(());
    }
    let label = |idx: usize| {
        elements[idx]
            .id
            .clone()
            .unwrap_or_else(|| format!("<{}> with no id", elements[idx].tag))
    };
    for (idx, element) in elements.iter().enumerate() {
        if element.style.blend != crate::model::BlendMode::Normal {
            // A blended element on a plane that tilts, or under a camera that turns, is drawn on a
            // projected layer, where blending against the frame is not possible.
            let projected = std::iter::successors(Some(idx), |&p| elements[p].parent).find(|&a| {
                elements[a]
                    .depth
                    .plane
                    .as_ref()
                    .is_some_and(|plane| plane.tilts())
                    || elements[a]
                        .depth
                        .stage
                        .as_ref()
                        .is_some_and(|stage| stage.orientation.is_some())
            });
            if let Some(a) = projected {
                bail!(
                    "mix-blend-mode on {} cannot be drawn on a projected plane of a 2.5D space: {} tilts or turns its camera; keep blended elements on planes that face an unturned camera",
                    label(idx),
                    label(a)
                );
            }
        }
        if element.depth.plane.is_none() {
            continue;
        }
        let inside = std::iter::successors(element.parent, |&p| elements[p].parent)
            .any(|p| elements[p].depth.stage.is_some());
        if !inside {
            bail!(
                "{} has 3D plane values (data-hf-z, data-hf-rotate-x/y, data-hf-backface, or z/rotateX/rotateY keyframes) but no data-hf-space ancestor",
                label(idx)
            );
        }
    }
    Ok(())
}
