//! Supported selectors: validation, specificity, and matching against a node.
use kuchiki::NodeRef;

pub(in crate::parse) fn valid_simple_selector(token: &str) -> bool {
    token == ":root"
        || (!token.is_empty()
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '*' | '-' | '_' | '#' | '.')))
}

pub(in crate::parse) fn specificity(selector: &str) -> usize {
    selector.bytes().filter(|c| *c == b'#').count() * 100
        + selector.bytes().filter(|c| *c == b'.').count() * 10
        + selector.matches(":root").count() * 10
        + selector
            .split_whitespace()
            .filter(|s| !s.starts_with(['.', '#', '*', ':']))
            .count()
}

pub(in crate::parse) fn matches_simple(node: &NodeRef, token: &str) -> bool {
    let Some(element) = node.as_element() else {
        return false;
    };
    let tag = element.name.local.as_ref();
    if token == ":root" {
        return tag == "html";
    }
    let attributes = element.attributes.borrow();
    let mut name = String::new();
    let mut mode = 't';
    let mut checks = Vec::new();
    for ch in token.chars().chain(std::iter::once('.')) {
        if ch == '#' || ch == '.' {
            if !name.is_empty() {
                checks.push((mode, name.clone()));
                name.clear();
            }
            mode = ch;
        } else {
            name.push(ch);
        }
    }
    checks.into_iter().all(|(mode, value)| match mode {
        't' => value == "*" || tag == value,
        '#' => attributes.get("id") == Some(value.as_str()),
        '.' => attributes
            .get("class")
            .is_some_and(|classes| classes.split_whitespace().any(|c| c == value)),
        _ => false,
    })
}

pub(in crate::parse) fn matches_selector(node: &NodeRef, selector: &str) -> bool {
    let parts: Vec<_> = selector.split_whitespace().collect();
    if !matches_simple(node, parts[parts.len() - 1]) {
        return false;
    }
    let mut ancestor = node.parent();
    for token in parts[..parts.len() - 1].iter().rev() {
        let Some(found) = ancestor.as_ref().and_then(|a| {
            std::iter::once(a.clone())
                .chain(a.ancestors())
                .find(|candidate| matches_simple(candidate, token))
        }) else {
            return false;
        };
        ancestor = found.parent();
    }
    true
}
