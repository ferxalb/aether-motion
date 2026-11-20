//! Declarative variable bindings applied to the DOM before the stylesheet and element tree are read:
//! `data-var-if` removes elements, `data-var-text` writes an element's own text, `data-var-src`
//! sets a media `src`, and `var(--id)` references are counted for the report.
//!
//! Every binding is validated against the declarations first, whatever the values are, so a
//! composition that is valid with its defaults stays valid with any accepted value.

use super::declare::{Declaration, Resolved};
use crate::model::{VariableKind, VariableUses};
use anyhow::{anyhow, bail, Result};
use kuchiki::NodeRef;

/// Attributes that bind a variable; any other `data-var-*` (or `data-var`) is an error.
const BINDINGS: [&str; 3] = ["data-var-text", "data-var-src", "data-var-if"];
/// Elements whose `src` may come from a variable.
const SRC_TAGS: [&str; 4] = ["img", "video", "audio", "source"];
/// Elements that hold no text of their own.
const NO_TEXT_TAGS: [&str; 13] = [
    "html", "head", "body", "meta", "title", "link", "style", "br", "img", "video", "audio",
    "source", "svg",
];

/// One declared variable with the value this parse uses.
pub(super) struct Entry<'a> {
    pub(super) declaration: &'a Declaration,
    pub(super) value: &'a Resolved,
}

/// A parsed `data-var-if` test.
enum Condition {
    /// `id` or `!id` on a boolean.
    Flag { negated: bool },
    /// `id=value` or `id!=value` on any other type.
    Equals { expected: String, negated: bool },
}

impl Condition {
    fn holds(&self, entry: &Entry) -> bool {
        match self {
            Self::Flag { negated } => entry.value.boolean == Some(!negated),
            Self::Equals { expected, negated } => {
                let equal = match entry.value.number {
                    Some(n) => expected.parse::<f64>().is_ok_and(|e| e == n),
                    None => *expected == entry.value.text,
                };
                equal != *negated
            }
        }
    }
}

fn label(node: &NodeRef) -> String {
    let Some(el) = node.as_element() else {
        return "node".into();
    };
    match el.attributes.borrow().get("id") {
        Some(id) => format!("<{}> #{id}", el.name.local),
        None => format!("<{}>", el.name.local),
    }
}

fn find(entries: &[Entry], id: &str, binding: &str, node: &NodeRef) -> Result<usize> {
    entries
        .iter()
        .position(|e| e.declaration.id == id)
        .ok_or_else(|| {
            let declared: Vec<&str> = entries.iter().map(|e| e.declaration.id.as_str()).collect();
            anyhow!(
                "{binding}=\"{id}\" on {}: no variable {id:?} is declared in data-composition-variables (declared: {})",
                label(node),
                if declared.is_empty() { "none".into() } else { declared.join(", ") }
            )
        })
}

fn parse_condition(raw: &str, entries: &[Entry], node: &NodeRef) -> Result<(usize, Condition)> {
    let raw = raw.trim();
    let binding = "data-var-if";
    let (id, condition) = if let Some((id, expected)) = raw.split_once("!=") {
        let expected = expected.trim().to_string();
        (
            id.trim(),
            Condition::Equals {
                expected,
                negated: true,
            },
        )
    } else if let Some((id, expected)) = raw.split_once('=') {
        let expected = expected.trim().to_string();
        (
            id.trim(),
            Condition::Equals {
                expected,
                negated: false,
            },
        )
    } else if let Some(id) = raw.strip_prefix('!') {
        (id.trim(), Condition::Flag { negated: true })
    } else {
        (raw, Condition::Flag { negated: false })
    };
    let idx = find(entries, id, binding, node)?;
    let declaration = entries[idx].declaration;
    let place = label(node);
    match (&condition, declaration.kind) {
        (Condition::Flag { .. }, VariableKind::Boolean) => {}
        (Condition::Flag { .. }, kind) => bail!(
            "{binding}=\"{raw}\" on {place}: {id} has type {}; test it with {id}=value or {id}!=value",
            kind.name()
        ),
        (Condition::Equals { .. }, VariableKind::Boolean) => {
            bail!("{binding}=\"{raw}\" on {place}: {id} has type boolean; test it with {id} or !{id}")
        }
        (Condition::Equals { expected, .. }, VariableKind::Number) => {
            if expected.parse::<f64>().map_or(true, |n| !n.is_finite()) {
                bail!("{binding}=\"{raw}\" on {place}: {id} has type number; compare it with a number");
            }
        }
        (Condition::Equals { expected, .. }, VariableKind::Enum) => {
            // Comparing against a value the enum cannot take is a typo, not a test.
            declaration.coerce(&serde_json::Value::String(expected.clone()), "compared value")?;
        }
        (Condition::Equals { .. }, _) => {}
    }
    Ok((idx, condition))
}

/// Counts `var(--name)` references in CSS text into `uses`.
fn count_css(css: &str, entries: &[Entry], uses: &mut [VariableUses]) {
    let mut rest = css;
    while let Some(pos) = rest.find("var(") {
        rest = &rest[pos + 4..];
        let end = rest.find([',', ')']).unwrap_or(rest.len());
        if let Some(name) = rest[..end].trim().strip_prefix("--") {
            if let Some(idx) = entries.iter().position(|e| e.declaration.id == name) {
                uses[idx].css += 1;
            }
        }
    }
}

