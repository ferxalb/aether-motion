//! `data-hf-camera`: zoom and pan keyframes compiled into scale and translate motion tracks.
//!
//! The camera is an ordinary (non-root) container. Its whole subtree scales around the container's
//! center, so a pan is a translation chosen to keep a focus point under that center:
//! `translate = -zoom * focus_offset`. Place the scene inside a transparent camera wrapper and give
//! the background to the root.
//!
//! Keyframes that use rig features (`"easing": "smooth"`, `anchorX`, `anchorY`, or `follow`) are
//! compiled by `rig.rs` into a [`crate::camera::CameraRig`] instead; classic keyframes keep the
//! exact tracks built here.
use crate::camera::curve::Interp;
use crate::model::{Easing, Motion, Pose, Track, TrackPoint};
use crate::parse::animation::timing::parse_keyframe_easing;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;

const MIN_ZOOM: f64 = 0.05;
const MAX_ZOOM: f64 = 50.0;
const MAX_OFFSET: f64 = 1_000_000.0;

/// One camera pose: `zoom` (1 = neutral) centered on the point `(fx, fy)` px from the container center.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::parse) struct CameraFrame {
    pub(in crate::parse) at: f64,
    pub(in crate::parse) zoom: f32,
    pub(in crate::parse) fx: f32,
    pub(in crate::parse) fy: f32,
    pub(in crate::parse) easing: Easing,
    /// Rig fields: values given on this keyframe (`None` inherits).
    pub(in crate::parse) given: [Option<f64>; 2],
    pub(in crate::parse) anchor: [Option<f64>; 2],
    pub(in crate::parse) follow: Option<f64>,
    /// `easing` as written (`smooth` included); `None` when omitted.
    pub(in crate::parse) interp: Option<Interp>,
}

/// Whether the keyframes need the camera rig (smooth easing, framing anchor, or follow weight).
pub(in crate::parse) fn uses_rig(frames: &[CameraFrame]) -> bool {
    frames.iter().any(|f| {
        f.interp == Some(Interp::Smooth)
            || f.follow.is_some()
            || f.anchor.iter().any(Option::is_some)
    })
}

pub(in crate::parse) fn parse_camera(raw: &str) -> Result<Vec<CameraFrame>> {
    let frames: Vec<BTreeMap<String, serde_json::Value>> =
        serde_json::from_str(raw).context("expected a JSON array of objects")?;
    if frames.is_empty() {
        bail!("at least one camera keyframe is required");
    }
    let mut previous = CameraFrame {
        at: -1.0,
        zoom: 1.0,
        fx: 0.0,
        fy: 0.0,
        easing: Easing::Linear,
        given: [None; 2],
        anchor: [None; 2],
        follow: None,
        interp: None,
    };
    let mut out = Vec::with_capacity(frames.len());
    for (index, frame) in frames.into_iter().enumerate() {
        let parsed = (|| -> Result<CameraFrame> {
            for key in frame.keys() {
                if !matches!(
                    key.as_str(),
                    "at" | "zoom" | "x" | "y" | "easing" | "anchorX" | "anchorY" | "follow"
                ) {
                    bail!("unknown field: {key} (use at, zoom, x, y, easing, anchorX, anchorY, follow)");
                }
            }
            let number = |key: &str, fallback: f64| -> Result<f64> {
                match frame.get(key) {
                    Some(value) => value
                        .as_f64()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| anyhow!("{key} must be a finite number")),
                    None => Ok(fallback),
                }
            };
            let at = number("at", -1.0)?;
            if at < 0.0 || at <= previous.at {
                bail!("at is required, and times must be non-negative and increasing");
            }
            let zoom = number("zoom", previous.zoom as f64)?;
            if !(MIN_ZOOM..=MAX_ZOOM).contains(&zoom) {
                bail!("zoom must be between {MIN_ZOOM} and {MAX_ZOOM}");
            }
            let fx = number("x", previous.fx as f64)?;
            let fy = number("y", previous.fy as f64)?;
            if fx.abs() > MAX_OFFSET || fy.abs() > MAX_OFFSET {
                bail!("x and y must be offsets of at most 1,000,000 px");
            }
            let interp = match frame.get("easing") {
                None => None,
                Some(value) => Some(parse_camera_easing(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("easing must be a string"))?,
                )?),
            };
            let easing = match interp {
                Some(Interp::Eased(easing)) => easing,
                _ => Easing::Linear,
            };
            let fraction = |key: &str| -> Result<Option<f64>> {
                let Some(value) = frame.get(key) else {
                    return Ok(None);
                };
                let n = value
                    .as_f64()
                    .filter(|n| (0.0..=1.0).contains(n))
                    .ok_or_else(|| anyhow!("{key} must be a fraction from 0 to 1"))?;
                Ok(Some(n))
            };
            let follow = match frame.get("follow") {
                Some(serde_json::Value::Bool(on)) => Some(if *on { 1.0 } else { 0.0 }),
                _ => fraction("follow")?,
            };
            Ok(CameraFrame {
                at,
                zoom: zoom as f32,
                fx: fx as f32,
                fy: fy as f32,
                easing,
                given: [frame.get("x").map(|_| fx), frame.get("y").map(|_| fy)],
                anchor: [fraction("anchorX")?, fraction("anchorY")?],
                follow,
                interp,
            })
        })()
        .with_context(|| format!("camera keyframe {}", index + 1))?;
        out.push(parsed);
        previous = parsed;
    }
    Ok(out)
}

