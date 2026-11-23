//! HTML attributes for root-level rigid bodies, their colliders, and revolute joints.

use super::{allowed, numbers, scalar, vector, MAX_ANGLE, MAX_COORDINATE};
use crate::parse::html::attr;
use crate::scene3d::physics::{Body, BodyKind, Collider, Hinge};
use anyhow::{anyhow, bail, ensure, Result};
use kuchiki::NodeRef;

pub(super) fn body(node: &NodeRef, parent: Option<usize>) -> Result<Body> {
    ensure!(
        parent.is_none(),
        "<hf-body> must be a direct child of the 3D viewport"
    );
    allowed(
        node,
        &[
            "type",
            "shape",
            "size",
            "radius",
            "friction",
            "restitution",
            "mass",
            "velocity",
            "angular-velocity",
            "ccd",
            "angular-damping",
        ],
    )?;
    let kind = match attr(node, "type").as_deref() {
        Some("fixed") => BodyKind::Fixed,
        Some("dynamic") => BodyKind::Dynamic,
        Some("kinematic") => BodyKind::Kinematic,
        _ => bail!("<hf-body> needs type=\"fixed\", \"dynamic\", or \"kinematic\""),
    };
    ensure!(
        attr(node, "scale").is_none(),
        "scale is unavailable on <hf-body>; scale its visual children"
    );
    if kind != BodyKind::Kinematic {
        ensure!(attr(node, "data-hf-keyframes").is_none(), "data-hf-keyframes is unavailable on fixed or dynamic <hf-body>; animate its visual children");
    }
    if kind != BodyKind::Dynamic {
        for name in [
            "mass",
            "velocity",
            "angular-velocity",
            "ccd",
            "angular-damping",
        ] {
            ensure!(attr(node, name).is_none(), "{name} requires a dynamic body");
        }
    }
    let collider = match attr(node, "shape").as_deref() {
        Some("box") => {
            ensure!(
                attr(node, "radius").is_none(),
                "radius requires a sphere collider"
            );
            let raw = attr(node, "size")
                .ok_or_else(|| anyhow!("a box collider needs size=\"width height depth\""))?;
            let values = numbers(&raw, "size")?;
            let [w, h, d] = values[..] else {
                bail!("box collider size takes three numbers");
            };
            ensure!(
                [w, h, d].iter().all(|v| (1.0..=100_000.0).contains(v)),
                "box collider dimensions must be between 1 and 100000 px"
            );
            Collider::Box([w, h, d])
        }
        Some("sphere") => {
            ensure!(attr(node, "size").is_none(), "size requires a box collider");
            ensure!(
                attr(node, "radius").is_some(),
                "a sphere collider needs a radius"
            );
            Collider::Sphere(scalar(node, "radius", 1.0, (1.0, 100_000.0))?)
        }
        _ => bail!("<hf-body> needs shape=\"box\" or shape=\"sphere\""),
    };
    let ccd = match attr(node, "ccd").as_deref() {
        None | Some("false") => false,
        Some("" | "true") => true,
        _ => bail!("ccd must be true or false"),
    };
    Ok(Body {
        collider,
        kind,
        friction: scalar(node, "friction", 0.5, (0.0, 1.0))?,
        restitution: scalar(node, "restitution", 0.0, (0.0, 1.0))?,
        mass: scalar(node, "mass", 1.0, (0.01, 10_000.0))?,
        velocity: vector(node, "velocity", [0.0; 3], MAX_COORDINATE)?,
        angular_velocity: vector(node, "angular-velocity", [0.0; 3], MAX_ANGLE)?,
        ccd,
        angular_damping: scalar(node, "angular-damping", 0.0, (0.0, 100.0))?,
    })
}

pub(super) fn hinge(node: &NodeRef, parent: Option<usize>) -> Result<Hinge> {
    ensure!(
        parent.is_none(),
        "<hf-hinge> must be a direct child of the 3D viewport"
    );
    allowed(
        node,
        &["body-a", "body-b", "anchor-a", "anchor-b", "axis", "limits"],
    )?;
    for name in ["position", "rotation", "scale", "data-hf-keyframes"] {
        ensure!(
            attr(node, name).is_none(),
            "{name} is unavailable on <hf-hinge>; use the bodies' local anchors and axis"
        );
    }
    let body_id = |name| {
        attr(node, name)
            .and_then(|v| {
                v.strip_prefix('#')
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            })
            .ok_or_else(|| anyhow!("{name} needs a body reference such as #lamp"))
    };
    let axis = vector(node, "axis", [0.0, 0.0, 1.0], 1.0)?;
    ensure!(
        axis.iter().any(|v| v.abs() >= 1e-6),
        "hinge axis must have a nonzero direction"
    );
    let limits = attr(node, "limits")
        .map(|raw| {
            let values = numbers(&raw, "limits")?;
            let [min, max] = values[..] else {
                bail!("limits takes two angles in degrees");
            };
            ensure!(
                min > -180.0 && max < 180.0 && min < max,
                "hinge limits must be strictly increasing between -180 and 180 degrees"
            );
            Ok([min, max])
        })
        .transpose()?;
    Ok(Hinge {
        bodies: [body_id("body-a")?, body_id("body-b")?],
        anchors: [
            vector(node, "anchor-a", [0.0; 3], MAX_COORDINATE)?,
            vector(node, "anchor-b", [0.0; 3], MAX_COORDINATE)?,
        ],
        axis,
        limits,
    })
}
