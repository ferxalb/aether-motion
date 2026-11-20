//! `data-hf-split`: kinetic text. Text becomes a flex row of per-word (or per-letter) elements, each
//! with a staggered entrance motion compiled into ordinary motion tracks. Text with `<br>` breaks,
//! automatic wrapping, line motion, or line masks becomes a flex column of line rows, and the stagger
//! continues across lines.
//!
//! Attributes on the text element:
//! - `data-hf-split="words|chars"`; `data-hf-stagger` (seconds between pieces, default 0.06);
//!   `data-hf-split-duration` (seconds per piece, default 0.6);
//!   `data-hf-split-from` (`up` default, `down`, `left`, `right`, `scale`, `fade`, `mask`);
//!   `data-hf-split-distance` (px); `data-hf-split-easing` (default `ease-out-expo`; `spring(...)`
//!   included); `data-hf-split-gap` (px between words; defaults to 0.28 of the font size).
//! - Start values that override the preset channel by channel: `data-hf-split-x`, `-y` (px),
//!   `-scale`, `-rotate` (degrees), and `-opacity` (0 to 1). `data-hf-split-order`
//!   (`forward`, `reverse`, `center`, `edges`) sets which pieces start first.
//! - `data-hf-split-mask="words|lines"` clips each word (or each line) to its own box, so pieces
//!   slide out from under an edge.
//! - `data-hf-split-wrap` wraps lines automatically at the element's content width (see
//!   `crate::render::split_wrap_breaks`); the breaks are measured on a first parse and applied by a
//!   second one (see [`SplitPlan`]), so the pieces and their timing do not change.
//! - Line motion: `data-hf-split-line-x`, `-line-y`, `-line-scale`, `-line-opacity` (start values),
//!   `-line-stagger` (default 0.12 s), `-line-delay`, `-line-duration`, and `-line-easing`.
//!
//! `mask` slides pieces up without fading; pair it with `overflow: hidden` on the element (or with
//! `data-hf-split-mask`) for a line reveal. Each letter of `chars` records the letter after it, so
//! `font-kerning: normal` kerns across pieces; spacing between words uses the gap above.

mod motion;

use crate::model::{Content, Easing, Element, LayoutHints, Length, Motion, Style};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::html::{attr, number, signed_number};
use crate::parse::tree::Pending;
use anyhow::{anyhow, bail, Result};
use kuchiki::NodeRef;
use motion::{Entrance, Order, StartPose};
use std::cell::Cell;
use std::collections::HashMap;

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

/// What `data-hf-split-mask` clips.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mask {
    None,
    Words,
    Lines,
}

/// Entrance of each line row (`data-hf-split-line-*`).
#[derive(Clone, Copy, Debug)]
struct LineMotion {
    from: StartPose,
    delay: f64,
    stagger: f64,
    duration: f64,
    easing: Easing,
}

/// Measured line breaks of every `data-hf-split-wrap` element, shared by the two parses of a
/// composition. The first parse numbers the wrapping elements in document order and marks them with
/// `LayoutHints::split_wrap`; the renderer measures them; the second parse receives the breaks.
#[derive(Debug, Default)]
pub(in crate::parse) struct SplitPlan {
    /// Word indices (counted across the whole text) that start a new, wrapped line, by ordinal.
    breaks: Option<HashMap<u32, Vec<usize>>>,
    next: Cell<u32>,
}

impl SplitPlan {
    /// The plan of the second parse, with the breaks measured on the first.
    pub(in crate::parse) fn resolved(breaks: HashMap<u32, Vec<usize>>) -> Self {
        Self {
            breaks: Some(breaks),
            next: Cell::new(0),
        }
    }

    /// Ordinal the next wrapping element will claim.
    pub(in crate::parse) fn cursor(&self) -> u32 {
        self.next.get()
    }

    /// Whether this parse still measures breaks (the first parse).
    pub(in crate::parse) fn unresolved(&self) -> bool {
        self.breaks.is_none()
    }

    /// Claims `count` ordinals at once, for a reused scene whose wrapping elements were numbered
    /// when it was first compiled.
    pub(in crate::parse) fn skip(&self, count: u32) {
        self.next.set(self.next.get() + count);
    }

    /// Whether the `count` elements starting at ordinal `at` get the same breaks as those starting
    /// at `from` (always true on the first parse, which has no breaks yet).
    pub(in crate::parse) fn same_breaks(&self, from: u32, at: u32, count: u32) -> bool {
        self.breaks.as_ref().is_none_or(|all| {
            (0..count).all(|k| {
                let empty = Vec::new();
                all.get(&(from + k)).unwrap_or(&empty) == all.get(&(at + k)).unwrap_or(&empty)
            })
        })
    }

