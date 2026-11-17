//! Inline rich text: an element with its own text and inline children (`<span>`, `<strong>`, `<b>`,
//! `<code>`) becomes one text element whose styled runs (`model::TextSpan`) wrap and align together.
//!
//! Runs may change `color`, `font-family`, `font-weight`, `letter-spacing`,
//! `-webkit-text-fill-color`, `-webkit-text-stroke`, and fill their glyphs with a gradient through
//! `background` plus `background-clip: text`. Box properties (size, position, padding, borders,
//! backgrounds without `background-clip: text`, transforms, effects, animation) and `font-size`
//! changes are explicit errors, since a run is not a box. Whitespace collapses across runs exactly as
//! the source spells it: no space is added between `Make<span>s</span>`.
use crate::model::{InlineStyle, Style, TextSpan};
use crate::parse::css::{Stylesheet, Vars};
use crate::parse::html::attr;
use crate::parse::style::{style_for, Resolved};
use anyhow::{bail, Context, Result};
use kuchiki::NodeRef;

/// Tags that become inline runs inside an element with its own text.
const INLINE: [&str; 4] = ["span", "strong", "b", "code"];

fn tag(node: &NodeRef) -> Option<String> {
    node.as_element().map(|e| e.name.local.to_string())
}

fn is_inline(node: &NodeRef) -> bool {
    tag(node).is_some_and(|t| INLINE.contains(&t.as_str()))
}

/// Whether `node` holds rich text: its own non-blank text plus at least one inline child. An element
/// with `data-var-text` keeps its children as elements, since the binding replaces only its own text.
pub(in crate::parse) fn has_inline_runs(node: &NodeRef) -> bool {
    attr(node, "data-var-text").is_none()
        && node
            .children()
            .any(|c| c.as_text().is_some_and(|t| !t.borrow().trim().is_empty()))
        && node.children().any(|c| is_inline(&c))
}

/// Rich text of `node`: normalized text, its styled runs, and the inline children it consumed (they
/// are not elements of their own).
pub(in crate::parse) struct RichText {
    pub(in crate::parse) text: String,
    pub(in crate::parse) spans: Vec<TextSpan>,
    pub(in crate::parse) consumed: Vec<NodeRef>,
}

enum Piece {
    Text(String, Option<usize>),
    Break,
}

pub(in crate::parse) fn collect(
    node: &NodeRef,
    sheet: &Stylesheet,
    vars: &Vars,
) -> Result<RichText> {
    let mut styles: Vec<InlineStyle> = Vec::new();
    let mut pieces = Vec::new();
    let mut consumed = Vec::new();
    for child in node.children() {
        if let Some(text) = child.as_text() {
            pieces.push(Piece::Text(text.borrow().to_string(), None));
        } else if tag(&child).as_deref() == Some("br") {
            pieces.push(Piece::Break);
        } else if is_inline(&child) {
            visit(&child, None, sheet, vars, &mut styles, &mut pieces)?;
            consumed.push(child);
        }
    }
    let (text, owners) = normalize(&pieces);
    let mut spans: Vec<TextSpan> = Vec::new();
    let mut offset = 0;
    for (ch, owner) in text.chars().zip(&owners) {
        let end = offset + ch.len_utf8();
        if let Some(index) = *owner {
            match spans.last_mut() {
                Some(span) if span.end == offset && span.style == styles[index] => span.end = end,
                _ => spans.push(TextSpan {
                    start: offset,
                    end,
                    style: styles[index].clone(),
                }),
            }
        }
        offset = end;
    }
    Ok(RichText {
        text,
        spans,
        consumed,
    })
}