/// A camera easing: `smooth` (automatic tangents, continuous velocity) or any keyframe easing.
pub(in crate::parse) fn parse_camera_easing(raw: &str) -> Result<Interp> {
    if raw.trim() == "smooth" {
        return Ok(Interp::Smooth);
    }
    Ok(Interp::Eased(parse_keyframe_easing(raw)?))
}

/// Builds the motion of a camera container; the first and last poses hold outside the keyed interval.
pub(in crate::parse) fn motion_from_camera(base: Pose, frames: &[CameraFrame]) -> Motion {
    let track = |value: &dyn Fn(&CameraFrame) -> f32| {
        Track(
            frames
                .iter()
                .map(|f| TrackPoint {
                    at: f.at,
                    value: value(f),
                    easing: f.easing,
                })
                .collect(),
        )
    };
    Motion {
        base,
        x: track(&|f| base.x - f.zoom * f.fx),
        y: track(&|f| base.y - f.zoom * f.fy),
        scale: track(&|f| base.scale * f.zoom),
        rotate: Track::default(),
        opacity: Track::default(),
        reveal: Track::default(),
        wipe: None,
        filter: Default::default(),
        fx: Default::default(),
        clip: Vec::new(),
        path: None,
        rig: None,
        hold_before: true,
        hold_after: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_offset_translates_against_zoom() {
        let frames = parse_camera(
            r#"[{"at":0,"zoom":1},{"at":2,"zoom":2,"x":100,"y":-50,"easing":"ease-out-expo"}]"#,
        )
        .unwrap();
        let motion = motion_from_camera(Pose::default(), &frames);
        let end = motion.pose(2.0);
        assert_eq!(end.scale, 2.0);
        assert_eq!(end.x, -200.0);
        assert_eq!(end.y, 100.0);
        assert_eq!(motion.pose(0.0).scale, 1.0);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_camera("[]").is_err());
        assert!(parse_camera(r#"[{"at":0,"zoom":0}]"#).is_err());
        assert!(parse_camera(r#"[{"at":1},{"at":1}]"#).is_err());
        assert!(parse_camera(r#"[{"at":0,"rotate":3}]"#).is_err());
        assert!(parse_camera(r#"[{"at":0,"anchorX":1.5}]"#).is_err());
    }

    #[test]
    fn rig_fields_switch_to_the_rig() {
        let classic =
            parse_camera(r#"[{"at":0,"zoom":1},{"at":2,"zoom":2,"easing":"ease-out"}]"#).unwrap();
        assert!(!uses_rig(&classic));
        let smooth = parse_camera(r#"[{"at":0},{"at":2,"zoom":2,"easing":"smooth"}]"#).unwrap();
        assert!(uses_rig(&smooth));
        let follow = parse_camera(r#"[{"at":0,"follow":true,"anchorY":0.6}]"#).unwrap();
        assert_eq!(follow[0].follow, Some(1.0));
        assert_eq!(follow[0].anchor, [None, Some(0.6)]);
    }
}
