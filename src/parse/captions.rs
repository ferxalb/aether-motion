//! `data-hf-captions`: timed captions from a transcript file. The element becomes a host for one
//! absolutely positioned row per cue; each row is visible only during its cue, and the word being
//! spoken is highlighted. Everything compiles to ordinary motion tracks, so frames stay a pure
//! function of time. Transcript times are seconds from the start of the captions element's clip.
//!
//! Attributes on an empty element: `data-hf-captions` (path to `.json`, `.srt`, or `.vtt`, see
//! `captions::transcript` for the accepted shapes; required);
//! - `-mode`: `highlight` (default; the spoken word switches to the highlight color), `reveal` (words
//!   appear as they are spoken, space reserved up front), or `plain` (whole cue, no word effect);
//! - `-highlight`: color of the spoken word (default `#ffd60a`); `-pop`: scale of the spoken word
//!   (1 to 2, default 1.08, 1 disables it);
//! - `-offset`: seconds added to every time (may be negative); `-linger`: seconds a cue stays after its
//!   last word, never past the next cue (0 to 5, default 0.3);
//! - grouping, ignored for SRT/VTT (which keep their cues) or with `-preserve-cues`: `-max-words`
//!   (1 to 30, default 8), `-max-chars` (1 to 200, default 40), `-pause` (seconds of silence that
//!   start a new cue, default 0.8);
//! - `-gap`: px between words (default 0.28 of the font size).
//!
//! A cue is one line: it does not wrap, so choose `-max-chars` and `font-size` to fit the width.
//! Give the element an explicit `left`, `top`, `width`, and `height`; rows sit at its top edge and
//! follow its `text-align`, `color`, and font.
use crate::captions::cues::{group, one_per_word, GroupOptions};
use crate::captions::transcript::load;
use crate::captions::{Cue, Word};
use crate::model::{Color, Content, Easing, Length, Motion, Style, Track, TrackPoint};
use crate::parse::counter::visibility;
use crate::parse::html::attr;
use crate::parse::parse_color;
use crate::parse::split::{push_piece, row_container_style};
use crate::parse::tree::Pending;
use anyhow::{bail, Context, Result};
use kuchiki::NodeRef;
use std::path::Path;

/// Most words one element expands (each costs about three generated elements).
const MAX_WORDS: usize = 1200;
/// Words closer than this (seconds) keep the highlight until the next word starts.
const HIGHLIGHT_BRIDGE: f64 = 0.3;
/// Seconds the spoken word takes to grow to its pop scale.
const POP_RISE: f64 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Highlight,
    Reveal,
    Plain,
}

#[derive(Clone, Debug)]
pub(in crate::parse) struct CaptionsSpec {
    tag: String,
    cues: Vec<Cue>,
    mode: Mode,
    highlight: Color,
    pop: f32,
    linger: f64,
    gap: f32,
}

const OPTIONS: [&str; 10] = [
    "data-hf-captions-mode",
    "data-hf-captions-highlight",
    "data-hf-captions-pop",
    "data-hf-captions-offset",
    "data-hf-captions-linger",
    "data-hf-captions-max-words",
    "data-hf-captions-max-chars",
    "data-hf-captions-pause",
    "data-hf-captions-preserve-cues",
    "data-hf-captions-gap",
];

