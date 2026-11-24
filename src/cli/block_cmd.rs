//! `block`: list the catalog of ready-made blocks and themes, or print one as HTML.
//!
//! The listing is the human view of `blocks::list` (grouped, wrapped, parameters with defaults);
//! printing a block writes only its HTML to stdout so it can be piped. `add` shares the listing.

use super::ui::{term, wrap, Style};
use crate::blocks;
use anyhow::Result;

pub fn run(raw: &[String]) -> Result<()> {
    if raw.is_empty() || raw == ["list"] {
        print!("{}", listing(Style::out())?);
    } else {
        print!("{}", blocks::run_cli(raw)?);
    }
    Ok(())
}

/// The catalog grouped into blocks and themes, with descriptions and parameter defaults.
pub fn listing(style: Style) -> Result<String> {
    let all = blocks::list()?;
    let pad = all.iter().map(|b| b.name.len()).max().unwrap_or(0);
    let width = term::width();
    let text_width = width.saturating_sub(pad + 4).max(30);
    let mut out = String::new();
    for (kind, title) in [("block", "Blocks"), ("theme", "Themes")] {
        let items: Vec<_> = all.iter().filter(|b| b.kind == kind).collect();
        if items.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&style.accent(title));
        out.push('\n');
        for block in items {
            let indent = " ".repeat(pad + 4);
            for (i, line) in wrap(&block.description, text_width).iter().enumerate() {
                if i == 0 {
                    let gap = " ".repeat(pad - block.name.len() + 2);
                    out.push_str(&format!("  {}{gap}{line}\n", style.bold(block.name)));
                } else {
                    out.push_str(&format!("{indent}{line}\n"));
                }
            }
            // The rigs' pose channels (all default 0) are summarized on one line after the rest.
            let channels = blocks::pose_channels();
            let posed = block
                .params
                .iter()
                .any(|p| channels.contains(&p.name.as_str()));
            let params: Vec<String> = block
                .params
                .iter()
                .filter(|p| !channels.contains(&p.name.as_str()))
                .map(|p| {
                    let value = if p.default.contains(' ') || p.default.is_empty() {
                        format!("\"{}\"", p.default)
                    } else {
                        p.default.clone()
                    };
                    format!("{}={value}", p.name)
                })
                .collect();
            if !params.is_empty() {
                // Parameters wrap as whole `key=value` items.
                let mut row = String::new();
                for param in params {
                    if !row.is_empty()
                        && row.chars().count() + 2 + param.chars().count() > text_width
                    {
                        out.push_str(&format!("{indent}{}\n", style.dim(&row)));
                        row.clear();
                    }
                    if !row.is_empty() {
                        row.push_str("  ");
                    }
                    row.push_str(&param);
                }
                out.push_str(&format!("{indent}{}\n", style.dim(&row)));
            }
            if posed {
                let line = format!(
                    "pose offsets (deg; body-x, body-y in px), default 0: {}",
                    channels.join(" ")
                );
                for row in wrap(&line, text_width) {
                    out.push_str(&format!("{indent}{}\n", style.dim(&row)));
                }
            }
        }
    }
    out.push('\n');
    out.push_str(&format!(
        "Insert a block: {}\n",
        style.bold("aether-motion add <block> --set key=value")
    ));
    out.push_str(&format!(
        "Print as HTML:  {}\n",
        style.bold("aether-motion block <name> --set key=value")
    ));
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn listing_names_every_block() {
        let text = super::listing(super::Style::plain()).unwrap();
        for block in crate::blocks::list().unwrap() {
            assert!(text.contains(block.name), "{} missing", block.name);
        }
    }
}
