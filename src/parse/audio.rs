//! `<audio>` parsing: turns an element into a pending `AudioClip`. Audio is not part of the
//! element tree (no layout, no painting); its start is resolved with the other clips' timeline.
//! `data-hf-speed` and `data-hf-speed-ramp` are read by `parse/speed.rs`, and the effect
//! attributes (`data-hf-volume-ramp`, `data-hf-eq`, `data-hf-duck`) by `parse/audio_effects.rs`.
use crate::model::AudioClip;
use crate::parse::audio_effects::parse_audio_effects;
use crate::parse::html::{attr, number};
use crate::parse::speed::parse_speed;
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::path::Path;

/// Largest accepted `data-volume` (linear gain, about +18 dB).
const MAX_VOLUME: f64 = 8.0;

#[derive(Debug)]
pub(in crate::parse) struct PendingAudio {
    pub(in crate::parse) clip: AudioClip,
    pub(in crate::parse) start_expr: Option<String>,
    /// Index of the enclosing element in the pending list, which supplies the default start.
    pub(in crate::parse) parent: Option<usize>,
}

pub(in crate::parse) fn parse_audio(
    node: &NodeRef,
    parent: Option<usize>,
    base: &Path,
) -> Result<PendingAudio> {
    let id = attr(node, "id");
    let label = id.as_deref().unwrap_or("no id").to_string();
    let parse = || -> Result<PendingAudio> {
        let src = attr(node, "src").ok_or_else(|| anyhow!("<audio> requires src"))?;
        if src.starts_with("data:") || src.contains("://") {
            bail!("only local audio files are supported: {src}");
        }
        let seconds = |name: &str| -> Result<Option<f64>> {
            attr(node, name).map(|raw| number(&raw, name)).transpose()
        };
        let duration = seconds("data-duration")?;
        if duration.is_some_and(|d| d <= 0.0) {
            bail!("data-duration must be greater than 0");
        }
        let volume = seconds("data-volume")?.unwrap_or(1.0);
        if volume > MAX_VOLUME {
            bail!("data-volume must be between 0 and {MAX_VOLUME}");
        }
        let looped = match attr(node, "data-hf-loop").as_deref() {
            None | Some("false") => false,
            Some("" | "true") => true,
            Some(other) => bail!("data-hf-loop must be true or false: {other}"),
        };
        if looped && duration.is_none() {
            bail!("data-hf-loop requires data-duration");
        }
        let fade_in = seconds("data-hf-fade-in")?.unwrap_or(0.0);
        let fade_out = seconds("data-hf-fade-out")?.unwrap_or(0.0);
        if duration.is_some_and(|d| fade_in + fade_out > d + 1e-9) {
            bail!("fade-in + fade-out exceeds the clip duration");
        }
        super::reads::note(&base.join(&src));
        Ok(PendingAudio {
            clip: AudioClip {
                id: id.clone(),
                src: base.join(src),
                start: 0.0,
                duration,
                scene_end: None,
                trim: seconds("data-hf-trim-start")?.unwrap_or(0.0),
                volume: volume as f32,
                fade_in,
                fade_out,
                looped,
                loop_from: seconds("data-hf-trim-start")?.unwrap_or(0.0),
                speed: parse_speed(node)?,
                effects: parse_audio_effects(node)?,
            },
            start_expr: attr(node, "data-start"),
            parent,
        })
    };
    parse().with_context(|| format!("<audio> ({label})"))
}
