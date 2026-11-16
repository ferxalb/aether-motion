//! Project configuration (`aether.json`) and entry-file resolution for the short commands.

use crate::encode::Quality;
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
    /// MP4 quality preset (`"quality"`, default `max`).
    pub quality: Quality,
    pub out_dir: PathBuf,
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
    Ok(Project {
        entry,
        fps,
        size,
        quality,
        out_dir,
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
            r#"{"entry":"a.html","fps":60,"size":"4k","out":"dist"}"#,
        )
        .unwrap();
        let p = resolve(&dir, None).unwrap();
        assert!(
            p.entry.ends_with("a.html")
                && p.fps == 60
                && p.size.is_some()
                && p.out_dir.ends_with("dist")
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
