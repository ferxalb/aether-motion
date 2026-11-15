//! Cascade: merges matching rules and inline styles in specificity order.
use crate::parse::css::selectors::matches_selector;
use crate::parse::css::stylesheet::declarations;
use crate::parse::css::Rule;
use crate::parse::html::attr;
use anyhow::Result;
use kuchiki::NodeRef;

pub(in crate::parse) fn cascade(node: &NodeRef, rules: &[Rule]) -> Result<Vec<(String, String)>> {
    let mut matches: Vec<_> = rules
        .iter()
        .filter(|r| matches_selector(node, &r.selector))
        .collect();
    matches.sort_by_key(|r| (r.specificity, r.order));
    let mut properties = Vec::<(String, String)>::new();
    for rule in matches {
        properties.extend(rule.declarations.iter().cloned());
    }
    if let Some(inline) = attr(node, "style") {
        properties.extend(declarations(&inline)?);
    }
    Ok(properties)
}
