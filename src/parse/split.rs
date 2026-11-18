//! `data-hf-split`: kinetic text. Text becomes a flex row of per-word (or per-letter) elements, each
//! with a staggered entrance motion compiled into ordinary motion tracks. Text with `<br>` breaks
//! becomes a flex column of such rows, one per line, and the stagger continues across lines.
//!
//! Attributes on the text element:
//! - `data-hf-split="words|chars"`; `data-hf-stagger` (seconds between pieces, default 0.06);
//!   `data-hf-split-duration` (seconds per piece, default 0.6);
//!   `data-hf-split-from` (`up` default, `down`, `left`, `right`, `scale`, `fade`, `mask`);
//!   `data-hf-split-distance` (px); `data-hf-split-easing` (default `ease-out-expo`);
//!   `data-hf-split-gap` (px between words; defaults to 0.28 of the font size).
//!
//! `mask` slides pieces up without fading; pair it with `overflow: hidden` on the element for a line
//! reveal. Lines break only at `<br>` (a line wider than its box overflows). Each letter of `chars`
//! records the letter after it, so `font-kerning: normal` kerns across pieces; spacing between words
//! uses the gap above.
use crate::model::{
    Content, Easing, Element, LayoutHints, Length, Motion, Style, Track, TrackPoint,
};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::html::{attr, number};
use crate::parse::tree::Pending;
use anyhow::{anyhow, bail, Result};
use kuchiki::NodeRef;

const MAX_PIECES: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Words,
    Chars,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Origin {
    Up,
    Down,
    Left,
    Right,
    Scale,
    Fade,
    Mask,
}

#[derive(Clone, Debug)]
pub(in crate::parse) struct SplitSpec {
    /// Tag of the original element; generated pieces reuse it so default font sizes still apply.
    tag: String,
    unit: Unit,
    origin: Origin,
    stagger: f64,
    duration: f64,
    distance: f32,
    easing: Easing,
    gap: f32,
    /// The element's `text-align` (its own, else inherited), which its rows follow.
    align: Option<String>,
}

/// Reads the split attributes of `node`; `None` when `data-hf-split` is absent.
/// `inherited_align` is the nearest ancestor's `text-align`, used when the element sets none.
pub(in crate::parse) fn parse_split(
    node: &NodeRef,
    tag: &str,
    style: &Style,
    inherited_align: Option<&str>,
) -> Result<Option<SplitSpec>> {
    let Some(unit) = attr(node, "data-hf-split") else {
        return Ok(None);
    };
    let unit = match unit.as_str() {
        "words" => Unit::Words,
        "chars" => Unit::Chars,
        other => bail!("data-hf-split must be words or chars: {other}"),
    };
    let origin = match attr(node, "data-hf-split-from").as_deref().unwrap_or("up") {
        "up" => Origin::Up,
        "down" => Origin::Down,
        "left" => Origin::Left,
        "right" => Origin::Right,
        "scale" => Origin::Scale,
        "fade" => Origin::Fade,
        "mask" => Origin::Mask,
        other => {
            bail!("data-hf-split-from must be up, down, left, right, scale, fade, or mask: {other}")
        }
    };
    let size = style.font_size.unwrap_or(match tag {
        "h1" => 64.0,
        "h2" => 48.0,
        "h3" => 36.0,
        _ => 24.0,
    });
    let num = |name: &str, default: f64| -> Result<f64> {
        let value = attr(node, name)
            .map(|raw| number(&raw, name))
            .transpose()?
            .unwrap_or(default);
        if !value.is_finite() {
            bail!("{name} must be a finite number");
        }
        Ok(value)
    };
    let stagger = num("data-hf-stagger", 0.06)?;
    let duration = num("data-hf-split-duration", 0.6)?;
    if !(0.0..=5.0).contains(&stagger) {
        bail!("data-hf-stagger must be between 0 and 5 seconds");
    }
    if !(0.01..=30.0).contains(&duration) {
        bail!("data-hf-split-duration must be between 0.01 and 30 seconds");
    }
    let default_distance = if origin == Origin::Mask {
        size * 1.2
    } else {
        size * 0.6
    };
    let distance = num("data-hf-split-distance", default_distance as f64)?;
    let gap = num("data-hf-split-gap", (size * 0.28) as f64)?;
    if distance.abs() > 10_000.0 || !(0.0..=1_000.0).contains(&gap) {
        bail!("data-hf-split-distance must be within 10000 px and data-hf-split-gap within 0..1000 px");
    }
    let easing = match attr(node, "data-hf-split-easing") {
        Some(raw) => parse_keyframe_easing(&raw)?,
        None => parse_keyframe_easing("ease-out-expo")?,
    };
    Ok(Some(SplitSpec {
        tag: tag.to_string(),
        unit,
        origin,
        stagger,
        duration,
        distance: distance as f32,
        easing,
        gap: gap as f32,
        align: style
            .text_align
            .clone()
            .or_else(|| inherited_align.map(str::to_string)),
    }))
}