    /// Ordinal of the next wrapping element, and its breaks when they are already measured.
    fn claim(&self) -> (u32, Option<Vec<usize>>) {
        let ordinal = self.next.get();
        self.next.set(ordinal + 1);
        let breaks = self
            .breaks
            .as_ref()
            .map(|all| all.get(&ordinal).cloned().unwrap_or_default());
        (ordinal, breaks)
    }
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
    /// Explicit start values (`data-hf-split-x` and friends) over the preset of `origin`.
    from: StartPose,
    order: Order,
    mask: Mask,
    line: Option<LineMotion>,
    /// `data-hf-split-wrap`: `Some(ordinal)` in document order among wrapping elements.
    wrap: Option<u32>,
    /// Measured wrap breaks (second parse only); `None` while they are still unknown.
    breaks: Option<Vec<usize>>,
}

/// Reads the split attributes of `node`; `None` when `data-hf-split` is absent.
/// `inherited_align` is the nearest ancestor's `text-align`, used when the element sets none.
pub(in crate::parse) fn parse_split(
    node: &NodeRef,
    tag: &str,
    style: &Style,
    inherited_align: Option<&str>,
    plan: &SplitPlan,
) -> Result<Option<SplitSpec>> {
    let Some(unit) = attr(node, "data-hf-split") else {
        if let Some(name) = split_option_without_split(node) {
            bail!("{name} needs data-hf-split=\"words\" or \"chars\" on the same element");
        }
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
    let num =
        |name: &str, default: f64| -> Result<f64> { Ok(optional(node, name)?.unwrap_or(default)) };
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
    let distance = attr(node, "data-hf-split-distance")
        .map(|raw| signed_number(&raw, "data-hf-split-distance"))
        .transpose()?
        .unwrap_or(default_distance as f64);
    let gap = num("data-hf-split-gap", (size * 0.28) as f64)?;
    if distance.abs() > 10_000.0 || !(0.0..=1_000.0).contains(&gap) {
        bail!("data-hf-split-distance must be within 10000 px and data-hf-split-gap within 0..1000 px");
    }
    let easing = match attr(node, "data-hf-split-easing") {
        Some(raw) => parse_keyframe_easing(&raw)?,
        None => parse_keyframe_easing("ease-out-expo")?,
    };
    let order = match attr(node, "data-hf-split-order") {
        None => Order::Forward,
        Some(raw) => Order::parse(raw.trim()).ok_or_else(|| {
            anyhow!("data-hf-split-order must be forward, reverse, center, or edges: {raw}")
        })?,
    };
    let mask = match attr(node, "data-hf-split-mask").as_deref().map(str::trim) {
        None | Some("none") => Mask::None,
        Some("words") => Mask::Words,
        Some("lines") => Mask::Lines,
        Some(other) => bail!("data-hf-split-mask must be words, lines, or none: {other}"),
    };
    let wrap = match attr(node, "data-hf-split-wrap").as_deref().map(str::trim) {
        None | Some("false") => false,
        Some("" | "true") => true,
        Some(other) => bail!("data-hf-split-wrap must be true or false: {other}"),
    };
    let (wrap, breaks) = if wrap {
        let (ordinal, breaks) = plan.claim();
        (Some(ordinal), breaks)
    } else {
        (None, None)
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
        from: start_pose(node, "data-hf-split")?,
        order,
        mask,
        line: line_motion(node, duration, easing)?,
        wrap,
        breaks,
    }))
}

/// The first `data-hf-split-*` option present on `node`, for the error when `data-hf-split` is not.
fn split_option_without_split(node: &NodeRef) -> Option<String> {
    let element = node.as_element()?;
    let attributes = element.attributes.borrow();
    attributes
        .map
        .keys()
        .map(|name| name.local.to_string())
        .filter(|name| name.starts_with("data-hf-split-"))
        .min()
}

/// A finite number attribute, if present.
fn optional(node: &NodeRef, name: &str) -> Result<Option<f64>> {
    let Some(value) = attr(node, name).map(|raw| number(&raw, name)).transpose()? else {
        return Ok(None);
    };
    if !value.is_finite() {
        bail!("{name} must be a finite number");
    }
    Ok(Some(value))
}

