//! Validates local image-sequence patterns and indexes contiguous frames without decoding pixels.

use super::html::attr;
use crate::model::{ImageSequence, SequencePlayback};
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::{collections::BTreeMap, path::Path};

const MAX_FRAMES: usize = 4096;

pub(super) fn parse(node: &NodeRef, tag: &str, base: &Path) -> Result<Option<ImageSequence>> {
    let options = [
        "data-hf-sequence-fps",
        "data-hf-sequence-first",
        "data-hf-sequence-playback",
    ];
    let Some(pattern) = attr(node, "data-hf-sequence") else {
        if options.iter().any(|name| attr(node, name).is_some()) {
            bail!("sequence options require data-hf-sequence");
        }
        return Ok(None);
    };
    if tag != "img" || attr(node, "src").is_some() {
        bail!("data-hf-sequence replaces src on an <img>");
    }
    let fps = attr(node, options[0])
        .unwrap_or_else(|| "12".into())
        .trim()
        .parse::<f64>()
        .context("data-hf-sequence-fps must be a number")?;
    if !(0.01..=120.0).contains(&fps) {
        bail!("data-hf-sequence-fps must be between 0.01 and 120");
    }
    let first = attr(node, options[1])
        .unwrap_or_else(|| "0".into())
        .trim()
        .parse::<u32>()
        .context("data-hf-sequence-first must be a non-negative frame number")?;
    let playback = match attr(node, options[2]).as_deref() {
        None | Some("hold") => SequencePlayback::Hold,
        Some("loop") => SequencePlayback::Loop,
        Some("pingpong") => SequencePlayback::Pingpong,
        _ => bail!("data-hf-sequence-playback must be hold, loop, or pingpong"),
    };
    let path = Path::new(&pattern);
    if path.is_absolute()
        || path.components().any(|part| {
            !matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        bail!("data-hf-sequence requires a relative local path");
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("sequence pattern needs a filename"))?;
    let folder = path.parent().unwrap_or(Path::new(""));
    if folder.to_string_lossy().contains(['{', '}', ':', '\\']) {
        bail!("sequence placeholder belongs in the filename");
    }
    let (prefix, spec) = name
        .split_once("{frame")
        .ok_or_else(|| anyhow!("expected {{frame}} or {{frame:04}} in sequence filename"))?;
    let (format, suffix) = spec
        .split_once('}')
        .ok_or_else(|| anyhow!("unclosed sequence placeholder"))?;
    let width = match format {
        "" => 0,
        _ => format
            .strip_prefix(":0")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|w| (1..=10).contains(w))
            .ok_or_else(|| anyhow!("sequence padding must be :01 through :010"))?,
    };
    if prefix.contains(['{', '}', ':', '\\'])
        || suffix.contains(['{', '}', ':', '\\'])
        || !matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("png" | "webp")
        )
    {
        bail!("sequence filename must contain one placeholder and end in .png or .webp");
    }
    let folder = base.join(folder);
    super::reads::note(&folder);
    let mut frames = BTreeMap::new();
    for entry in std::fs::read_dir(&folder)
        .with_context(|| format!("cannot list sequence directory {}", folder.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let Some(digits) = name
            .to_str()
            .and_then(|n| n.strip_prefix(prefix))
            .and_then(|n| n.strip_suffix(suffix))
        else {
            continue;
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let number: u32 = digits
            .parse()
            .context("sequence frame number exceeds u32")?;
        if digits != format!("{number:0width$}") {
            bail!("invalid sequence padding for {}", entry.path().display());
        }
        if number >= first {
            if frames.len() == MAX_FRAMES {
                bail!("image sequences support at most {MAX_FRAMES} frames");
            }
            let file = entry.path();
            if !file.is_file() {
                bail!("sequence frame is not a file: {}", file.display());
            }
            frames.insert(number, file);
        }
    }
    if frames.is_empty() {
        bail!("no sequence frames match {pattern}");
    }
    let mut files = Vec::with_capacity(frames.len());
    for (offset, (number, file)) in frames.into_iter().enumerate() {
        let expected = u64::from(first) + offset as u64;
        if u64::from(number) != expected {
            bail!("missing sequence frame {expected} in {pattern}");
        }
        let format = image::ImageFormat::from_path(&file)?;
        let reader = image::ImageReader::open(&file)?.with_guessed_format()?;
        if reader.format() != Some(format) {
            bail!(
                "sequence frame format does not match its extension: {}",
                file.display()
            );
        }
        crate::render::check_image(&file)?;
        files.push(file);
    }
    Ok(Some(ImageSequence {
        files: files.into(),
        fps,
        playback,
    }))
}
