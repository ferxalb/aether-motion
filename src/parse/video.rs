//! `<video>` parsing: a local H.264 MP4 becomes `Content::Video`. The container is indexed here
//! (no decoding), which gives the source length used as the default clip duration.
//!
//! Attributes: `src` (or one `<source src>` child; `poster`, `autoplay`, `controls`, `preload`, and
//! `playsinline` are accepted and have no effect, since a render always plays from the start), `data-media-start` (seconds into the source, default 0), `loop` or
//! `data-hf-loop` (repeat the source; `data-duration` is then required), `muted`, `data-volume`
//! (0 to 8), `data-hf-fade-in`/`data-hf-fade-out`, and `data-hf-speed` or `data-hf-speed-ramp`
//! (see `parse/speed.rs`). Unless `muted`, the source's audio track becomes an `AudioClip` that
//! plays with the clip, trimmed by `data-media-start`, at the same speed as the picture, and loops
//! over the whole source like the picture.
use crate::model::{AudioClip, VideoSource};
use crate::parse::html::{attr, number};
use crate::parse::speed::parse_speed;
use crate::video::VideoIndex;
use anyhow::{anyhow, bail, Result};
use kuchiki::NodeRef;
use std::path::Path;
use std::sync::Arc;

/// Largest accepted `data-volume` (linear gain), as for `<audio>`.
const MAX_VOLUME: f64 = 8.0;

pub(in crate::parse) struct ParsedVideo {
    pub(in crate::parse) source: VideoSource,
    /// Clip time that plays the source left after `data-media-start` at the clip's speed: the clip
    /// length when `data-duration` is absent.
    pub(in crate::parse) default_duration: f64,
    /// The source's own sound, when it has an audio track and the element is not `muted`. Its
    /// `start` and `duration` are filled in from the clip.
    pub(in crate::parse) audio: Option<AudioClip>,
}

fn flag(node: &NodeRef, name: &str) -> Result<bool> {
    match attr(node, name).as_deref() {
        None | Some("false") => Ok(false),
        Some("" | "true") => Ok(true),
        Some(other) => bail!("{name} must be true or false: {other}"),
    }
}

pub(in crate::parse) fn parse_video(
    node: &NodeRef,
    base: &Path,
    has_duration: bool,
) -> Result<ParsedVideo> {
    let mut sources = Vec::new();
    for child in node.children() {
        if let Some(element) = child.as_element() {
            if &*element.name.local != "source" {
                bail!("<video> can only contain <source> elements");
            }
            sources.push(child.clone());
        }
    }
    let src = match attr(node, "src") {
        Some(src) if sources.is_empty() => src,
        Some(_) => bail!("use either src or <source> children on a <video>, not both"),
        // The first <source> that can be an MP4 (no `type`, or an MP4/QuickTime type).
        None => sources
            .iter()
            .filter(|s| {
                attr(s, "type").is_none_or(|t| t.contains("mp4") || t.contains("quicktime"))
            })
            .find_map(|s| attr(s, "src"))
            .ok_or_else(|| {
                anyhow!("<video> requires src, or a <source src> that is an MP4 file")
            })?,
    };
    if src.starts_with("data:") || src.contains("://") {
        bail!("only local video files are supported: {src}");
    }
    let path = base.join(src);
    let index = VideoIndex::load(&path)?;
    let media_start = attr(node, "data-media-start")
        .map(|raw| number(&raw, "data-media-start"))
        .transpose()?
        .unwrap_or(0.0);
    if media_start < 0.0 || media_start >= index.duration {
        bail!(
            "data-media-start must be at least 0 and before the end of the source ({:.3} s)",
            index.duration
        );
    }
    let looped = flag(node, "loop")? || flag(node, "data-hf-loop")?;
    if looped && !has_duration {
        bail!("a looping <video> requires data-duration");
    }
    let speed = parse_speed(node)?;
    let remaining = index.duration - media_start;
    let default_duration = match &speed {
        None => remaining,
        Some(curve) => match curve.time_for(remaining) {
            Some(time) => time,
            None if has_duration => 0.0,
            None => {
                bail!("this speed ramp ends at speed 0 before the source ends; set data-duration")
            }
        },
    };
    let audio = if index.has_audio && !flag(node, "muted")? {
        let seconds = |name: &str| -> Result<f64> {
            Ok(attr(node, name)
                .map(|raw| number(&raw, name))
                .transpose()?
                .unwrap_or(0.0))
        };
        let volume = attr(node, "data-volume")
            .map(|raw| number(&raw, "data-volume"))
            .transpose()?
            .unwrap_or(1.0);
        if !(0.0..=MAX_VOLUME).contains(&volume) {
            bail!("data-volume must be between 0 and {MAX_VOLUME}");
        }
        Some(AudioClip {
            id: attr(node, "id"),
            src: path.clone(),
            start: 0.0,
            duration: None,
            trim: media_start,
            volume: volume as f32,
            fade_in: seconds("data-hf-fade-in")?,
            fade_out: seconds("data-hf-fade-out")?,
            looped,
            loop_from: 0.0,
            speed: speed.clone(),
        })
    } else {
        None
    };
    Ok(ParsedVideo {
        default_duration,
        source: VideoSource {
            path,
            media_start,
            looped,
            audio: audio.is_some(),
            speed,
            index: Arc::new(index),
        },
        audio,
    })
}