impl SplitSpec {
    /// Turns the original element into the flex row that holds the pieces of `text`, or, when it
    /// has `<br>` breaks, into a flex column of line rows (stretched to its width).
    pub(in crate::parse) fn apply_container_style(&self, style: &mut Style, text: &str) {
        if lines(text).count() > 1 {
            style.display = Some("flex".to_string());
            style.column = true;
            style.gap = 0.0;
            style.align_items = None;
            style.justify_content = None;
        } else {
            let own = style.text_align.clone();
            style.text_align = self.align.clone();
            row_container_style(style, self.gap);
            style.text_align = own;
        }
    }

    /// Entrance of piece `index`: starts at `index * stagger` after the clip start and holds before and after.
    fn entrance(&self, index: usize) -> Motion {
        let delay = index as f64 * self.stagger;
        let end = delay + self.duration;
        let track = |from: f32, to: f32, easing: Easing| {
            Track(vec![
                TrackPoint {
                    at: delay,
                    value: from,
                    easing: Easing::Linear,
                },
                TrackPoint {
                    at: end,
                    value: to,
                    easing,
                },
            ])
        };
        let d = self.distance;
        let mut motion = Motion {
            hold_before: true,
            hold_after: true,
            ..Motion::default()
        };
        match self.origin {
            Origin::Up | Origin::Mask => motion.y = track(d, 0.0, self.easing),
            Origin::Down => motion.y = track(-d, 0.0, self.easing),
            Origin::Left => motion.x = track(-d, 0.0, self.easing),
            Origin::Right => motion.x = track(d, 0.0, self.easing),
            Origin::Scale => motion.scale = track(0.6, 1.0, self.easing),
            Origin::Fade => {}
        }
        // Opacity never overshoots, even with `back` easings.
        if self.origin != Origin::Mask {
            motion.opacity = track(0.0, 1.0, Easing::EaseOut);
        }
        motion
    }

    /// Appends the pieces of `text` as children of `parent` (the already pushed container): directly
    /// for one line, or inside one row per line for text with `<br>` breaks.
    pub(in crate::parse) fn expand(
        &self,
        text: &str,
        parent: usize,
        out: &mut Vec<Pending>,
    ) -> Result<()> {
        let pieces: usize = lines(text)
            .flat_map(str::split_whitespace)
            .map(|word| match self.unit {
                Unit::Words => 1,
                Unit::Chars => word.chars().count(),
            })
            .sum();
        if pieces == 0 {
            return Err(anyhow!("data-hf-split needs some text"));
        }
        if pieces > MAX_PIECES {
            bail!("data-hf-split would create {pieces} pieces; the limit is {MAX_PIECES}");
        }
        let multiline = lines(text).count() > 1;
        let mut index = 0;
        for line in lines(text) {
            let row = if multiline {
                // Rows span the element, so each line aligns by `text-align`.
                let mut style = Style {
                    text_align: self.align.clone(),
                    width: Some(Length::Percent(100.0)),
                    ..Style::default()
                };
                row_container_style(&mut style, self.gap);
                let row = self.push(out, parent, Content::Container, style, Motion::default());
                if line.split_whitespace().next().is_none() {
                    // An empty line keeps its height.
                    let text = Content::Text(String::new());
                    self.push(out, row, text, Style::default(), Motion::default());
                }
                row
            } else {
                parent
            };
            self.expand_line(line, row, out, &mut index);
        }
        Ok(())
    }