/// Reads the captions attributes of `node`; `None` when `data-hf-captions` is absent.
pub(in crate::parse) fn parse_captions(
    node: &NodeRef,
    tag: &str,
    style: &Style,
    base: &Path,
) -> Result<Option<CaptionsSpec>> {
    let Some(src) = attr(node, "data-hf-captions") else {
        if let Some(name) = OPTIONS.iter().find(|name| attr(node, name).is_some()) {
            bail!("{name} requires data-hf-captions");
        }
        return Ok(None);
    };
    if src.is_empty() || src.starts_with("data:") || src.contains("://") {
        bail!("data-hf-captions needs a local transcript path: {src}");
    }
    let num = |name: &str, default: f64, range: (f64, f64)| -> Result<f64> {
        let value = match attr(node, name) {
            Some(raw) => raw
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .with_context(|| format!("{name}: invalid number: {raw}"))?,
            None => default,
        };
        if !(range.0..=range.1).contains(&value) {
            bail!("{name} must be between {} and {}", range.0, range.1);
        }
        Ok(value)
    };
    let mode = match attr(node, "data-hf-captions-mode").as_deref() {
        None | Some("highlight") => Mode::Highlight,
        Some("reveal") => Mode::Reveal,
        Some("plain") => Mode::Plain,
        Some(other) => bail!("data-hf-captions-mode must be highlight, reveal, or plain: {other}"),
    };
    let highlight = match attr(node, "data-hf-captions-highlight") {
        Some(raw) => parse_color(&raw)?,
        None => parse_color("#ffd60a")?,
    };
    let preserve = match attr(node, "data-hf-captions-preserve-cues").as_deref() {
        None | Some("false") => false,
        Some("" | "true") => true,
        Some(other) => bail!("data-hf-captions-preserve-cues must be true or false: {other}"),
    };
    let pop = num("data-hf-captions-pop", 1.08, (1.0, 2.0))?;
    let linger = num("data-hf-captions-linger", 0.3, (0.0, 5.0))?;
    let offset = num("data-hf-captions-offset", 0.0, (-86_400.0, 86_400.0))?;
    let options = GroupOptions {
        max_words: num("data-hf-captions-max-words", 8.0, (1.0, 30.0))?.round() as usize,
        max_chars: num("data-hf-captions-max-chars", 40.0, (1.0, 200.0))?.round() as usize,
        pause: num("data-hf-captions-pause", 0.8, (0.0, 10.0))?,
    };
    let size = style.font_size.unwrap_or(match tag {
        "h1" => 64.0,
        "h2" => 48.0,
        "h3" => 36.0,
        _ => 24.0,
    });
    let gap = num("data-hf-captions-gap", (size * 0.28) as f64, (0.0, 1000.0))?;

    super::reads::note(&base.join(&src));
    let transcript = load(&base.join(&src))?;
    let cues = match (&transcript.cues, preserve) {
        (_, true) => one_per_word(&transcript.words),
        (Some(cues), false) => cues.clone(),
        (None, false) => group(&transcript.words, options),
    };
    let cues = shift(cues, offset);
    let words: usize = cues.iter().map(|c| c.words.len()).sum();
    if words == 0 {
        bail!("{src} has no words after applying data-hf-captions-offset");
    }
    if words > MAX_WORDS {
        bail!(
            "{src} has {words} words; the limit per element is {MAX_WORDS}. Split the transcript and use one captions element per part"
        );
    }
    Ok(Some(CaptionsSpec {
        tag: tag.to_string(),
        cues,
        mode,
        highlight,
        pop: pop as f32,
        linger,
        gap: gap as f32,
    }))
}

/// Adds `offset` to every time; words that end before 0 are dropped and starts are clamped to 0.
fn shift(cues: Vec<Cue>, offset: f64) -> Vec<Cue> {
    cues.into_iter()
        .map(|cue| Cue {
            words: cue
                .words
                .into_iter()
                .map(|w| Word {
                    text: w.text,
                    start: (w.start + offset).max(0.0),
                    end: w.end + offset,
                })
                .filter(|w| w.end > 0.0 || w.start > 0.0)
                .collect(),
        })
        .filter(|cue| !cue.words.is_empty())
        .collect()
}

/// Smallest visible span, so zero-length words still show for a frame at any frame rate up to 1 kHz.
const MIN_SPAN: f64 = 1e-3;

