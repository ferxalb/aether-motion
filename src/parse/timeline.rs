//! Resolution of `data-start` expressions (seconds or clip references with offsets).
use crate::parse::html::number;
use crate::parse::tree::Pending;
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;

pub(in crate::parse) fn resolve_start(
    idx: usize,
    pending: &[Pending],
    ids: &HashMap<String, usize>,
    memo: &mut [Option<f64>],
    visiting: &mut [bool],
) -> Result<f64> {
    if let Some(n) = memo[idx] {
        return Ok(n);
    }
    if visiting[idx] {
        bail!(
            "cycle in data-start near {}",
            pending[idx]
                .element
                .id
                .as_deref()
                .unwrap_or("element with no id")
        );
    }
    visiting[idx] = true;
    let raw = pending[idx].start_expr.as_deref();
    let start = if let Some(raw) = raw {
        if let Ok(n) = number(raw, "data-start") {
            n
        } else {
            let normalized = raw.replace(' ', "");
            let split = normalized
                .char_indices()
                .skip(1)
                .find(|(_, c)| *c == '+' || *c == '-');
            let (id, offset) = if let Some((pos, sign)) = split {
                let offset = number(&normalized[pos + 1..], "offset data-start")?;
                (
                    &normalized[..pos],
                    if sign == '-' { -offset } else { offset },
                )
            } else {
                (normalized.as_str(), 0.0)
            };
            let referenced = *ids
                .get(id)
                .ok_or_else(|| anyhow!("data-start references unknown id: {id}"))?;
            let duration = pending[referenced]
                .duration
                .ok_or_else(|| anyhow!("data-start references {id} without data-duration"))?;
            resolve_start(referenced, pending, ids, memo, visiting)? + duration + offset
        }
    } else if let Some(parent) = pending[idx].element.parent {
        resolve_start(parent, pending, ids, memo, visiting)?
    } else {
        0.0
    };
    if !start.is_finite() || start < 0.0 {
        bail!("resolved data-start is out of range");
    }
    memo[idx] = Some(start);
    visiting[idx] = false;
    Ok(start)
}
