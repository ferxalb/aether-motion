//! `--variables '<json>'` and `--variables-file <path>`: values for the composition's declared
//! variables, layered key by key over the project's `aether.json` `"variables"` object.

use crate::parse::{read_values_file, values_from_json, VariableValues};
use anyhow::{bail, Result};
use std::path::Path;

/// `base` (the project's values) with the values passed on the command line on top.
pub fn values(
    inline: Option<&str>,
    file: Option<&str>,
    base: &VariableValues,
) -> Result<VariableValues> {
    let passed = match (inline, file) {
        (Some(_), Some(_)) => bail!("choose either --variables or --variables-file"),
        (Some(text), None) => values_from_json(text)?,
        (None, Some(path)) => read_values_file(Path::new(path))?,
        (None, None) => VariableValues::new(),
    };
    let mut merged = base.clone();
    merged.extend(passed);
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_override_project_values_per_key() {
        let base = values_from_json(r#"{"a": 1, "b": "x"}"#).unwrap();
        let merged = values(Some(r#"{"b": "y"}"#), None, &base).unwrap();
        assert_eq!(merged["a"], 1);
        assert_eq!(merged["b"], "y");
        assert!(values(Some("{}"), Some("v.json"), &base).is_err());
        assert!(values(Some("[1]"), None, &base).is_err());
    }
}