impl CaptionsSpec {
    /// Appends one row per cue below `parent` (the already pushed host element).
    pub(in crate::parse) fn expand(
        &self,
        parent: usize,
        host: &Style,
        out: &mut Vec<Pending>,
    ) -> Result<()> {
        for (i, cue) in self.cues.iter().enumerate() {
            let start = cue.start();
            let mut hide = cue.end() + self.linger;
            if let Some(next) = self.cues.get(i + 1) {
                hide = hide.min(next.start());
            }
            let hide = hide.max(start + MIN_SPAN);
            let mut style = Style {
                position: Some("absolute".to_string()),
                left: Some(Length::Px(0.0)),
                top: Some(Length::Px(0.0)),
                width: Some(Length::Percent(100.0)),
                text_align: host.text_align.clone(),
                ..Style::default()
            };
            row_container_style(&mut style, self.gap);
            let motion = Motion {
                opacity: visibility(start, Some(hide)),
                hold_before: true,
                hold_after: true,
                ..Motion::default()
            };
            let row = push_piece(out, &self.tag, parent, Content::Container, style, motion);
            for (j, word) in cue.words.iter().enumerate() {
                let next = cue.words.get(j + 1).map(|n| n.start);
                self.push_word(out, row, word, next, hide);
            }
        }
        Ok(())
    }

    fn push_word(
        &self,
        out: &mut Vec<Pending>,
        row: usize,
        word: &Word,
        next: Option<f64>,
        hide: f64,
    ) {
        let text = || Content::Text(word.text.clone());
        let tag = &self.tag;
        match self.mode {
            Mode::Plain => {
                push_piece(out, tag, row, text(), Style::default(), Motion::default());
            }
            Mode::Reveal => {
                let motion = Motion {
                    opacity: visibility(word.start, None),
                    hold_before: true,
                    hold_after: true,
                    ..Motion::default()
                };
                push_piece(out, tag, row, text(), Style::default(), motion);
            }
            Mode::Highlight => {
                // The spoken word stays highlighted until the next one starts when they nearly touch.
                let mut end = word.end;
                if let Some(next) = next {
                    if next - word.end < HIGHLIGHT_BRIDGE {
                        end = next;
                    }
                }
                let end = end.min(hide).max(word.start + MIN_SPAN);
                // A flex holder takes the size of its in-flow text; the overlay is absolute.
                let mut holder_style = Style::default();
                row_container_style(&mut holder_style, 0.0);
                let holder = push_piece(
                    out,
                    tag,
                    row,
                    Content::Container,
                    holder_style,
                    self.pop_motion(word.start, end),
                );
                push_piece(
                    out,
                    tag,
                    holder,
                    text(),
                    Style::default(),
                    Motion::default(),
                );
                let overlay = Style {
                    position: Some("absolute".to_string()),
                    left: Some(Length::Px(0.0)),
                    top: Some(Length::Px(0.0)),
                    width: Some(Length::Percent(100.0)),
                    color: Some(self.highlight),
                    // The base word already casts the shadow.
                    text_shadow: Some(Vec::new()),
                    ..Style::default()
                };
                let motion = Motion {
                    opacity: visibility(word.start, Some(end)),
                    hold_before: true,
                    hold_after: true,
                    ..Motion::default()
                };
                push_piece(out, tag, holder, text(), overlay, motion);
            }
        }
    }