/// `{prefix}-x`, `-y`, `-scale`, `-rotate`, and `-opacity` start values (offsets and angles may be
/// negative).
fn start_pose(node: &NodeRef, prefix: &str) -> Result<StartPose> {
    let get = |suffix: &str| {
        let name = format!("{prefix}-{suffix}");
        attr(node, &name)
            .map(|raw| signed_number(&raw, &name))
            .transpose()
    };
    let pose = StartPose {
        x: get("x")?.map(|v| v as f32),
        y: get("y")?.map(|v| v as f32),
        scale: get("scale")?.map(|v| v as f32),
        rotate: get("rotate")?.map(|v| v as f32),
        opacity: get("opacity")?.map(|v| v as f32),
    };
    let offset = |v: Option<f32>| v.is_some_and(|v| v.abs() > 10_000.0);
    if offset(pose.x) || offset(pose.y) {
        bail!("{prefix}-x and {prefix}-y must be within 10000 px");
    }
    if pose.scale.is_some_and(|s| !(0.0..=20.0).contains(&s)) {
        bail!("{prefix}-scale must be between 0 and 20");
    }
    if pose.rotate.is_some_and(|r| r.abs() > 3600.0) {
        bail!("{prefix}-rotate must be within 3600 degrees");
    }
    if pose.opacity.is_some_and(|o| !(0.0..=1.0).contains(&o)) {
        bail!("{prefix}-opacity must be between 0 and 1");
    }
    Ok(pose)
}

/// `data-hf-split-line-*`: `None` unless a line start value is set.
fn line_motion(node: &NodeRef, duration: f64, easing: Easing) -> Result<Option<LineMotion>> {
    let from = start_pose(node, "data-hf-split-line")?;
    let timing = ["stagger", "delay", "duration", "easing"]
        .map(|suffix| format!("data-hf-split-line-{suffix}"));
    if from.is_rest() {
        if let Some(name) = timing.iter().find(|name| attr(node, name).is_some()) {
            bail!("{name} needs a line start value such as data-hf-split-line-y or data-hf-split-line-opacity");
        }
        return Ok(None);
    }
    let stagger = optional(node, &timing[0])?.unwrap_or(0.12);
    let delay = optional(node, &timing[1])?.unwrap_or(0.0);
    let duration = optional(node, &timing[2])?.unwrap_or(duration);
    if !(0.0..=5.0).contains(&stagger) || !(0.0..=60.0).contains(&delay) {
        bail!("data-hf-split-line-stagger must be 0 to 5 s and data-hf-split-line-delay 0 to 60 s");
    }
    if !(0.01..=30.0).contains(&duration) {
        bail!("data-hf-split-line-duration must be between 0.01 and 30 seconds");
    }
    let easing = match attr(node, &timing[3]) {
        Some(raw) => parse_keyframe_easing(&raw)?,
        None => easing,
    };
    Ok(Some(LineMotion {
        from,
        delay,
        stagger,
        duration,
        easing,
    }))
}

impl SplitSpec {
    /// Whether the pieces sit in line rows inside a flex column (breaks, wrapping, line motion, or
    /// line masks) rather than directly in one flex row.
    fn uses_rows(&self, text: &str) -> bool {
        lines(text).count() > 1
            || self.wrap.is_some()
            || self.line.is_some()
            || self.mask == Mask::Lines
    }

    /// Ordinal of a wrapping element whose breaks are not measured yet (first parse).
    pub(in crate::parse) fn unresolved_wrap(&self) -> Option<u32> {
        self.wrap.filter(|_| self.breaks.is_none())
    }

