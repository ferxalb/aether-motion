//! HTML node helpers: attribute access, number parsing, and direct text content.
use anyhow::{bail, Context, Result};
use kuchiki::NodeRef;

pub(in crate::parse) fn attr(node: &NodeRef, key: &str) -> Option<String> {
    node.as_element()
        .and_then(|e| e.attributes.borrow().get(key).map(str::to_owned))
}

pub(in crate::parse) fn number(value: &str, name: &str) -> Result<f64> {
    let n: f64 = value
        .parse()
        .with_context(|| format!("{name}: invalid number: {value}"))?;
    if !n.is_finite() || n < 0.0 {
        bail!("{name}: must be a finite non-negative number: {value}");
    }
    Ok(n)
}

pub(in crate::parse) fn direct_text(node: &NodeRef) -> String {
    node.children()
        .filter_map(|n| n.as_text().map(|t| t.borrow().to_string()))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