    /// Scale pulse of the spoken word: up over `POP_RISE`, held, and back down after `end`.
    fn pop_motion(&self, start: f64, end: f64) -> Motion {
        if self.pop <= 1.0 {
            return Motion::default();
        }
        let rise = POP_RISE.min((end - start) / 2.0);
        let point = |at: f64, value: f32, easing: Easing| TrackPoint { at, value, easing };
        Motion {
            scale: Track(vec![
                point(start, 1.0, Easing::Linear),
                point(start + rise, self.pop, Easing::EaseOut),
                point(end, self.pop, Easing::Linear),
                point(end + rise, 1.0, Easing::EaseOut),
            ]),
            hold_before: true,
            hold_after: true,
            ..Motion::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Element, ElementBody};

    fn word(text: &str, start: f64, end: f64) -> Word {
        Word {
            text: text.to_string(),
            start,
            end,
        }
    }

    fn spec(mode: Mode) -> CaptionsSpec {
        CaptionsSpec {
            tag: "div".to_string(),
            cues: vec![
                Cue {
                    words: vec![word("Hi", 1.0, 1.4), word("there", 1.5, 2.0)],
                },
                Cue {
                    words: vec![word("Bye", 2.1, 2.5)],
                },
            ],
            mode,
            highlight: Color {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            pop: 1.1,
            linger: 0.3,
            gap: 8.0,
        }
    }

    fn expanded(mode: Mode) -> Vec<Pending> {
        let host = Pending {
            element: Element::new(
                None,
                0.0,
                f64::INFINITY,
                ElementBody {
                    tag: "div".to_string(),
                    content: Content::Container,
                    style: Style::default(),
                    fade_in: 0.0,
                    fade_out: 0.0,
                    track: 0,
                    motion: Motion::default(),
                    layout: Default::default(),
                    spans: Vec::new(),
                    effects: Default::default(),
                    depth: Default::default(),
                },
            ),
            start_expr: None,
            duration: None,
            keyframes: Vec::new(),
            camera: Default::default(),
            transition: None,
            reveal: None,
            animation: None,
            motion_path: None,
        };
        let mut out = vec![host];
        spec(mode).expand(0, &Style::default(), &mut out).unwrap();
        out
    }

    #[test]
    fn rows_show_only_during_their_cue_and_never_overlap() {
        let out = expanded(Mode::Plain);
        let rows: Vec<_> = out[0]
            .element
            .children
            .iter()
            .map(|&i| &out[i].element.motion)
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pose(0.5).opacity, 0.0);
        assert_eq!(rows[0].pose(1.2).opacity, 1.0);
        // Cue 1 ends at 2.0 + 0.3 linger but the next cue starts at 2.1.
        assert_eq!(rows[0].pose(2.1).opacity, 0.0);
        assert_eq!(rows[1].pose(2.1).opacity, 1.0);
        assert_eq!(rows[1].pose(3.0).opacity, 0.0);
    }

    #[test]
    fn highlight_follows_the_spoken_word() {
        let out = expanded(Mode::Highlight);
        let row = out[0].element.children[0];
        let holders = &out[row].element.children;
        let overlay = |holder: usize| &out[out[holder].element.children[1]].element.motion;
        // "Hi" ends at 1.4, "there" starts at 1.5 (< 0.3 apart), so the highlight bridges to 1.5.
        assert_eq!(overlay(holders[0]).pose(1.45).opacity, 1.0);
        assert_eq!(overlay(holders[0]).pose(1.5).opacity, 0.0);
        assert_eq!(overlay(holders[1]).pose(1.5).opacity, 1.0);
        assert_eq!(overlay(holders[1]).pose(2.05).opacity, 0.0);
        let pop = &out[holders[0]].element.motion;
        assert!((pop.pose(1.2).scale - 1.1).abs() < 1e-6);
        assert_eq!(pop.pose(0.0).scale, 1.0);
    }

    #[test]
    fn reveal_keeps_the_slot_and_shows_words_as_spoken() {
        let out = expanded(Mode::Reveal);
        let row = out[0].element.children[0];
        let second = &out[out[row].element.children[1]].element.motion;
        assert_eq!(second.pose(1.2).opacity, 0.0);
        assert_eq!(second.pose(1.6).opacity, 1.0);
    }

    #[test]
    fn negative_offset_drops_words_before_zero() {
        let cues = shift(spec(Mode::Plain).cues, -1.45);
        assert_eq!(cues[0].words.len(), 1);
        assert_eq!(cues[0].words[0].text, "there");
        assert!((cues[0].words[0].start - 0.05).abs() < 1e-9);
    }
}
