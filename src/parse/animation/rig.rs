//! Camera rig options (`data-hf-camera-*` on a camera container) and the conversion of rig-style
//! `data-hf-camera` keyframes into [`RigKey`]s.
//!
//! Options: `data-hf-camera-subject` (id of the followed descendant), `-lag` and `-lookahead`
//! (seconds, 0 to 5), `-deadzone` and `-offset` (`"x [y]"` px in camera space), `-axis`
//! (`both`, `x`, or `y`), `-anchor` (`"x [y]"` fractions of the camera box where the subject
//! sits), and `-easing` (the default easing of keys that name none: `smooth` or any keyframe
//! easing). Any other `data-hf-camera-*` attribute is an error, so typos never pass silently.
use crate::camera::curve::Interp;
use crate::camera::tracking::FollowSettings;
use crate::camera::{CameraRig, FocusValue, RigKey, Subject};
use crate::model::{Easing, Element};
use crate::parse::animation::camera::{parse_camera_easing, CameraFrame};
use crate::parse::html::{attr, number, signed_number};
use anyhow::{bail, Context, Result};
use kuchiki::NodeRef;
use std::collections::HashMap;
use std::sync::Arc;

const KNOWN: [&str; 8] = [
    "data-hf-camera-subject",
    "data-hf-camera-lag",
    "data-hf-camera-lookahead",
    "data-hf-camera-deadzone",
    "data-hf-camera-offset",
    "data-hf-camera-axis",
    "data-hf-camera-anchor",
    "data-hf-camera-easing",
];

/// Camera-level rig options.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::parse) struct RigOptions {
    /// `id` of the followed element.
    pub(in crate::parse) subject: Option<String>,
    pub(in crate::parse) settings: FollowSettings,
    /// Initial framing anchor.
    pub(in crate::parse) anchor: [f64; 2],
    /// Default easing for keys that name none.
    pub(in crate::parse) easing: Option<Interp>,
}

impl Default for RigOptions {
    fn default() -> Self {
        Self {
            subject: None,
            settings: FollowSettings::default(),
            anchor: [0.5, 0.5],
            easing: None,
        }
    }
}

/// One or two numbers (`"x [y]"`); a single value applies to both axes.
fn pair(raw: &str, name: &str, signed: bool) -> Result<[f64; 2]> {
    let values: Vec<f64> = raw
        .split_whitespace()
        .map(|part| {
            if signed {
                signed_number(part, name)
            } else {
                number(part, name)
            }
        })
        .collect::<Result<_>>()?;
    match values[..] {
        [v] => Ok([v, v]),
        [x, y] => Ok([x, y]),
        _ => bail!("{name} takes one or two numbers (\"x y\")"),
    }
}

