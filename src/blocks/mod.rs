//! Ready-made blocks and themes: parametrized HTML fragments for premium compositions.
//!
//! `catalog` embeds the templates from `blocks/` and lists the generated rigs, `template` fills the
//! templates' `{{name|default}}` placeholders, and `rig` computes the controllable human figures.
//! This facade serves the CLI (`aether-motion block ...`) and the `aether_block` MCP tool.

mod catalog;
mod rig;
mod template;

pub use template::Param;

use anyhow::{anyhow, bail, Result};
use catalog::{Entry, Kind, Source, ENTRIES};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// The pose channels of the human rigs: every one is also a parameter (a static offset, default 0)
/// and a key of their `timeline` and `sequence` steps.
pub fn pose_channels() -> &'static [&'static str] {
    &rig::CHANNELS
}

/// Public description of one catalog entry.
#[derive(Clone, Debug)]
pub struct BlockInfo {
    pub name: &'static str,
    /// `block` or `theme`.
    pub kind: &'static str,
    pub description: String,
    pub params: Vec<Param>,
}

/// Text of the leading `<!-- ... -->` comment of a template.
fn description(template: &str) -> String {
    let first = template.lines().next().unwrap_or("").trim();
    first
        .strip_prefix("<!--")
        .and_then(|rest| rest.strip_suffix("-->"))
        .map_or_else(String::new, |text| text.trim().to_string())
}

fn info(entry: &Entry) -> Result<BlockInfo> {
    Ok(BlockInfo {
        name: entry.name,
        kind: match entry.kind {
            Kind::Block => "block",
            Kind::Theme => "theme",
        },
        description: match entry.source {
            Source::Template(text) => description(text),
            Source::Rig(rig) => rig.description().to_string(),
        },
        params: match entry.source {
            Source::Template(text) => template::params(text)?,
            Source::Rig(rig) => rig
                .params()
                .into_iter()
                .map(|(name, default)| Param { name, default })
                .collect(),
        },
    })
}

/// Every block and theme, in catalog order.
pub fn list() -> Result<Vec<BlockInfo>> {
    ENTRIES.iter().map(info).collect()
}

/// The catalog as JSON, for the MCP tool.
pub fn list_json() -> Result<Value> {
    Ok(Value::Array(
        list()?
            .into_iter()
            .map(|b| {
                json!({
                    "name": b.name,
                    "kind": b.kind,
                    "description": b.description,
                    "params": b.params.iter().map(|p| json!({"name": p.name, "default": p.default})).collect::<Vec<_>>(),
                })
            })
            .collect(),
    ))
}

/// Renders the block or theme `name` with `values` (unset parameters use their defaults).
pub fn render(name: &str, values: &BTreeMap<String, String>) -> Result<String> {
    let entry = ENTRIES.iter().find(|e| e.name == name).ok_or_else(|| {
        let names: Vec<&str> = ENTRIES.iter().map(|e| e.name).collect();
        anyhow!("unknown block '{name}'; available: {}", names.join(", "))
    })?;
    match entry.source {
        Source::Template(text) => template::render(text, values),
        Source::Rig(rig) => rig.render(values),
    }
}

fn format_list() -> Result<String> {
    let mut out = String::new();
    for b in list()? {
        out.push_str(&format!("{} ({}): {}\n", b.name, b.kind, b.description));
        for p in &b.params {
            out.push_str(&format!("    {} = {}\n", p.name, p.default));
        }
    }
    Ok(out)
}

/// `aether-motion block [list | <name> [--set key=value]...]`; returns the text to print.
pub fn run_cli(args: &[String]) -> Result<String> {
    match args.first().map(String::as_str) {
        None | Some("list") => format_list(),
        Some(name) => {
            let mut values = BTreeMap::new();
            let mut rest = args[1..].iter();
            while let Some(flag) = rest.next() {
                if flag != "--set" {
                    bail!("unknown option: {flag}; use --set key=value");
                }
                let pair = rest
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --set"))?;
                let (key, value) = pair
                    .split_once('=')
                    .ok_or_else(|| anyhow!("--set expects key=value: {pair}"))?;
                values.insert(key.to_string(), value.to_string());
            }
            render(name, &values)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_a_description_and_valid_placeholders() {
        for b in list().unwrap() {
            assert!(!b.description.is_empty(), "{} has no description", b.name);
        }
    }

    #[test]
    fn blocks_render_with_defaults_and_overrides() {
        for entry in ENTRIES {
            let html = render(entry.name, &BTreeMap::new()).unwrap();
            assert!(!html.is_empty(), "{} rendered nothing", entry.name);
            assert!(!html.contains("{{"), "{} left a placeholder", entry.name);
        }
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), "intro".to_string());
        values.insert("title".to_string(), "Hello <world>".to_string());
        let html = render("hero-title", &values).unwrap();
        assert!(html.contains("id=\"intro\""));
        assert!(html.contains("Hello &lt;world&gt;"));
        assert!(render("nope", &values).is_err());
    }

    #[test]
    fn cli_parses_set_pairs() {
        let args: Vec<String> = ["cta", "--set", "button=Join"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(run_cli(&args).unwrap().contains("Join"));
        assert!(run_cli(&["cta".into(), "--bad".into()]).is_err());
        assert!(run_cli(&["cta".into(), "--set".into(), "oops".into()]).is_err());
    }
}