/// Collects the text of inline element `node`, styled over the enclosing run `outer`.
fn visit(
    node: &NodeRef,
    outer: Option<usize>,
    sheet: &Stylesheet,
    vars: &Vars,
    styles: &mut Vec<InlineStyle>,
    pieces: &mut Vec<Piece>,
) -> Result<()> {
    let name = tag(node).unwrap_or_default();
    let label = || {
        format!(
            "inline <{name}> ({})",
            attr(node, "id").as_deref().unwrap_or("no id")
        )
    };
    if let Some(element) = node.as_element() {
        for key in element.attributes.borrow().map.keys() {
            let key = &*key.local;
            if key.starts_with("data-") {
                bail!("{}: {key} is not supported on inline text; put it on the text element or use a separate element", label());
            }
        }
    }
    if attr(node, "class").is_some_and(|c| c.split_whitespace().any(|x| x == "clip")) {
        bail!(
            "{}: inline text cannot be a clip; give it its own element outside the text",
            label()
        );
    }
    let Resolved {
        style,
        vars: inner_vars,
        animation,
    } = style_for(node, sheet, vars, false).with_context(label)?;
    if animation.is_some() {
        bail!("{}: CSS animation is not supported on inline text", label());
    }
    if let Some(property) = box_property(&style) {
        bail!("{}: {property} is not supported on inline text (runs are not boxes); style the text element instead", label());
    }
    let mut own = InlineStyle {
        color: style.color,
        font_family: style.font_family.clone(),
        font_weight: style.font_weight,
        letter_spacing: style.letter_spacing,
        text_fill_color: style.text_fill_color,
        text_stroke_width: style.text_stroke_width,
        text_stroke_color: style.text_stroke_color,
        text_background: style
            .background
            .clone()
            .filter(|_| style.background_clip_text),
    };
    match name.as_str() {
        "strong" | "b" => own.font_weight = own.font_weight.or(Some(700)),
        "code" => own.font_family = own.font_family.or_else(|| Some("monospace".into())),
        _ => {}
    }
    let merged = match outer {
        Some(index) => inherit(&styles[index], own),
        None => own,
    };
    styles.push(merged);
    let index = styles.len() - 1;
    for child in node.children() {
        if let Some(text) = child.as_text() {
            pieces.push(Piece::Text(text.borrow().to_string(), Some(index)));
        } else if tag(&child).as_deref() == Some("br") {
            pieces.push(Piece::Break);
        } else if is_inline(&child) {
            visit(&child, Some(index), sheet, &inner_vars, styles, pieces)?;
        } else if let Some(inner) = tag(&child) {
            bail!(
                "{}: <{inner}> cannot be inside inline text; only span, strong, b, code, and br",
                label()
            );
        }
    }
    Ok(())
}

/// `own` over `outer`: fields `own` sets win.
fn inherit(outer: &InlineStyle, own: InlineStyle) -> InlineStyle {
    InlineStyle {
        color: own.color.or(outer.color),
        font_family: own.font_family.or_else(|| outer.font_family.clone()),
        font_weight: own.font_weight.or(outer.font_weight),
        letter_spacing: own.letter_spacing.or(outer.letter_spacing),
        text_fill_color: own.text_fill_color.or(outer.text_fill_color),
        text_stroke_width: own.text_stroke_width.or(outer.text_stroke_width),
        text_stroke_color: own.text_stroke_color.or(outer.text_stroke_color),
        // A gradient fills the run that declares it; inner runs keep painting with it.
        text_background: own
            .text_background
            .or_else(|| outer.text_background.clone()),
    }
}

