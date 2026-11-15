//! Custom properties: inheritance and `var()` substitution.
use crate::parse::css::Vars;
use anyhow::{anyhow, bail, Result};

/// Replaces every `var(--name, fallback)` with its value.
pub(in crate::parse) fn substitute(value: &str, vars: &Vars, depth: usize) -> Result<String> {
    if depth > 16 {
        bail!("var() is circular or nested too deeply: {value}");
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(pos) = rest.find("var(") {
        out.push_str(&rest[..pos]);
        let args = pos + 4;
        let mut level = 1usize;
        let mut end = None;
        for (i, c) in rest[args..].char_indices() {
            match c {
                '(' => level += 1,
                ')' => {
                    level -= 1;
                    if level == 0 {
                        end = Some(args + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.ok_or_else(|| anyhow!("var() without ')': {value}"))?;
        let inner = &rest[args..end];
        let (name, fallback) = match inner.split_once(',') {
            Some((name, fallback)) => (name.trim(), Some(fallback.trim())),
            None => (inner.trim(), None),
        };
        if !name.starts_with("--") {
            bail!("var(): expected --name, got {name}");
        }
        let resolved = match (vars.get(name), fallback) {
            (Some(found), _) => substitute(found, vars, depth + 1)?,
            (None, Some(fallback)) => substitute(fallback, vars, depth + 1)?,
            (None, None) => bail!("CSS variable not defined: {name}"),
        };
        out.push_str(&resolved);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

pub(in crate::parse) fn inherit_vars(properties: &[(String, String)], parent: &Vars) -> Vars {
    let mut vars = parent.clone();
    for (key, value) in properties {
        if key.starts_with("--") {
            vars.insert(key.clone(), value.clone());
        }
    }
    vars
}
