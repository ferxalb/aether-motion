//! Minimal argument parser shared by the subcommands, and the [`UsageError`] they report.
//!
//! A usage error (an unknown option, a missing or malformed value) exits with code 2 and points at
//! the command's help; other errors exit with code 1.

use super::suggest;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

/// A mistake in the command line rather than in the composition or the environment.
#[derive(Debug)]
pub struct UsageError {
    pub message: String,
    /// Extra lines printed under the error, such as a suggestion.
    pub hint: Option<String>,
}

impl UsageError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UsageError {}

/// A usage error as an `anyhow::Error`.
pub fn usage(message: impl Into<String>) -> anyhow::Error {
    UsageError::new(message).into()
}

/// `unknown option: <arg>`, with the closest known option as a hint.
pub fn unknown_option<'a>(arg: &str, known: impl IntoIterator<Item = &'a str>) -> anyhow::Error {
    let error = UsageError::new(format!("unknown option: {arg}"));
    match suggest::closest(arg, known) {
        Some(near) => error.with_hint(format!("did you mean {near}?")).into(),
        None => error.into(),
    }
}

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
                    .ok_or_else(|| usage(format!("missing value for {arg}")))?;
                args.values.insert(name.to_string(), value.clone());
            } else if switches.contains(&name) {
                args.switches.insert(name.to_string());
            } else if arg.starts_with('-') && arg.len() > 1 {
                return Err(unknown_option(arg, values.iter().chain(switches).copied()));
            } else {
                args.positional.push(arg.clone());
            }
        }
        Ok(args)
    }

    /// The optional composition file of `command`; more than one positional is a usage error.
    pub fn file(&self, command: &str) -> Result<Option<&Path>> {
        match self.positional.as_slice() {
            [] => Ok(None),
            [file] => Ok(Some(Path::new(file))),
            many => Err(usage(format!(
                "{command} takes one composition, got {}",
                many.join(" ")
            ))),
        }
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
            .map(|v| v.parse().map_err(|_| usage(format!("invalid {name}: {v}"))))
            .transpose()
    }

    /// Parses a flag whose `FromStr` names the accepted values (presets, modes), as a usage error.
    pub fn choice<T: std::str::FromStr<Err = anyhow::Error>>(
        &self,
        name: &str,
    ) -> Result<Option<T>> {
        self.value(name)
            .map(|v| {
                v.parse()
                    .map_err(|e: anyhow::Error| usage(format!("{e:#}")))
            })
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
        let error = Args::parse(&s(&["--ouptut", "x"]), &["--output"], &[]).unwrap_err();
        let usage = error.downcast_ref::<UsageError>().unwrap();
        assert_eq!(usage.hint.as_deref(), Some("did you mean --output?"));
    }
}