/// The first property set on `style` that an inline run cannot have.
fn box_property(style: &Style) -> Option<&'static str> {
    let checks: [(bool, &'static str); 30] = [
        (style.font_size.is_some(), "font-size"),
        (style.line_height.is_some(), "line-height"),
        (style.text_shadow.is_some(), "text-shadow"),
        (style.text_align.is_some(), "text-align"),
        (style.white_space.is_some(), "white-space"),
        (style.left.is_some() || style.top.is_some(), "left/top"),
        (
            style.width.is_some() || style.height.is_some(),
            "width/height",
        ),
        (style.max_width.is_some(), "max-width"),
        (
            style.background.is_some() && !style.background_clip_text,
            "background without background-clip: text",
        ),
        (
            style.background_clip_text && style.background.is_none(),
            "background-clip: text without a background",
        ),
        (style.border_radius != 0.0, "border-radius"),
        (style.border_width != 0.0, "border"),
        (style.opacity.is_some(), "opacity"),
        (!style.shadows.is_empty(), "box-shadow"),
        (style.backdrop_blur != 0.0, "backdrop-filter"),
        (
            style.filter_blur != 0.0
                || !style.filter_colors.is_empty()
                || style.drop_shadow.is_some(),
            "filter",
        ),
        (
            style.blend != crate::model::BlendMode::Normal,
            "mix-blend-mode",
        ),
        (style.padding != [0.0; 4], "padding"),
        (style.margin != [0.0; 4], "margin"),
        (style.overflow_hidden, "overflow"),
        (style.display.is_some(), "display"),
        (style.position.is_some(), "position"),
        (style.z_index.is_some(), "z-index"),
        (
            style.translate != (0.0, 0.0) || style.scale.is_some() || style.rotate != 0.0,
            "transform",
        ),
        (style.clip_path.is_some(), "clip-path"),
        (style.mask.is_some(), "mask-image"),
        (style.flex_grow != 0.0, "flex"),
        (style.gap != 0.0 || style.column, "flex layout"),
        (
            style.align_items.is_some() || style.justify_content.is_some(),
            "alignment",
        ),
        (style.object_fit.is_some(), "object-fit"),
    ];
    checks
        .into_iter()
        .find(|(set, _)| *set)
        .map(|(_, name)| name)
}

/// Collapses whitespace like `white-space: normal`: runs of whitespace become one space, lines are
/// trimmed, and a trailing `<br>` adds no line. Returns the text and the run index of each char.
fn normalize(pieces: &[Piece]) -> (String, Vec<Option<usize>>) {
    let mut text = String::new();
    let mut owners: Vec<Option<usize>> = Vec::new();
    let mut pending: Option<Option<usize>> = None;
    let mut line_start = true;
    for piece in pieces {
        match piece {
            Piece::Break => {
                text.push('\n');
                owners.push(None);
                pending = None;
                line_start = true;
            }
            Piece::Text(s, owner) => {
                for ch in s.chars() {
                    if ch.is_whitespace() {
                        if !line_start && pending.is_none() {
                            pending = Some(*owner);
                        }
                        continue;
                    }
                    if let Some(space_owner) = pending.take() {
                        text.push(' ');
                        owners.push(space_owner);
                    }
                    text.push(ch);
                    owners.push(*owner);
                    line_start = false;
                }
            }
        }
    }
    if text.ends_with('\n') && text.len() > 1 {
        text.pop();
        owners.pop();
    }
    (text, owners)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(pieces: &[(&str, Option<usize>)]) -> (String, Vec<Option<usize>>) {
        let pieces: Vec<Piece> = pieces
            .iter()
            .map(|(s, o)| {
                if *s == "\n" {
                    Piece::Break
                } else {
                    Piece::Text(s.to_string(), *o)
                }
            })
            .collect();
        normalize(&pieces)
    }

    #[test]
    fn collapses_whitespace_without_adding_spaces_between_runs() {
        let (t, owners) = text(&[("  Make", None), ("s", Some(0)), (" \n  motion ", None)]);
        assert_eq!(t, "Makes motion");
        assert_eq!(owners[4], Some(0));
        assert_eq!(owners[5], None);
        let (t, _) = text(&[("a ", None), ("b", Some(0)), (" c", None)]);
        assert_eq!(t, "a b c");
    }

    #[test]
    fn breaks_trim_lines_and_a_trailing_break_adds_nothing() {
        let (t, _) = text(&[
            ("one ", None),
            ("\n", None),
            (" two", Some(0)),
            ("\n", None),
        ]);
        assert_eq!(t, "one\ntwo");
    }
}
