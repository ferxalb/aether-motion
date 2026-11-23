//! Reads a bounded light shaft and validates explicit box occluders for native 3D.

use super::{allowed, color, scalar, vector, MAX_COORDINATE};
use crate::{model::Color, parse::css::Vars, parse::html::attr, scene3d::beam::Beam};
use anyhow::{anyhow, ensure, Result};
use kuchiki::NodeRef;

pub(super) fn parse(node: &NodeRef, vars: Option<&Vars>) -> Result<(Beam, Option<String>)> {
    allowed(
        node,
        &[
            "direction",
            "length",
            "radius",
            "density",
            "color",
            "occluder",
        ],
    )?;
    let direction = vector(node, "direction", [0.0, 1.0, 0.0], MAX_COORDINATE)?;
    ensure!(
        direction.iter().any(|v| v.abs() >= 1e-6),
        "beam direction must be nonzero"
    );
    let occluder = attr(node, "occluder")
        .map(|value| {
            value
                .strip_prefix('#')
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("beam occluder needs a box mesh reference such as #column"))
        })
        .transpose()?;
    Ok((
        Beam {
            direction,
            length: scalar(node, "length", 200.0, (1.0, 10_000.0))?,
            radius: scalar(node, "radius", 40.0, (1.0, 10_000.0))?,
            density: scalar(node, "density", 0.02, (0.0, 1.0))?,
            color: color(node, Color::WHITE, vars)?,
            occluder: None,
        },
        occluder,
    ))
}
