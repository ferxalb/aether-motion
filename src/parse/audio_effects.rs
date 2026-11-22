//! Audio effect attributes shared by `<audio>` and the sound of `<video>`:
//! `data-hf-volume-ramp` (JSON keyframes of `at` plus `volume` or `db`), `data-hf-eq` (a preset
//! name or a JSON band list), and `data-hf-duck` (comma list of trigger clip ids) with its
//! `-amount`, `-threshold`, `-attack`, `-hold`, `-release`, `-detector` (`peak` or `rms`), and
//! `-window` (the RMS window) options. Trigger ids are resolved
//! to clip indices by `resolve_ducking` once every clip is known.
use crate::audio::effects::automation::{VolumeCurve, VolumeKey, MAX_GAIN};
use crate::audio::effects::duck::{self, Duck};
use crate::audio::effects::eq::{self, Band, BandKind, Equalizer};
use crate::audio::effects::AudioEffects;
use crate::model::{AudioClip, Easing};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::html::attr;
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

const DUCK_OPTIONS: [&str; 7] = [
    "data-hf-duck-amount",
    "data-hf-duck-threshold",
    "data-hf-duck-attack",
    "data-hf-duck-hold",
    "data-hf-duck-release",
    "data-hf-duck-detector",
    "data-hf-duck-window",
];

pub(in crate::parse) fn parse_audio_effects(node: &NodeRef) -> Result<AudioEffects> {
    let volume = attr(node, "data-hf-volume-ramp")
        .map(|raw| parse_volume_ramp(&raw).context("data-hf-volume-ramp"))
        .transpose()?
        .map(Arc::new);
    let eq = attr(node, "data-hf-eq")
        .map(|raw| parse_eq(&raw).context("data-hf-eq"))
        .transpose()?
        .map(Arc::new);
    let duck = parse_duck(node)?;
    Ok(AudioEffects { volume, eq, duck })
}

/// A signed decimal with an optional `dB` suffix.
fn signed(raw: &str, name: &str) -> Result<f64> {
    let trimmed = raw.trim();
    let number = trimmed
        .strip_suffix("dB")
        .or_else(|| trimmed.strip_suffix("db"))
        .unwrap_or(trimmed)
        .trim();
    number
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| anyhow!("{name}: invalid number: {raw}"))
}

fn ranged(node: &NodeRef, name: &str, default: f64, min: f64, max: f64) -> Result<f64> {
    let Some(raw) = attr(node, name) else {
        return Ok(default);
    };
    let value = signed(&raw, name)?;
    if !(min..=max).contains(&value) {
        bail!("{name} must be between {min} and {max}: {raw}");
    }
    Ok(value)
}

fn parse_duck(node: &NodeRef) -> Result<Option<Duck>> {
    let Some(raw) = attr(node, "data-hf-duck") else {
        if let Some(option) = DUCK_OPTIONS.iter().find(|o| attr(node, o).is_some()) {
            bail!("{option} needs data-hf-duck=\"<trigger clip id>\"");
        }
        return Ok(None);
    };
    let by: Vec<String> = raw
        .split(',')
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if by.is_empty() {
        bail!(
            "data-hf-duck needs one or more trigger clip ids, for example data-hf-duck=\"voice\""
        );
    }
    Ok(Some(Duck {
        by,
        sources: Vec::new(),
        amount_db: ranged(
            node,
            "data-hf-duck-amount",
            duck::DEFAULT_AMOUNT_DB,
            0.0,
            duck::MAX_AMOUNT_DB,
        )?,
        threshold_db: ranged(
            node,
            "data-hf-duck-threshold",
            duck::DEFAULT_THRESHOLD_DB,
            duck::MIN_THRESHOLD_DB,
            0.0,
        )?,
        attack: ranged(
            node,
            "data-hf-duck-attack",
            duck::DEFAULT_ATTACK,
            0.0,
            duck::MAX_TIME,
        )?,
        hold: ranged(
            node,
            "data-hf-duck-hold",
            duck::DEFAULT_HOLD,
            0.0,
            duck::MAX_TIME,
        )?,
        release: ranged(
            node,
            "data-hf-duck-release",
            duck::DEFAULT_RELEASE,
            0.0,
            duck::MAX_TIME,
        )?,
        detector: match attr(node, "data-hf-duck-detector")
            .as_deref()
            .map(str::trim)
        {
            None | Some("peak") => {
                if attr(node, "data-hf-duck-window").is_some() {
                    bail!("data-hf-duck-window needs data-hf-duck-detector=\"rms\"");
                }
                duck::Detector::Peak
            }
            Some("rms") => duck::Detector::Rms {
                window: ranged(
                    node,
                    "data-hf-duck-window",
                    duck::DEFAULT_WINDOW,
                    duck::MIN_WINDOW,
                    duck::MAX_WINDOW,
                )?,
            },
            Some(other) => bail!("data-hf-duck-detector must be peak or rms: {other}"),
        },
    }))
}

