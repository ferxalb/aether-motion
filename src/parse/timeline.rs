//! Resolution of `data-start` expressions (seconds or clip references with offsets).
use crate::parse::html::number;
use crate::parse::tree::Pending;
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;

/// Evaluates a `data-start` expression: seconds, or `id` / `id+offset` / `id-offset`, meaning the
/// end of the referenced clip (its start plus its `data-duration`) plus the offset.
fn eval_expression(
    raw: &str,
    pending: &[Pending],
    ids: &HashMap<String, usize>,
    memo: &mut [Option<f64>],
    visiting: &mut [bool],
) -> Result<f64> {
    if let Ok(n) = number(raw, "data-start") {
        return Ok(n);
    }
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
    Ok(resolve_start(referenced, pending, ids, memo, visiting)? + duration + offset)
}

fn checked(start: f64) -> Result<f64> {
    if !start.is_finite() || start < 0.0 {
        bail!("resolved data-start is out of range");
    }
    Ok(start)
}

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
    let start = if let Some(raw) = pending[idx].start_expr.as_deref() {
        eval_expression(raw, pending, ids, memo, visiting)?
    } else if let Some(parent) = pending[idx].element.parent {
        resolve_start(parent, pending, ids, memo, visiting)?
    } else {
        0.0
    };
    let start = checked(start)?;
    memo[idx] = Some(start);
    visiting[idx] = false;
    Ok(start)
}

/// Resolves the start of an `<audio>` clip: its own `data-start`, else its parent clip's start.
/// Audio has no `data-start` dependents, so it needs no memo entry of its own.
pub(in crate::parse) fn resolve_audio_start(
    start_expr: Option<&str>,
    parent: Option<usize>,
    pending: &[Pending],
    ids: &HashMap<String, usize>,
    memo: &mut [Option<f64>],
    visiting: &mut [bool],
) -> Result<f64> {
    let start = if let Some(raw) = start_expr {
        eval_expression(raw, pending, ids, memo, visiting)?
    } else if let Some(parent) = parent {
        resolve_start(parent, pending, ids, memo, visiting)?
    } else {
        0.0
    };
    checked(start)
}
