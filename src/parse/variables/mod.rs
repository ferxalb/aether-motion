//! Composition variables: typed declarations in `data-composition-variables`, values passed by the
//! caller (CLI, `aether.json`, or MCP), and the bindings that apply them to the document before it
//! is parsed, so every frame stays a pure function of time for one set of values.
//!
//! Submodules: `declare` (declarations and value coercion) and `bind` (`data-var-text`,
//! `data-var-src`, `data-var-if`, and `var(--id)` counting). `apply` is the entry point used by
//! `parse_file_with`; the `--id` custom properties it returns are pinned on the composition root.

mod bind;
mod declare;

use crate::model::{Variable, VariableUses, Variables};
use crate::parse::css::Vars;
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use serde_json::Value;
use std::path::Path;

/// Values passed for one render, keyed by variable id. Missing ids keep their declared default.
pub type VariableValues = serde_json::Map<String, Value>;

const ATTRIBUTE: &str = "data-composition-variables";

/// Parses a JSON object of variable values, such as `{"title": "Q4", "accent": "#ff3366"}`.
pub fn values_from_json(text: &str) -> Result<VariableValues> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| anyhow!("variable values are not valid JSON: {e}"))?;
    match value {
        Value::Object(values) => Ok(values),
        _ => bail!("variable values must be a JSON object keyed by variable id"),
    }
}

/// Reads a JSON file holding a variable-values object.
pub fn read_values_file(path: &Path) -> Result<VariableValues> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read variables file {}", path.display()))?;
    values_from_json(&text).with_context(|| format!("variables file {}", path.display()))
}

/// What applying the variables produced.
pub(in crate::parse) struct Applied {
    /// `--id` custom properties set on the composition root, over any declaration of the same name.
    pub(in crate::parse) pinned: Vars,
    pub(in crate::parse) report: Variables,
}

/// Reads the declarations from `<html>` or `metadata` (the element with `data-composition-id`),
/// resolves `values` against them, and applies every binding in `document`. `root` is the
/// composition root element.
pub(in crate::parse) fn apply(
    document: &NodeRef,
    metadata: &NodeRef,
    root: &NodeRef,
    values: &VariableValues,
) -> Result<Applied> {
    let html = document
        .descendants()
        .find(|n| n.as_element().is_some_and(|e| &*e.name.local == "html"));
    let mut sources: Vec<(NodeRef, String)> = Vec::new();
    for node in document.descendants() {
        let Some(el) = node.as_element() else {
            continue;
        };
        let Some(raw) = el.attributes.borrow().get(ATTRIBUTE).map(str::to_string) else {
            continue;
        };
        if Some(&node) != html.as_ref() && node != *metadata {
            bail!(
                "{ATTRIBUTE} belongs on <html> or on the element with data-composition-id, not on <{}>",
                el.name.local
            );
        }
        sources.push((node.clone(), raw));
    }
    if sources.len() > 1 {
        bail!("{ATTRIBUTE} appears on both <html> and the composition root; declare the variables once");
    }
    let declarations = match sources.first() {
        Some((_, raw)) => declare::parse_declarations(raw)?,
        None => Vec::new(),
    };
    if let Some(unknown) = values
        .keys()
        .find(|k| !declarations.iter().any(|d| &d.id == *k))
    {
        let declared: Vec<&str> = declarations.iter().map(|d| d.id.as_str()).collect();
        bail!(
            "a value was passed for variable {unknown:?}, which the composition does not declare in {ATTRIBUTE} (declared: {})",
            if declared.is_empty() { "none".into() } else { declared.join(", ") }
        );
    }
    let resolved = declarations
        .iter()
        .map(|d| match values.get(&d.id) {
            Some(value) => d.coerce(value, "value passed"),
            None => Ok(d.default.clone()),
        })
        .collect::<Result<Vec<_>>>()?;
    let entries: Vec<bind::Entry> = declarations
        .iter()
        .zip(&resolved)
        .map(|(declaration, value)| bind::Entry { declaration, value })
        .collect();
    let protected: Vec<NodeRef> = [metadata, root]
        .into_iter()
        .flat_map(|node| node.inclusive_ancestors())
        .collect();
    let mut uses = vec![VariableUses::default(); entries.len()];
    let removed = bind::bind(document, &protected, &entries, &mut uses)?;
    let pinned = entries
        .iter()
        .map(|e| {
            (
                format!("--{}", e.declaration.id),
                e.declaration.css(e.value),
            )
        })
        .collect();
    let declared = entries
        .iter()
        .zip(uses)
        .map(|(e, uses)| Variable {
            id: e.declaration.id.clone(),
            kind: e.declaration.kind,
            value: e.value.text.clone(),
            default: e.declaration.default.text.clone(),
            overridden: values.contains_key(&e.declaration.id),
            uses,
        })
        .collect();
    Ok(Applied {
        pinned,
        report: Variables { declared, removed },
    })
}