/// Resolves every `data-hf-duck` id to the index of a clip with sound, and rejects duplicate clip
/// ids, self-ducking, and cycles.
pub(in crate::parse) fn resolve_ducking(audio: &mut [AudioClip]) -> Result<()> {
    let mut ids: HashMap<String, usize> = HashMap::new();
    for (index, clip) in audio.iter().enumerate() {
        if let Some(id) = &clip.id {
            if ids.insert(id.clone(), index).is_some() {
                bail!("duplicate id: {id}");
            }
        }
    }
    for (index, clip) in audio.iter_mut().enumerate() {
        let label = clip.id.as_deref().unwrap_or("no id").to_string();
        let Some(duck) = &mut clip.effects.duck else {
            continue;
        };
        let mut sources = Vec::with_capacity(duck.by.len());
        for id in &duck.by {
            let source = *ids.get(id).ok_or_else(|| {
                anyhow!("<audio> ({label}): data-hf-duck references {id}, which is not an <audio> or unmuted <video> with sound")
            })?;
            if source == index {
                bail!("<audio> ({label}): data-hf-duck cannot reference the clip itself");
            }
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        duck.sources = sources;
    }
    let triggers: Vec<&[usize]> = audio
        .iter()
        .map(|c| c.effects.duck.as_ref().map_or(&[][..], |d| &d.sources[..]))
        .collect();
    if let Err(i) = duck::processing_order(&triggers) {
        bail!(
            "<audio> ({}): data-hf-duck forms a cycle; a clip cannot duck under a clip that ducks under it",
            audio[i].id.as_deref().unwrap_or("no id")
        );
    }
    Ok(())
}

fn parse_volume_ramp(raw: &str) -> Result<VolumeCurve> {
    let frames: Vec<BTreeMap<String, serde_json::Value>> = serde_json::from_str(raw).context(
        "expected a JSON array of objects like [{\"at\":0,\"volume\":1},{\"at\":2,\"db\":-12,\"easing\":\"ease-in-out\"}]",
    )?;
    let mut previous = 1.0;
    let mut keys = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        let key = (|| -> Result<VolumeKey> {
            if let Some(key) = frame
                .keys()
                .find(|k| !matches!(k.as_str(), "at" | "volume" | "db" | "easing"))
            {
                bail!("unknown field: {key}");
            }
            let at = frame
                .get("at")
                .ok_or_else(|| anyhow!("at is required"))?
                .as_f64()
                .ok_or_else(|| anyhow!("at must be a number"))?;
            let number = |name: &str| -> Result<Option<f64>> {
                frame
                    .get(name)
                    .map(|v| v.as_f64().ok_or_else(|| anyhow!("{name} must be a number")))
                    .transpose()
            };
            let gain = match (number("volume")?, number("db")?) {
                (Some(_), Some(_)) => bail!("use volume or db, not both"),
                (Some(volume), None) => volume,
                (None, Some(db)) => {
                    let gain = 10f64.powf(db / 20.0);
                    if gain > MAX_GAIN * (1.0 + 1e-9) {
                        bail!("db must be at most +18");
                    }
                    gain.min(MAX_GAIN)
                }
                (None, None) => previous,
            };
            let easing = match frame.get("easing") {
                None => Easing::Linear,
                Some(value) => parse_keyframe_easing(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("easing must be a string"))?,
                )?,
            };
            Ok(VolumeKey { at, gain, easing })
        })()
        .with_context(|| format!("keyframe {index}"))?;
        previous = key.gain;
        keys.push(key);
    }
    VolumeCurve::new(keys)
}

fn parse_eq(raw: &str) -> Result<Equalizer> {
    let raw = raw.trim();
    if !raw.starts_with('[') {
        return Equalizer::preset(raw).ok_or_else(|| {
            anyhow!(
                "unknown preset {raw}; use {} or a JSON band list",
                eq::PRESETS.join(", ")
            )
        });
    }
    let frames: Vec<BTreeMap<String, serde_json::Value>> = serde_json::from_str(raw).context(
        "expected a JSON array of bands like [{\"type\":\"highpass\",\"freq\":80},{\"type\":\"peak\",\"freq\":3000,\"gain\":3,\"q\":1}]",
    )?;
    let mut bands = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        let band = (|| -> Result<Band> {
            if let Some(key) = frame
                .keys()
                .find(|k| !matches!(k.as_str(), "type" | "freq" | "gain" | "q"))
            {
                bail!("unknown field: {key}");
            }
            let name = frame
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("type is required: highpass, lowpass, bandpass, notch, peak, lowshelf, or highshelf"))?;
            let kind = BandKind::parse(name).ok_or_else(|| {
                anyhow!("unknown type {name}; use highpass, lowpass, bandpass, notch, peak, lowshelf, or highshelf")
            })?;
            let number = |name: &str| -> Result<Option<f64>> {
                frame
                    .get(name)
                    .map(|v| v.as_f64().ok_or_else(|| anyhow!("{name} must be a number")))
                    .transpose()
            };
            let freq = number("freq")?.ok_or_else(|| anyhow!("freq is required (Hz)"))?;
            let gain = number("gain")?;
            let gain = match (kind.has_gain(), gain) {
                (true, Some(gain)) => gain,
                (true, None) => bail!("{name} needs gain (dB)"),
                (false, Some(_)) => bail!("{name} takes no gain"),
                (false, None) => 0.0,
            };
            let q = number("q")?.unwrap_or(kind.default_q());
            Band::new(kind, freq, gain, q)
        })()
        .with_context(|| format!("band {index}"))?;
        bands.push(band);
    }
    Equalizer::new(bands, None)
}
