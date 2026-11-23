//! HTML attributes for root-level native 3D rigid bodies and their centered colliders.

use super::{allowed, numbers, scalar, vector, MAX_ANGLE, MAX_COORDINATE};
use crate::parse::html::attr;
use crate::scene3d::physics::{Body, Collider};
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
        ],
    )?;
    for name in ["scale", "data-hf-keyframes"] {
        ensure!(
            attr(node, name).is_none(),
            "{name} is unavailable on <hf-body>; animate or scale its visual children"
        );
    }
    let dynamic = match attr(node, "type").as_deref() {
        Some("fixed") => false,
        Some("dynamic") => true,
        _ => bail!("<hf-body> needs type=\"fixed\" or type=\"dynamic\""),
    };
    if !dynamic {
        for name in ["mass", "velocity", "angular-velocity", "ccd"] {
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
        dynamic,
        friction: scalar(node, "friction", 0.5, (0.0, 1.0))?,
        restitution: scalar(node, "restitution", 0.0, (0.0, 1.0))?,
        mass: scalar(node, "mass", 1.0, (0.01, 10_000.0))?,
        velocity: vector(node, "velocity", [0.0; 3], MAX_COORDINATE)?,
        angular_velocity: vector(node, "angular-velocity", [0.0; 3], MAX_ANGLE)?,
        ccd,
    })
}
