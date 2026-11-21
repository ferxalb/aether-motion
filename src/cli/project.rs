//! Project configuration (`aether.json`), entry-file resolution, and the mode rule shared by
//! `render`, `check`, and `preview` ([`select`]): a named file runs standalone with built-in
//! defaults and never reads `aether.json`; no file means the project.

use crate::encode::Quality;
use crate::parse::VariableValues;
use crate::render::OutputSize;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG: &str = "aether.json";

/// Settings that the short commands fall back to when a flag is not given.
#[derive(Debug)]
pub struct Project {
    pub entry: PathBuf,
    pub fps: u32,
    pub size: Option<OutputSize>,
    /// MP4 quality preset (`"quality"`, default `standard`).
    pub quality: Quality,
    pub out_dir: PathBuf,
    /// Values for the composition's declared variables (`"variables"`, an object keyed by id).
    pub variables: VariableValues,
}

/// The settings of a command run: [`standalone`] for a named file, else the project in the
/// current directory.
pub fn select(file: Option<&Path>) -> Result<Project> {
    match file {
        Some(file) => standalone(file),
        None => resolve(Path::new("."), None),
    }
}

/// `file` with the built-in defaults (30 fps, the composition's own size, quality `standard`,
/// output folder `out`, no variable values). `aether.json` is not read.
pub fn standalone(file: &Path) -> Result<Project> {
    if !file.is_file() {
        bail!("composition not found: {}", file.display());
    }
    Ok(Project {
        entry: file.to_path_buf(),
        fps: 30,
        size: None,
        quality: Quality::default(),
        out_dir: PathBuf::from("out"),
        variables: VariableValues::new(),
    })
}

/// Resolves the composition file and settings relative to `dir`.
///
/// Entry order: explicit `file`, `entry` in `aether.json`, `index.html`, the only `*.html` in `dir`.
pub fn resolve(dir: &Path, file: Option<&str>) -> Result<Project> {
    let config: Value = match fs::read_to_string(dir.join(CONFIG)) {
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("{CONFIG} is not valid JSON"))?
        }
        Err(_) => Value::Null,
    };
    let entry = match file.or_else(|| config["entry"].as_str()) {
        Some(f) => dir.join(f),
        None => default_entry(dir)?,
    };
    if !entry.is_file() {
        bail!("composition not found: {}", entry.display());
    }
    let fps = config["fps"].as_u64().map_or(Ok(30), |f| {
        u32::try_from(f)
            .ok()
            .filter(|f| (1..=60).contains(f))
            .ok_or_else(|| anyhow!("{CONFIG}: fps must be 1 to 60"))
    })?;
    let size = config["size"]
        .as_str()
        .map(str::parse)
        .transpose()
        .context("aether.json: size")?;
    let quality = config["quality"]
        .as_str()
        .map(str::parse)
        .transpose()
        .context("aether.json: quality")?
        .unwrap_or_default();
    let out_dir = dir.join(config["out"].as_str().unwrap_or("out"));
    let variables = match &config["variables"] {
        Value::Null => VariableValues::new(),
        Value::Object(values) => values.clone(),
        _ => bail!("{CONFIG}: variables must be an object keyed by variable id"),
    };
    Ok(Project {
        entry,
        fps,
        size,
        quality,
        out_dir,
        variables,
    })
}

fn default_entry(dir: &Path) -> Result<PathBuf> {
    let index = dir.join("index.html");
    if index.is_file() {
        return Ok(index);
    }
    let mut html: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "html"))
        .collect();
    match html.len() {
        1 => Ok(html.remove(0)),
        0 => bail!("no composition found; run `aether-motion init` or pass a .html file"),
        _ => bail!("several .html files here; pass one, or set \"entry\" in {CONFIG}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_config_then_index_then_single_html() {
        let dir = std::env::temp_dir().join(format!("aether-project-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(resolve(&dir, None).is_err());
        fs::write(dir.join("only.html"), "").unwrap();
        assert!(resolve(&dir, None).unwrap().entry.ends_with("only.html"));
        fs::write(dir.join("a.html"), "").unwrap();
        assert!(resolve(&dir, None).is_err());
        fs::write(dir.join("index.html"), "").unwrap();
        assert!(resolve(&dir, None).unwrap().entry.ends_with("index.html"));
        fs::write(
            dir.join(CONFIG),
            r#"{"entry":"a.html","fps":60,"size":"4k","out":"dist","variables":{"title":"x"}}"#,
        )
        .unwrap();
        let p = resolve(&dir, None).unwrap();
        assert!(
            p.entry.ends_with("a.html")
                && p.fps == 60
                && p.size.is_some()
                && p.out_dir.ends_with("dist")
                && p.variables["title"] == "x"
        );
        fs::write(dir.join(CONFIG), r#"{"variables":[1]}"#).unwrap();
        assert!(resolve(&dir, None).is_err());
        // A named file never reads the (now broken) configuration.
        let alone = standalone(&dir.join("a.html")).unwrap();
        assert!(alone.fps == 30 && alone.size.is_none() && alone.variables.is_empty());
        assert!(standalone(&dir.join("missing.html")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