    /// Appends the pieces of one line to `row`, numbering their entrances from `index`.
    fn expand_line(&self, line: &str, row: usize, out: &mut Vec<Pending>, index: &mut usize) {
        for word in line.split_whitespace() {
            match self.unit {
                Unit::Words => {
                    self.push(
                        out,
                        row,
                        Content::Text(word.to_string()),
                        Style::default(),
                        self.entrance(*index),
                    );
                    *index += 1;
                }
                Unit::Chars => {
                    let group = Style {
                        display: Some("flex".to_string()),
                        align_items: Some("flex-start".to_string()),
                        ..Style::default()
                    };
                    let holder = self.push(out, row, Content::Container, group, Motion::default());
                    let mut letters = word.chars().peekable();
                    while let Some(ch) = letters.next() {
                        let piece = self.push(
                            out,
                            holder,
                            Content::Text(ch.to_string()),
                            Style::default(),
                            self.entrance(*index),
                        );
                        out[piece].element.layout.kern_after = letters.peek().copied();
                        *index += 1;
                    }
                }
            }
        }
    }

    fn push(
        &self,
        out: &mut Vec<Pending>,
        parent: usize,
        content: Content,
        style: Style,
        motion: Motion,
    ) -> usize {
        push_piece(out, &self.tag, parent, content, style, motion)
    }
}

/// Lines of split text: its `<br>` breaks. `html::direct_text` already dropped the empty line of a
/// single trailing `<br>`, so a remaining empty line (from consecutive breaks) is a blank row.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
}

/// Appends a generated child of `parent` that reuses `tag` (so default font sizes still apply) and
/// inherits its parent's start; returns its index.
pub(in crate::parse) fn push_piece(
    out: &mut Vec<Pending>,
    tag: &str,
    parent: usize,
    content: Content,
    style: Style,
    motion: Motion,
) -> usize {
    let idx = out.len();
    out.push(Pending {
        element: Element {
            tag: tag.to_string(),
            id: None,
            parent: Some(parent),
            children: Vec::new(),
            content,
            style,
            start: 0.0,
            end: f64::INFINITY,
            fade_in: 0.0,
            fade_out: 0.0,
            track: 0,
            motion,
            layout: LayoutHints::default(),
            spans: Vec::new(),
        },
        start_expr: None,
        duration: None,
        keyframes: Vec::new(),
        camera: Vec::new(),
        transition: None,
        reveal: None,
        animation: None,
        motion_path: None,
    });
    out[parent].element.children.push(idx);
    idx
}

/// Turns `style` into a flex row whose pieces sit at the start, center, or end following `text-align`.
pub(in crate::parse) fn row_container_style(style: &mut Style, gap: f32) {
    style.display = Some("flex".to_string());
    style.column = false;
    style.gap = gap;
    style.align_items = Some("flex-start".to_string());
    style.justify_content = Some(
        match style.text_align.as_deref() {
            Some("center") => "center",
            Some("right") => "flex-end",
            _ => "flex-start",
        }
        .to_string(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(unit: Unit, origin: Origin) -> SplitSpec {
        SplitSpec {
            tag: "h1".to_string(),
            unit,
            origin,
            stagger: 0.1,
            duration: 0.5,
            distance: 40.0,
            easing: Easing::Linear,
            gap: 8.0,
            align: None,
        }
    }

    #[test]
    fn pieces_start_staggered_and_hold() {
        let s = spec(Unit::Words, Origin::Up);
        let third = s.entrance(2);
        assert_eq!(third.pose(0.0).opacity, 0.0);
        assert_eq!(third.pose(0.2).y, 40.0);
        assert_eq!(third.pose(0.8).y, 0.0);
        assert_eq!(third.pose(0.8).opacity, 1.0);
        assert!((third.pose(0.45).y - 20.0).abs() < 1e-3);
    }

    #[test]
    fn mask_does_not_fade() {
        let motion = spec(Unit::Words, Origin::Mask).entrance(0);
        assert!(motion.opacity.0.is_empty());
        assert_eq!(motion.pose(0.0).opacity, 1.0);
    }
}