/// The `data-hf-camera-*` options of `node`, or `None` when it has none.
pub(in crate::parse) fn parse_rig_options(node: &NodeRef) -> Result<Option<RigOptions>> {
    let Some(element) = node.as_element() else {
        return Ok(None);
    };
    let mut any = false;
    for key in element.attributes.borrow().map.keys() {
        let key = &*key.local;
        if key.starts_with("data-hf-camera-") {
            if !KNOWN.contains(&key) {
                bail!("unknown camera option {key} (use {})", KNOWN.join(", "));
            }
            any = true;
        }
    }
    if !any {
        return Ok(None);
    }
    let mut options = RigOptions {
        subject: attr(node, "data-hf-camera-subject")
            .map(|id| id.trim().trim_start_matches('#').to_string()),
        ..RigOptions::default()
    };
    if options.subject.as_deref() == Some("") {
        bail!("data-hf-camera-subject needs the id of the element to follow");
    }
    let settings = &mut options.settings;
    let seconds = |name: &str, fallback: f64| -> Result<f64> {
        let Some(raw) = attr(node, name) else {
            return Ok(fallback);
        };
        let value = number(raw.trim().trim_end_matches('s'), name)?;
        if value > 5.0 {
            bail!("{name} must be between 0 and 5 seconds");
        }
        Ok(value)
    };
    settings.lag = seconds("data-hf-camera-lag", settings.lag)?;
    settings.lookahead = seconds("data-hf-camera-lookahead", settings.lookahead)?;
    if let Some(raw) = attr(node, "data-hf-camera-deadzone") {
        settings.deadzone = pair(&raw, "data-hf-camera-deadzone", false)?;
        if settings.deadzone.iter().any(|&v| v > 10_000.0) {
            bail!("data-hf-camera-deadzone must be at most 10000 px");
        }
    }
    if let Some(raw) = attr(node, "data-hf-camera-offset") {
        settings.offset = pair(&raw, "data-hf-camera-offset", true)?;
        if settings.offset.iter().any(|v| v.abs() > 100_000.0) {
            bail!("data-hf-camera-offset must be at most 100000 px");
        }
    }
    if let Some(raw) = attr(node, "data-hf-camera-axis") {
        settings.axis = match raw.trim() {
            "both" => [true, true],
            "x" => [true, false],
            "y" => [false, true],
            other => bail!("data-hf-camera-axis must be both, x, or y, not {other}"),
        };
    }
    if let Some(raw) = attr(node, "data-hf-camera-anchor") {
        options.anchor = pair(&raw, "data-hf-camera-anchor", false)?;
        if options.anchor.iter().any(|&v| v > 1.0) {
            bail!("data-hf-camera-anchor takes fractions from 0 to 1 (0.5 0.5 is the center)");
        }
    }
    if let Some(raw) = attr(node, "data-hf-camera-easing") {
        options.easing = Some(parse_camera_easing(&raw).context("data-hf-camera-easing")?);
    }
    let tracking_option = [
        "data-hf-camera-lag",
        "data-hf-camera-lookahead",
        "data-hf-camera-deadzone",
        "data-hf-camera-offset",
        "data-hf-camera-axis",
        "data-hf-camera-anchor",
    ]
    .into_iter()
    .find(|name| attr(node, name).is_some());
    if let (None, Some(name)) = (&options.subject, tracking_option) {
        bail!("{name} needs data-hf-camera-subject=\"<id>\" (the element to follow)");
    }
    Ok(Some(options))
}

/// Rig keys from `data-hf-camera` keyframes: omitted values inherit, the follow weight starts at
/// 1 with a subject (0 without), and keys without an easing take the camera's default (`linear`).
pub(in crate::parse) fn keys_from_frames(
    frames: &[CameraFrame],
    options: &RigOptions,
) -> Vec<RigKey> {
    let default_interp = options.easing.unwrap_or(Interp::Eased(Easing::Linear));
    let mut anchor = options.anchor;
    let mut follow = if options.subject.is_some() { 1.0 } else { 0.0 };
    if frames.is_empty() {
        return vec![RigKey {
            at: 0.0,
            zoom: 1.0,
            focus: [FocusValue::Inherit; 2],
            anchor,
            follow,
            interp: default_interp,
        }];
    }
    frames
        .iter()
        .map(|frame| {
            for (slot, value) in anchor.iter_mut().zip(frame.anchor) {
                if let Some(value) = value {
                    *slot = value;
                }
            }
            follow = frame.follow.unwrap_or(follow);
            RigKey {
                at: frame.at,
                zoom: f64::from(frame.zoom),
                focus: frame
                    .given
                    .map(|v| v.map_or(FocusValue::Inherit, FocusValue::Fixed)),
                anchor,
                follow,
                interp: frame.interp.unwrap_or(default_interp),
            }
        })
        .collect()
}

