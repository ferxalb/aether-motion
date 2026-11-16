//! Minimal argument parser shared by the human-facing subcommands.

use anyhow::{anyhow, bail, Result};
use std::collections::{HashMap, HashSet};

/// Parsed arguments: positionals, `--flag value` pairs, and `--switch` booleans.
#[derive(Debug, Default)]
pub struct Args {
    pub positional: Vec<String>,
    values: HashMap<String, String>,
    switches: HashSet<String>,
}

/// Short aliases accepted everywhere.
fn canonical(arg: &str) -> &str {
    match arg {
        "-o" => "--output",
        "-y" => "--yes",
        "-f" => "--force",
        other => other,
    }
}

impl Args {
    /// Parses `raw`; flags must be listed in `values` (take an argument) or `switches` (do not).
    pub fn parse(raw: &[String], values: &[&str], switches: &[&str]) -> Result<Self> {
        let mut args = Args::default();
        let mut iter = raw.iter();
        while let Some(arg) = iter.next() {
            let name = canonical(arg);
            if values.contains(&name) {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow!("missing value for {arg}"))?;
                args.values.insert(name.to_string(), value.clone());
            } else if switches.contains(&name) {
                args.switches.insert(name.to_string());
            } else if arg.starts_with('-') && arg.len() > 1 {
                bail!("unknown option: {arg}");
            } else {
                args.positional.push(arg.clone());
            }
        }
        Ok(args)
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn has(&self, name: &str) -> bool {
        self.switches.contains(name)
    }

    /// Parses a numeric flag, naming it in the error.
    pub fn number<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>> {
        self.value(name)
            .map(|v| v.parse().map_err(|_| anyhow!("invalid {name}: {v}")))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_values_switches_and_aliases() {
        let a = Args::parse(
            &s(&["a.html", "-o", "x.mp4", "-y"]),
            &["--output"],
            &["--yes"],
        )
        .unwrap();
        assert_eq!(a.positional, ["a.html"]);
        assert_eq!(a.value("--output"), Some("x.mp4"));
        assert!(a.has("--yes"));
        assert!(Args::parse(&s(&["--nope"]), &[], &[]).is_err());
        assert!(Args::parse(&s(&["--output"]), &["--output"], &[]).is_err());
    }
}
