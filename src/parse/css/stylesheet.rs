//! Stylesheet parsing: comments, declarations, rules, and `@keyframes` blocks.
use crate::parse::css::selectors::{specificity, valid_simple_selector};
use crate::parse::css::{RawFrame, Rule, Stylesheet};
use anyhow::{anyhow, bail, Result};

pub(in crate::parse) fn strip_comments(css: &str) -> Result<String> {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut closed = false;
            while let Some(x) = chars.next() {
                if x == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    closed = true;
                    break;
                }
            }
            if !closed {
                bail!("unclosed CSS comment");
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

pub(in crate::parse) fn declarations(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for declaration in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let (key, value) = declaration
            .split_once(':')
            .ok_or_else(|| anyhow!("invalid CSS declaration: {declaration}"))?;
        let key = key.trim();
        let key = if key.starts_with("--") {
            key.to_string()
        } else {
            key.to_ascii_lowercase()
        };
        let value = value.trim().to_string();
        if value.is_empty() {
            bail!("empty CSS value for {key}");
        }
        out.push((key, value));
    }
    Ok(out)
}

pub(in crate::parse) fn matching_brace(text: &str, open: usize) -> Result<usize> {
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(open + i);
                }
            }
            _ => {}
        }
    }
    bail!("CSS block without '}}'")
}

pub(in crate::parse) fn parse_keyframe_blocks(body: &str) -> Result<Vec<RawFrame>> {
    let mut rest = body;
    let mut frames = Vec::new();
    while !rest.trim().is_empty() {
        let open = rest
            .find('{')
            .ok_or_else(|| anyhow!("@keyframes: block without '{{'"))?;
        let close = matching_brace(rest, open)?;
        let declarations = declarations(&rest[open + 1..close])?;
        for selector in rest[..open].split(',').map(str::trim) {
            let offset = match selector {
                "from" => 0.0,
                "to" => 1.0,
                _ => {
                    let pct: f64 = selector
                        .strip_suffix('%')
                        .and_then(|n| n.trim().parse().ok())
                        .filter(|n| (0.0..=100.0).contains(n))
                        .ok_or_else(|| {
                            anyhow!("@keyframes: invalid selector (use from, to or 0%-100%): {selector}")
                        })?;
                    pct / 100.0
                }
            };
            frames.push(RawFrame {
                offset,
                declarations: declarations.clone(),
            });
        }
        rest = &rest[close + 1..];
    }
    if frames.is_empty() {
        bail!("@keyframes without frames");
    }
    Ok(frames)
}

pub(in crate::parse) fn parse_css(css: &str, sheet: &mut Stylesheet) -> Result<()> {
    let css = strip_comments(css)?;
    let mut rest = css.as_str();
    while !rest.trim().is_empty() {
        let open = rest
            .find('{')
            .ok_or_else(|| anyhow!("CSS block is missing '{{'"))?;
        let close = matching_brace(rest, open)?;
        let prelude = rest[..open].trim();
        let body = &rest[open + 1..close];
        if let Some(name) = prelude
            .strip_prefix("@keyframes")
            .filter(|n| n.starts_with(char::is_whitespace))
        {
            let name = name.trim();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            {
                bail!("@keyframes: invalid name: {name}");
            }
            sheet
                .keyframes
                .insert(name.to_string(), parse_keyframe_blocks(body)?);
        } else if prelude.starts_with('@') {
            bail!("unsupported CSS at-rule (only @keyframes): {prelude}");
        } else {
            let decls = declarations(body)?;
            for selector in prelude.split(',').map(str::trim) {
                if selector.is_empty() || !selector.split_whitespace().all(valid_simple_selector) {
                    bail!("unsupported CSS selector: {selector}");
                }
                sheet.rules.push(Rule {
                    selector: selector.to_string(),
                    declarations: decls.clone(),
                    specificity: specificity(selector),
                    order: sheet.rules.len(),
                });
            }
        }
        rest = &rest[close + 1..];
    }
    Ok(())
}
