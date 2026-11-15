//! `data-hf-transition`: scene transitions compiled into ordinary motion tracks.
//!
//! The incoming clip declares `data-hf-transition="<kind>"` and `data-hf-transition-from="<outgoing id>"`
//! (optional `data-hf-transition-easing`, default `ease-in-out-cubic`). The transition runs over the
//! overlap of the two clips, `incoming.start .. outgoing.end`, so start the incoming clip early, for
//! example `data-start="a-0.6"`. The incoming clip must draw above the outgoing one (higher
//! `data-track-index`, or the same track and later in the document).
//!
//! Kinds: `crossfade`; `push-{left,right,up,down}` (both scenes slide); `cover-{left,right,up,down}`
//! (the incoming scene slides over a static one); `zoom-in` (outgoing grows, incoming settles from 80%);
//! `zoom-out` (outgoing shrinks, incoming settles from 120%). A clip can take part in two transitions
//! (entering and leaving) as long as their windows do not overlap.
use crate::model::{Easing, Element, Motion, Track, TrackPoint};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::html::attr;
use anyhow::{anyhow, bail, Result};
use kuchiki::NodeRef;

/// Longest allowed overlap; a longer one almost always means the outgoing clip lacks `data-duration`.
const MAX_WINDOW: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Dir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Crossfade,
    Push(Dir),
    Cover(Dir),
    ZoomIn,
    ZoomOut,
}

#[derive(Clone, Debug)]
pub(in crate::parse) struct TransitionSpec {
    kind: Kind,
    /// Id of the outgoing clip.
    from: String,
    easing: Easing,
}

fn parse_dir(raw: &str) -> Option<Dir> {
    match raw {
        "left" => Some(Dir::Left),
        "right" => Some(Dir::Right),
        "up" => Some(Dir::Up),
        "down" => Some(Dir::Down),
        _ => None,
    }
}

fn parse_kind(raw: &str) -> Result<Kind> {
    let kind = match raw {
        "crossfade" => Some(Kind::Crossfade),
        "zoom-in" => Some(Kind::ZoomIn),
        "zoom-out" => Some(Kind::ZoomOut),
        _ => raw
            .strip_prefix("push-")
            .and_then(parse_dir)
            .map(Kind::Push)
            .or_else(|| {
                raw.strip_prefix("cover-")
                    .and_then(parse_dir)
                    .map(Kind::Cover)
            }),
    };
    kind.ok_or_else(|| {
        anyhow!("unknown data-hf-transition '{raw}'; use crossfade, push-<left|right|up|down>, cover-<left|right|up|down>, zoom-in, or zoom-out")
    })
}

/// Reads the transition attributes of the incoming clip; `None` when `data-hf-transition` is absent.
pub(in crate::parse) fn parse_transition(node: &NodeRef) -> Result<Option<TransitionSpec>> {
    let Some(raw) = attr(node, "data-hf-transition") else {
        if attr(node, "data-hf-transition-from").is_some()
            || attr(node, "data-hf-transition-easing").is_some()
        {
            bail!("data-hf-transition-from and data-hf-transition-easing require data-hf-transition");
        }
        return Ok(None);
    };
    let kind = parse_kind(raw.trim())?;
    let from = attr(node, "data-hf-transition-from")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("data-hf-transition requires data-hf-transition-from=\"<outgoing clip id>\""))?;
    let easing = match attr(node, "data-hf-transition-easing") {
        Some(raw) => parse_keyframe_easing(&raw)?,
        None => parse_keyframe_easing("ease-in-out-cubic")?,
    };
    Ok(Some(TransitionSpec { kind, from, easing }))
}

/// Tracks to add to one clip. Empty channels are left alone.
#[derive(Default)]
struct Channels {
    x: Track,
    y: Track,
    scale: Track,
    opacity: Track,
}

/// Two points: `from` at `at0` and `to` at `at1`, with `easing` shaping the segment.
fn segment(at0: f64, at1: f64, from: f32, to: f32, easing: Easing) -> Track {
    Track(vec![
        TrackPoint {
            at: at0,
            value: from,
            easing: Easing::Linear,
        },
        TrackPoint {
            at: at1,
            value: to,
            easing,
        },
    ])
}

/// Union of two tracks whose time ranges do not overlap; `None` when they do.
fn merge_track(existing: &Track, new: Track) -> Option<Track> {
    let (Some(new_first), Some(new_last)) = (new.0.first(), new.0.last()) else {
        return Some(existing.clone());
    };
    let (Some(old_first), Some(old_last)) = (existing.0.first(), existing.0.last()) else {
        return Some(new);
    };
    if new_first.at < old_last.at && old_first.at < new_last.at {
        return None;
    }
    let mut points = existing.0.clone();
    points.extend(new.0.iter().copied());
    points.sort_by(|a, b| a.at.total_cmp(&b.at));
    Some(Track(points))
}

fn merge_into(motion: &mut Motion, channels: Channels, label: &str) -> Result<()> {
    let overlap = || anyhow!("transition windows overlap on {label}; lengthen the clip or shorten the overlaps");
    motion.x = merge_track(&motion.x, channels.x).ok_or_else(overlap)?;
    motion.y = merge_track(&motion.y, channels.y).ok_or_else(overlap)?;
    motion.scale = merge_track(&motion.scale, channels.scale).ok_or_else(overlap)?;
    motion.opacity = merge_track(&motion.opacity, channels.opacity).ok_or_else(overlap)?;
    motion.hold_before = true;
    motion.hold_after = true;
    Ok(())
}