    /// Turns the original element into the flex row that holds the pieces of `text`, or into a
    /// flex column of line rows (stretched to its width).
    pub(in crate::parse) fn apply_container_style(&self, style: &mut Style, text: &str) {
        if self.uses_rows(text) {
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

    /// Start values of a piece: the preset of `data-hf-split-from`, then the explicit overrides.
    fn piece_start(&self) -> StartPose {
        let d = self.distance;
        let fade = Some(0.0);
        let preset = match self.origin {
            Origin::Up => StartPose {
                y: Some(d),
                opacity: fade,
                ..StartPose::default()
            },
            Origin::Mask => StartPose {
                y: Some(d),
                ..StartPose::default()
            },
            Origin::Down => StartPose {
                y: Some(-d),
                opacity: fade,
                ..StartPose::default()
            },
            Origin::Left => StartPose {
                x: Some(-d),
                opacity: fade,
                ..StartPose::default()
            },
            Origin::Right => StartPose {
                x: Some(d),
                opacity: fade,
                ..StartPose::default()
            },
            Origin::Scale => StartPose {
                scale: Some(0.6),
                opacity: fade,
                ..StartPose::default()
            },
            Origin::Fade => StartPose {
                opacity: fade,
                ..StartPose::default()
            },
        };
        preset.overridden_by(self.from)
    }

    /// Entrance of piece `index` of `count`, delayed by its stagger steps; it holds before and after.
    fn entrance(&self, index: usize, count: usize) -> Motion {
        Entrance {
            from: self.piece_start(),
            delay: self.order.steps(index, count) * self.stagger,
            duration: self.duration,
            easing: self.easing,
        }
        .motion()
    }

    /// Appends the pieces of `text` as children of `parent` (the already pushed container): directly
    /// for one plain line, or inside one row per line.
    pub(in crate::parse) fn expand(
        &self,
        text: &str,
        parent: usize,
        out: &mut Vec<Pending>,
    ) -> Result<()> {
        let count: usize = lines(text)
            .flat_map(str::split_whitespace)
            .map(|word| match self.unit {
                Unit::Words => 1,
                Unit::Chars => word.chars().count(),
            })
            .sum();
        if count == 0 {
            return Err(anyhow!("data-hf-split needs some text"));
        }
        if count > MAX_PIECES {
            bail!("data-hf-split would create {count} pieces; the limit is {MAX_PIECES}");
        }
        let mut cursor = Cursor {
            piece: 0,
            word: 0,
            count,
        };
        if !self.uses_rows(text) {
            for word in text.split_whitespace() {
                self.expand_word(word, parent, out, &mut cursor);
            }
            return Ok(());
        }
        let breaks = self.breaks.as_deref().unwrap_or(&[]);
        let mut line_index = 0;
        for line in lines(text) {
            let words: Vec<&str> = line.split_whitespace().collect();
            if words.is_empty() {
                // An empty line keeps its height.
                let row = self.push_row(out, parent, line_index);
                let text = Content::Text(String::new());
                self.push(out, row, text, Style::default(), Motion::default());
                line_index += 1;
                continue;
            }
            let mut row = None;
            for (i, word) in words.iter().enumerate() {
                if row.is_none() || (i > 0 && breaks.contains(&cursor.word)) {
                    row = Some(self.push_row(out, parent, line_index));
                    line_index += 1;
                }
                self.expand_word(word, row.unwrap(), out, &mut cursor);
            }
        }
        Ok(())
    }

    /// Appends one line row to `parent`: it spans the element, aligns by `text-align`, and carries
    /// the line motion of line `index` and the line mask.
    fn push_row(&self, out: &mut Vec<Pending>, parent: usize, index: usize) -> usize {
        let mut style = Style {
            text_align: self.align.clone(),
            width: Some(Length::Percent(100.0)),
            overflow_hidden: self.mask == Mask::Lines,
            ..Style::default()
        };
        row_container_style(&mut style, self.gap);
        let motion = self.line.map_or_else(Motion::default, |line| {
            Entrance {
                from: line.from,
                delay: line.delay + index as f64 * line.stagger,
                duration: line.duration,
                easing: line.easing,
            }
            .motion()
        });
        self.push(out, parent, Content::Container, style, motion)
    }

    /// Appends the pieces of one word to `row`, numbering their entrances from the cursor.
    fn expand_word(&self, word: &str, row: usize, out: &mut Vec<Pending>, cursor: &mut Cursor) {
        cursor.word += 1;
        let clip = self.mask == Mask::Words;
        match self.unit {
            Unit::Words => {
                let holder = if clip {
                    let style = Style {
                        display: Some("flex".to_string()),
                        align_items: Some("flex-start".to_string()),
                        overflow_hidden: true,
                        ..Style::default()
                    };
                    self.push(out, row, Content::Container, style, Motion::default())
                } else {
                    row
                };
                let motion = self.entrance(cursor.piece, cursor.count);
                self.push(
                    out,
                    holder,
                    Content::Text(word.to_string()),
                    Style::default(),
                    motion,
                );
                cursor.piece += 1;
            }
            Unit::Chars => {
                let group = Style {
                    display: Some("flex".to_string()),
                    align_items: Some("flex-start".to_string()),
                    overflow_hidden: clip,
                    ..Style::default()
                };
                let holder = self.push(out, row, Content::Container, group, Motion::default());
                let mut letters = word.chars().peekable();
                while let Some(ch) = letters.next() {
                    let motion = self.entrance(cursor.piece, cursor.count);
                    let piece = self.push(
                        out,
                        holder,
                        Content::Text(ch.to_string()),
                        Style::default(),
                        motion,
                    );
                    out[piece].element.layout.kern_after = letters.peek().copied();
                    cursor.piece += 1;
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

/// Position of the expansion: pieces and words emitted so far, and the total piece count.
struct Cursor {
    piece: usize,
    /// Words emitted so far (also the index of the next word), across lines.
    word: usize,
    count: usize,
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
            effects: Default::default(),
        },
        start_expr: None,
        duration: None,
        keyframes: Vec::new(),
        camera: Default::default(),
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
            from: StartPose::default(),
            order: Order::Forward,
            mask: Mask::None,
            line: None,
            wrap: None,
            breaks: None,
        }
    }

    #[test]
    fn pieces_start_staggered_and_hold() {
        let s = spec(Unit::Words, Origin::Up);
        let third = s.entrance(2, 5);
        assert_eq!(third.pose(0.0).opacity, 0.0);
        assert_eq!(third.pose(0.2).y, 40.0);
        assert_eq!(third.pose(0.8).y, 0.0);
        assert_eq!(third.pose(0.8).opacity, 1.0);
        assert!((third.pose(0.45).y - 20.0).abs() < 1e-3);
    }

    #[test]
    fn mask_does_not_fade() {
        let motion = spec(Unit::Words, Origin::Mask).entrance(0, 1);
        assert!(motion.opacity.0.is_empty());
        assert_eq!(motion.pose(0.0).opacity, 1.0);
    }

    #[test]
    fn explicit_start_values_override_the_preset() {
        let mut s = spec(Unit::Words, Origin::Up);
        s.from = StartPose {
            scale: Some(0.5),
            opacity: Some(0.25),
            ..StartPose::default()
        };
        let start = s.entrance(0, 1).pose(0.0);
        assert_eq!((start.y, start.scale, start.opacity), (40.0, 0.5, 0.25));
    }

    fn expanded(spec: &SplitSpec, text: &str) -> Vec<Pending> {
        let mut out = Vec::new();
        out.push(Pending {
            element: Element {
                tag: "h1".into(),
                id: None,
                parent: None,
                children: Vec::new(),
                content: Content::Container,
                style: Style::default(),
                start: 0.0,
                end: 1.0,
                fade_in: 0.0,
                fade_out: 0.0,
                track: 0,
                motion: Motion::default(),
                layout: LayoutHints::default(),
                spans: Vec::new(),
                effects: Default::default(),
            },
            start_expr: None,
            duration: None,
            keyframes: Vec::new(),
            camera: Default::default(),
            transition: None,
            reveal: None,
            animation: None,
            motion_path: None,
        });
        spec.expand(text, 0, &mut out).unwrap();
        out
    }

    #[test]
    fn measured_breaks_split_lines_without_changing_piece_timing() {
        let mut s = spec(Unit::Words, Origin::Up);
        s.wrap = Some(0);
        s.breaks = Some(vec![2]);
        s.line = Some(LineMotion {
            from: StartPose {
                y: Some(30.0),
                ..StartPose::default()
            },
            delay: 0.0,
            stagger: 0.25,
            duration: 0.5,
            easing: Easing::Linear,
        });
        let out = expanded(&s, "one two three four");
        let rows = &out[0].element.children;
        assert_eq!(rows.len(), 2);
        let words = |row: usize| out[row].element.children.len();
        assert_eq!((words(rows[0]), words(rows[1])), (2, 2));
        // The third word keeps its own stagger (index 2), and its line starts one line later.
        let third = &out[out[rows[1]].element.children[0]].element.motion;
        assert_eq!(third.x.0.len() + third.y.0.len(), 2);
        assert_eq!(third.y.0[0].at, 0.2);
        let second_line = &out[rows[1]].element.motion;
        assert_eq!(second_line.y.0[0].at, 0.25);
        assert_eq!(second_line.pose(0.0).y, 30.0);
    }

    #[test]
    fn word_masks_wrap_each_word_in_a_clip() {
        let mut s = spec(Unit::Words, Origin::Mask);
        s.mask = Mask::Words;
        let out = expanded(&s, "a b");
        let holders = &out[0].element.children;
        assert_eq!(holders.len(), 2);
        assert!(holders
            .iter()
            .all(|&h| out[h].element.style.overflow_hidden));
        assert!(holders.iter().all(|&h| out[h].element.children.len() == 1));
    }
}
