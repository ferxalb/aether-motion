//! `render --cloud`: the same render, run in an ephemeral Modal app, with the MP4 downloaded back.
//!
//! The flag is not in the help catalog. The composition and the files it reads are copied into a
//! temporary bundle (`Composition::dependencies`), and the remaining flags are forwarded to a
//! remote `aether-motion render`, so every flag keeps its meaning. `cloud.py` is the Modal app; it
//! streams the remote `--progress json` lines back, which this module shows with the usual
//! reporter. The CLI states that it renders remotely and how much it uploads. The container is always Linux x64,
//! so it runs the release build of this version (or the Linux binary named by `AETHER_CLOUD_BIN`),
//! never the local executable: the local OS can be macOS, Linux, or Windows. Fonts that are only installed on this machine
//! are not part of the bundle, so a composition must bring its own fonts to match a local render.

use super::super::args::usage;
use super::super::options::Options;
use super::super::progress::StderrProgress;
use crate::encode::{ProgressObserver, ProgressUpdate};
use crate::model::Composition;
use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

const DRIVER: &str = include_str!("cloud.py");

/// Flags that take a value and are rebuilt or dropped when forwarding.
const OWN_VALUES: &[&str] = &["--output", "-o", "--progress", "--variables-file"];

/// Rejects the combinations a remote MP4 render cannot honor, before anything is uploaded.
pub fn check(opts: &Options, is_mp4: bool) -> Result<()> {
    if opts.input.is_none() {
        return Err(usage("--cloud needs a composition file"));
    }
    if !is_mp4 || opts.segments.is_some() || opts.parts.is_some() {
        return Err(usage("--cloud renders whole MP4 files"));
    }
    if opts.variables_file.is_some() {
        return Err(usage("--cloud takes --variables, not --variables-file"));
    }
    Ok(())
}

/// The flags to run remotely: everything the user typed except `--cloud`, the output, the
/// progress mode, and the composition (renamed to its place in the bundle).
fn forwarded(raw: &[String], remote_entry: &str) -> Vec<String> {
    let mut out = vec!["render".to_string(), remote_entry.to_string()];
    let mut iter = raw.iter();
    let mut skipped_input = false;
    while let Some(arg) = iter.next() {
        if arg == "--cloud" || arg == "-f" || arg == "--force" {
            continue;
        }
        if OWN_VALUES.contains(&arg.as_str()) {
            iter.next();
        } else if arg.starts_with('-') && arg.len() > 1 {
            out.push(arg.clone());
            // Every other flag with a value is followed by it; switches are not.
            if super::super::options::takes_value(arg) {
                out.extend(iter.next().cloned());
            }
        } else if !skipped_input {
            skipped_input = true;
        } else {
            out.push(arg.clone());
        }
    }
    out
}

/// Copies `path` (a file or a folder) to `dest`.
fn copy(path: &Path, dest: &Path) -> Result<u64> {
    if path.is_dir() {
        let mut total = 0;
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            total += copy(&entry.path(), &dest.join(entry.file_name()))?;
        }
        return Ok(total);
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(fs::copy(path, dest)?)
}

/// A temporary bundle that is removed when dropped.
struct Bundle(PathBuf);

impl Drop for Bundle {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Fails fast, with the fix, when Modal is missing or not signed in. Without this the runner can
/// block on a login prompt nobody can see, which looks like a render stuck on "uploading".
fn preflight() -> Result<()> {
    let found = Command::new("modal")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !found {
        bail!("--cloud needs the Modal CLI: run `pip install modal`, then `modal setup`");
    }
    let env_token = std::env::var_os("MODAL_TOKEN_ID").is_some()
        && std::env::var_os("MODAL_TOKEN_SECRET").is_some();
    let config = std::env::var_os("MODAL_CONFIG_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|h| PathBuf::from(h).join(".modal.toml"))
        });
    if !env_token && !config.is_some_and(|c| c.exists()) {
        bail!("Modal is not signed in: run `modal setup` (or set MODAL_TOKEN_ID and MODAL_TOKEN_SECRET)");
    }
    Ok(())
}

/// Renders `entry` remotely into `dest`.
pub fn run(
    composition: &Composition,
    entry: &Path,
    raw: &[String],
    opts: &Options,
    dest: &Path,
) -> Result<()> {
    preflight()?;
    let root = fs::canonicalize(entry)?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let bundle = Bundle(std::env::temp_dir().join(format!("aether-cloud-{}", std::process::id())));
    fs::create_dir_all(&bundle.0)?;
    let (mut files, mut bytes) = (0, 0);
    for dependency in &composition.dependencies {
        let source = fs::canonicalize(dependency)
            .with_context(|| format!("could not read {}", dependency.display()))?;
        let relative = source.strip_prefix(&root).map_err(|_| {
            anyhow::anyhow!(
                "{} is outside the composition folder; move it next to the composition",
                dependency.display()
            )
        })?;
        bytes += copy(&source, &bundle.0.join(relative))?;
        files += 1;
    }
    let remote_entry = fs::canonicalize(entry)?
        .strip_prefix(&root)?
        .to_string_lossy()
        .replace('\\', "/");
    let args = forwarded(raw, &remote_entry);
    let script = std::env::temp_dir().join(format!("aether-cloud-{}.py", std::process::id()));
    fs::write(&script, DRIVER)?;
    let style = super::super::ui::Style::err();
    eprintln!(
        "{}",
        style.dim(&format!(
            "rendering remotely · bundling {files} files ({:.1} MB)",
            bytes as f64 / 1e6
        ))
    );
    let mut child = Command::new("modal")
        .args(["run"])
        .arg(&script)
        .args(["--args", &serde_json::to_string(&args)?, "--out"])
        .arg(dest)
        .env("AETHER_CLOUD_VERSION", env!("CARGO_PKG_VERSION"))
        .env("AETHER_CLOUD_BUNDLE", &bundle.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start the remote runner")?;
    let reporter = StderrProgress::new(opts.progress);
    let start = Instant::now();
    let mut tail = Vec::new();
    for line in BufReader::new(child.stderr.take().context("no runner output")?).lines() {
        let line = line?;
        match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(v) if v.get("done").is_some() => {
                if let Some(reporter) = &reporter {
                    reporter.update(ProgressUpdate {
                        done: v["done"].as_u64().unwrap_or(0) as usize,
                        total: v["total"].as_u64().unwrap_or(0) as usize,
                        elapsed: start.elapsed(),
                    });
                }
            }
            _ => {
                // Show runner stages live (image build, upload, container start) so a slow
                // first step is visible instead of looking frozen.
                if !line.trim().is_empty()
                    && !matches!(opts.progress, super::super::progress::ProgressMode::Off)
                {
                    eprintln!("{}", style.dim(line.trim_end()));
                }
                tail.push(line);
                if tail.len() > 20 {
                    tail.remove(0);
                }
            }
        }
    }
    let status = child.wait()?;
    let _ = fs::remove_file(&script);
    if let Some(reporter) = &reporter {
        reporter.finish();
    }
    if !status.success() || !dest.exists() {
        bail!("remote render failed:\n{}", tail.join("\n"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn forwards_every_flag_except_the_local_ones() {
        let raw = s(&[
            "a.html",
            "--cloud",
            "-o",
            "x.mp4",
            "-f",
            "--fps",
            "60",
            "--quality",
            "high",
            "--progress",
            "off",
        ]);
        assert_eq!(
            forwarded(&raw, "a.html"),
            s(&["render", "a.html", "--fps", "60", "--quality", "high"])
        );
    }
}