/// Offset of the incoming scene's entry point: it starts `dir`-ward of the frame and ends at rest.
fn entry_offset(dir: Dir, width: f32, height: f32) -> (f32, f32) {
    match dir {
        Dir::Left => (width, 0.0),
        Dir::Right => (-width, 0.0),
        Dir::Up => (0.0, height),
        Dir::Down => (0.0, -height),
    }
}

/// Compiles every transition. `transitions` holds `(incoming index, spec)`; `animated[i]` tells whether
/// clip `i` already has its own animation, which transitions cannot be combined with.
pub(in crate::parse) fn apply_transitions(
    elements: &mut [Element],
    transitions: &[(usize, TransitionSpec)],
    ids: &std::collections::HashMap<String, usize>,
    size: (u32, u32),
) -> Result<()> {
    if transitions.is_empty() {
        return Ok(());
    }
    let animated: Vec<bool> = elements.iter().map(|e| !e.motion.is_static()).collect();
    let (width, height) = (size.0 as f32, size.1 as f32);
    for (b, spec) in transitions {
        let b = *b;
        let label_b = elements[b].id.clone().unwrap_or_else(|| "element with no id".into());
        let a = *ids.get(&spec.from).ok_or_else(|| {
            anyhow!("data-hf-transition-from on {label_b} references unknown id: {}", spec.from)
        })?;
        if a == b || a == 0 || b == 0 {
            bail!("data-hf-transition on {label_b} must connect two different clips inside the root");
        }
        if animated[a] || animated[b] {
            bail!("transitions cannot be combined with CSS animation, data-hf-keyframes, or data-hf-camera on the same clip ({label_b} / {})", spec.from);
        }
        let (a_start, a_end) = (elements[a].start, elements[a].end);
        let (b_start, b_end) = (elements[b].start, elements[b].end);
        let window = a_end - b_start;
        if b_start < a_start || window <= 0.0 {
            bail!("transition {label_b}: the incoming clip must start inside the outgoing one (for example data-start=\"{}-0.6\")", spec.from);
        }
        if window > MAX_WINDOW {
            bail!("transition {label_b}: the overlap is {window:.2}s (limit {MAX_WINDOW}s); give the outgoing clip a data-duration");
        }
        if b_end - b_start < window - 1e-9 {
            bail!("transition {label_b}: the incoming clip is shorter than the overlap");
        }
        let out_at = b_start - a_start;
        let easing = spec.easing;
        let (a_base, b_base) = (elements[a].motion.base, elements[b].motion.base);
        let mut out = Channels::default();
        let mut inc = Channels::default();
        let enter = |channels: &mut Channels, dir: Dir| {
            let (dx, dy) = entry_offset(dir, width, height);
            if dx != 0.0 {
                channels.x = segment(0.0, window, b_base.x + dx, b_base.x, easing);
            }
            if dy != 0.0 {
                channels.y = segment(0.0, window, b_base.y + dy, b_base.y, easing);
            }
        };
        match spec.kind {
            Kind::Crossfade => {
                inc.opacity = segment(0.0, window, 0.0, b_base.opacity, easing);
            }
            Kind::Cover(dir) => enter(&mut inc, dir),
            Kind::Push(dir) => {
                enter(&mut inc, dir);
                let (dx, dy) = entry_offset(dir, width, height);
                if dx != 0.0 {
                    out.x = segment(out_at, out_at + window, a_base.x, a_base.x - dx, easing);
                }
                if dy != 0.0 {
                    out.y = segment(out_at, out_at + window, a_base.y, a_base.y - dy, easing);
                }
            }
            Kind::ZoomIn => {
                out.scale = segment(
                    out_at,
                    out_at + window,
                    a_base.scale,
                    a_base.scale * 1.3,
                    easing,
                );
                inc.scale = segment(0.0, window, b_base.scale * 0.8, b_base.scale, easing);
                inc.opacity = segment(0.0, window, 0.0, b_base.opacity, Easing::EaseOut);
            }
            Kind::ZoomOut => {
                out.scale = segment(
                    out_at,
                    out_at + window,
                    a_base.scale,
                    a_base.scale * 0.8,
                    easing,
                );
                inc.scale = segment(0.0, window, b_base.scale * 1.2, b_base.scale, easing);
                inc.opacity = segment(0.0, window, 0.0, b_base.opacity, Easing::EaseOut);
            }
        }
        let label_a = elements[a].id.clone().unwrap_or_else(|| spec.from.clone());
        merge_into(&mut elements[a].motion, out, &label_a)?;
        merge_into(&mut elements[b].motion, inc, &label_b)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kinds() {
        assert_eq!(parse_kind("crossfade").unwrap(), Kind::Crossfade);
        assert_eq!(parse_kind("push-left").unwrap(), Kind::Push(Dir::Left));
        assert_eq!(parse_kind("cover-down").unwrap(), Kind::Cover(Dir::Down));
        assert!(parse_kind("push-sideways").is_err());
        assert!(parse_kind("wipe").is_err());
    }

    #[test]
    fn merging_requires_disjoint_windows() {
        let first = segment(0.0, 1.0, 5.0, 0.0, Easing::Linear);
        let later = segment(3.0, 4.0, 0.0, -5.0, Easing::Linear);
        let merged = merge_track(&first, later).unwrap();
        assert_eq!(merged.0.len(), 4);
        assert!(merge_track(&first, segment(0.5, 2.0, 0.0, 1.0, Easing::Linear)).is_none());
        // Touching windows are fine: one scene ends its entry exactly when its exit starts.
        assert!(merge_track(&first, segment(1.0, 2.0, 0.0, 1.0, Easing::Linear)).is_some());
    }
}
