//! Reads optional finishing controls from the composition metadata.

use super::html::{attr, signed_number};
use crate::model::Filmic;
use anyhow::{bail, Result};
use kuchiki::NodeRef;

pub(super) fn parse(node: &NodeRef) -> Result<Option<Filmic>> {
    let read = |name, min, max| {
        let value = attr(node, name)
            .map(|raw| signed_number(&raw, name))
            .transpose()?
            .unwrap_or(0.0);
        if !(min..=max).contains(&value) {
            bail!("{name} must be between {min} and {max}");
        }
        Ok(value)
    };
    let exposure = read("data-hf-exposure", -4.0, 4.0)?;
    let highlights = read("data-hf-highlight-rolloff", 0.0, 1.0)?;
    let shadows = read("data-hf-shadow-toe", 0.0, 1.0)?;
    Ok(
        (exposure != 0.0 || highlights != 0.0 || shadows != 0.0).then_some(Filmic {
            exposure,
            highlights,
            shadows,
        }),
    )
}