/// Replaces the element's own text, keeping its child elements: the first direct text node takes
/// the value and the others are emptied; without one, the value is inserted first.
fn set_own_text(node: &NodeRef, value: &str) {
    let children: Vec<NodeRef> = node.children().collect();
    if !children.iter().any(|c| c.as_element().is_some()) {
        for child in children {
            child.detach();
        }
        node.append(NodeRef::new_text(value));
        return;
    }
    let mut written = false;
    for child in children {
        if let Some(text) = child.as_text() {
            *text.borrow_mut() = if written {
                String::new()
            } else {
                value.to_string()
            };
            written = true;
        }
    }
    if !written {
        node.prepend(NodeRef::new_text(value));
    }
}

/// Validates and applies every binding in `document`. `protected` lists the elements that cannot
/// be removed (the composition root, the metadata element, and their ancestors). Returns how many
/// elements `data-var-if` removed, counting each removed subtree once.
pub(super) fn bind(
    document: &NodeRef,
    protected: &[NodeRef],
    entries: &[Entry],
    uses: &mut [VariableUses],
) -> Result<usize> {
    let mut texts = Vec::new();
    let mut sources = Vec::new();
    let mut conditions = Vec::new();
    for node in document.descendants() {
        if let Some(text) = node.as_text() {
            let in_style = node
                .parent()
                .and_then(|p| p.as_element().map(|e| &*e.name.local == "style"))
                .unwrap_or(false);
            if in_style {
                count_css(&text.borrow(), entries, uses);
            }
            continue;
        }
        let Some(el) = node.as_element() else {
            continue;
        };
        let tag = el.name.local.to_string();
        let attributes = el.attributes.borrow();
        for name in attributes.map.keys() {
            let name = &*name.local;
            if (name == "data-var" || name.starts_with("data-var-")) && !BINDINGS.contains(&name) {
                bail!(
                    "{name} on {} is not a variable binding; use data-var-text, data-var-src, or data-var-if",
                    label(&node)
                );
            }
            if name == "data-variable-values" && !attributes.contains("data-composition-src") {
                bail!("data-variable-values on {} is not supported without data-composition-src; for root variables pass values to the render", label(&node));
            }
        }
        if let Some(style) = attributes.get("style") {
            count_css(style, entries, uses);
        }
        if let Some(id) = attributes.get("data-var-text") {
            let idx = find(entries, id.trim(), "data-var-text", &node)?;
            let in_svg = node
                .ancestors()
                .any(|a| a.as_element().is_some_and(|e| &*e.name.local == "svg"));
            if NO_TEXT_TAGS.contains(&tag.as_str()) || in_svg {
                bail!(
                    "data-var-text on {} cannot set text there; put it on a text element such as <div>, <p>, or <span>",
                    label(&node)
                );
            }
            uses[idx].text += 1;
            texts.push((node.clone(), idx));
        }
        if let Some(id) = attributes.get("data-var-src") {
            let idx = find(entries, id.trim(), "data-var-src", &node)?;
            if !SRC_TAGS.contains(&tag.as_str()) {
                bail!(
                    "data-var-src on {} is only allowed on <img>, <video>, <audio>, and <source>",
                    label(&node)
                );
            }
            let kind = entries[idx].declaration.kind;
            if !matches!(
                kind,
                VariableKind::Image | VariableKind::String | VariableKind::Enum
            ) {
                bail!(
                    "data-var-src on {}: {} has type {}; a source needs an image, string, or enum variable",
                    label(&node),
                    entries[idx].declaration.id,
                    kind.name()
                );
            }
            uses[idx].src += 1;
            sources.push((node.clone(), idx));
        }
        if let Some(raw) = attributes.get("data-var-if") {
            let (idx, condition) = parse_condition(raw, entries, &node)?;
            if protected.contains(&node) || matches!(tag.as_str(), "html" | "head" | "body") {
                bail!(
                    "data-var-if on {} cannot remove the composition root, its ancestors, or <head>",
                    label(&node)
                );
            }
            uses[idx].condition += 1;
            conditions.push((node.clone(), idx, condition));
        }
    }
    let mut removed: Vec<NodeRef> = Vec::new();
    for (node, idx, condition) in &conditions {
        if condition.holds(&entries[*idx]) {
            continue;
        }
        if !node.ancestors().any(|a| removed.contains(&a)) {
            removed.push(node.clone());
        }
    }
    for node in &removed {
        node.detach();
    }
    for (node, idx) in &texts {
        set_own_text(node, &entries[*idx].value.text);
    }
    for (node, idx) in &sources {
        let value = &entries[*idx].value.text;
        // An empty value keeps the authored `src` as the fallback.
        if !value.is_empty() {
            if let Some(el) = node.as_element() {
                el.attributes.borrow_mut().insert("src", value.clone());
            }
        }
    }
    Ok(removed.len())
}
