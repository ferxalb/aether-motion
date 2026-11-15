//! Placeholder templates: `{{name|default}}` scanning, parameter listing, and HTML-escaped substitution.
//!
//! Names use lowercase letters, digits, and `-`. A name that appears several times takes the default of
//! its first occurrence everywhere, so one parameter always renders to one value.

use anyhow::{anyhow, bail, Result};
use std::collections::BTreeMap;

/// One template parameter with the default used when no value is given.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub default: String,
}

struct Slot<'a> {
    start: usize,
    end: usize,
    name: &'a str,
    default: &'a str,
}

fn slots(template: &str) -> Result<Vec<Slot<'_>>> {
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(open) = template[pos..].find("{{") {
        let start = pos + open;
        let inner_start = start + 2;
        let close = template[inner_start..]
            .find("}}")
            .ok_or_else(|| anyhow!("unclosed placeholder at byte {start}"))?;
        let inner = &template[inner_start..inner_start + close];
        let (name, default) = inner.split_once('|').unwrap_or((inner, ""));
        let name = name.trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            bail!("invalid placeholder name '{name}'");
        }
        let end = inner_start + close + 2;
        found.push(Slot {
            start,
            end,
            name,
            default,
        });
        pos = end;
    }
    Ok(found)
}

/// Parameters in order of first appearance.
pub fn params(template: &str) -> Result<Vec<Param>> {
    let mut out: Vec<Param> = Vec::new();
    for slot in slots(template)? {
        if !out.iter().any(|p| p.name == slot.name) {
            out.push(Param {
                name: slot.name.to_string(),
                default: slot.default.to_string(),
            });
        }
    }
    Ok(out)
}

/// Escapes text so it is safe inside element content and quoted attributes.
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Fills the template. Values are HTML-escaped; defaults are used verbatim. Unknown names are errors.
pub fn render(template: &str, values: &BTreeMap<String, String>) -> Result<String> {
    let known = params(template)?;
    for key in values.keys() {
        if !known.iter().any(|p| &p.name == key) {
            let names: Vec<&str> = known.iter().map(|p| p.name.as_str()).collect();
            bail!(
                "unknown parameter '{key}'; available: {}",
                names.join(", ")
            );
        }
    }
    let mut out = String::with_capacity(template.len());
    let mut last = 0;
    for slot in slots(template)? {
        out.push_str(&template[last..slot.start]);
        match values.get(slot.name) {
            Some(value) => out.push_str(&escape_html(value)),
            None => {
                let default = known
                    .iter()
                    .find(|p| p.name == slot.name)
                    .map_or(slot.default, |p| p.default.as_str());
                out.push_str(default);
            }
        }
        last = slot.end;
    }
    out.push_str(&template[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn lists_parameters_once_with_first_default() {
        let found = params("a {{id|x}} b {{title|Hi}} c {{id|other}}").unwrap();
        assert_eq!(
            found,
            [
                Param {
                    name: "id".into(),
                    default: "x".into()
                },
                Param {
                    name: "title".into(),
                    default: "Hi".into()
                }
            ]
        );
    }

    #[test]
    fn substitutes_values_and_defaults() {
        let template = "<p id=\"{{id|a}}\">{{text|none}} {{id|zzz}}</p>";
        assert_eq!(
            render(template, &values(&[("text", "hello")])).unwrap(),
            "<p id=\"a\">hello a</p>"
        );
        assert_eq!(
            render(template, &values(&[("id", "b")])).unwrap(),
            "<p id=\"b\">none b</p>"
        );
    }

    #[test]
    fn escapes_values_but_not_defaults() {
        let out = render("{{t|<b>}}", &values(&[("t", "<i a=\"1\">&")])).unwrap();
        assert_eq!(out, "&lt;i a=&quot;1&quot;&gt;&amp;");
        assert_eq!(render("{{t|<b>}}", &values(&[])).unwrap(), "<b>");
    }

    #[test]
    fn rejects_unknown_and_malformed_input() {
        let err = render("{{a|1}}", &values(&[("b", "2")])).unwrap_err();
        assert!(err.to_string().contains("available: a"));
        assert!(params("{{a|1").is_err());
        assert!(params("{{Bad Name|1}}").is_err());
    }
}
