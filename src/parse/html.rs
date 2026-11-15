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

/// Whitespace-normalized direct text of `node`; each `<br>` becomes a `\n` line break.
///
/// A single trailing `<br>` adds no line, as in CSS.
pub(in crate::parse) fn direct_text(node: &NodeRef) -> String {
    let mut segments: Vec<Vec<String>> = vec![Vec::new()];
    for child in node.children() {
        if let Some(text) = child.as_text() {
            segments
                .last_mut()
                .unwrap()
                .push(text.borrow().to_string());
        } else if child
            .as_element()
            .is_some_and(|element| &*element.name.local == "br")
        {
            segments.push(Vec::new());
        }
    }
    let mut lines: Vec<String> = segments
        .iter()
        .map(|parts| parts.join(" ").split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    if lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    if lines.iter().all(String::is_empty) && lines.len() == 1 {
        return String::new();
    }
    lines.join("\n")
}