/// Builds each rig camera once every element is known: resolves `data-hf-camera-subject` to a
/// descendant of the camera and the parallax of the plane that holds it. `rigs` holds
/// `(camera index, keys, options)`.
pub(in crate::parse) fn attach_rigs(
    elements: &mut [Element],
    rigs: Vec<(usize, Vec<RigKey>, RigOptions)>,
    ids: &HashMap<String, usize>,
) -> Result<()> {
    for (camera, keys, options) in rigs {
        let label = elements[camera]
            .id
            .clone()
            .unwrap_or_else(|| format!("<{}>", elements[camera].tag));
        let subject = match &options.subject {
            None => None,
            Some(id) => {
                let &element = ids.get(id).ok_or_else(|| {
                    anyhow::anyhow!("data-hf-camera-subject on {label} references unknown id: {id}")
                })?;
                let mut plane = element;
                loop {
                    match elements[plane].parent {
                        Some(parent) if parent == camera => break,
                        Some(parent) => {
                            if elements[parent].motion.rig.is_some() {
                                bail!("data-hf-camera-subject on {label}: {id} is inside another camera rig ({}); follow it from that camera", elements[parent].id.as_deref().unwrap_or("no id"));
                            }
                            plane = parent;
                        }
                        None => bail!("data-hf-camera-subject on {label}: {id} must be inside the camera container"),
                    }
                }
                if element == camera {
                    bail!("data-hf-camera-subject on {label} cannot name the camera itself");
                }
                let depth = f64::from(elements[plane].layout.parallax.unwrap_or(1.0));
                if depth <= 0.0 {
                    bail!("data-hf-camera-subject on {label}: {id} sits in a plane with data-hf-parallax=\"0\", which never pans and cannot be followed");
                }
                Some(Subject {
                    element,
                    depth,
                    settings: options.settings,
                })
            }
        };
        let base_scale = f64::from(elements[camera].motion.base.scale);
        let rig = CameraRig::new(keys, subject, base_scale)
            .with_context(|| format!("camera rig on {label}"))?;
        elements[camera].motion.rig = Some(Arc::new(rig));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::animation::camera::parse_camera;

    fn node(attrs: &str) -> NodeRef {
        use kuchiki::traits::TendrilSink;
        let doc = kuchiki::parse_html().one(format!("<div id='cam' {attrs}></div>"));
        doc.select_first("#cam").unwrap().as_node().clone()
    }

    #[test]
    fn options_parse_and_validate() {
        assert_eq!(parse_rig_options(&node("")).unwrap(), None);
        let options = parse_rig_options(&node(
            "data-hf-camera-subject='#hero' data-hf-camera-lag='0.4s' data-hf-camera-deadzone='12 4' data-hf-camera-axis='x' data-hf-camera-anchor='0.4 0.6' data-hf-camera-easing='smooth'",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(options.subject.as_deref(), Some("hero"));
        assert_eq!(options.settings.lag, 0.4);
        assert_eq!(options.settings.deadzone, [12.0, 4.0]);
        assert_eq!(options.settings.axis, [true, false]);
        assert_eq!(options.anchor, [0.4, 0.6]);
        assert_eq!(options.easing, Some(Interp::Smooth));
        for bad in [
            "data-hf-camera-lag='0.3'",
            "data-hf-camera-subject='a' data-hf-camera-lag='9'",
            "data-hf-camera-subject='a' data-hf-camera-axis='z'",
            "data-hf-camera-subject='a' data-hf-camera-anchor='1.2'",
            "data-hf-camera-subjekt='a'",
        ] {
            assert!(parse_rig_options(&node(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn frames_inherit_into_keys() {
        let frames = parse_camera(
            r#"[{"at":0,"zoom":1.2,"x":10},{"at":2,"zoom":1.5,"anchorX":0.4,"easing":"smooth"},{"at":3,"follow":0,"x":0}]"#,
        )
        .unwrap();
        let options = RigOptions {
            subject: Some("a".into()),
            ..RigOptions::default()
        };
        let keys = keys_from_frames(&frames, &options);
        assert_eq!(keys[0].focus[0], FocusValue::Fixed(10.0));
        assert_eq!(keys[1].focus[0], FocusValue::Inherit);
        assert_eq!(keys[1].anchor, [0.4, 0.5]);
        assert_eq!(keys[2].anchor, [0.4, 0.5]);
        assert_eq!((keys[1].follow, keys[2].follow), (1.0, 0.0));
        assert_eq!(keys[2].zoom, 1.5);
        assert_eq!(keys[1].interp, Interp::Smooth);
        assert_eq!(keys[2].interp, Interp::Eased(Easing::Linear));
    }
}
