//! Resolution of inherited text properties (family, size, weight, style, kerning, color, alignment,
//! wrapping, fill, stroke) and of the per-run styles of rich text (inline spans).

use crate::model::{
    Background, Color, Composition, Gradient, LineHeight, Shadow, TextSpan, WhiteSpace,
};
use crate::render::text::fonts::FontRequest;
use crate::render::Renderer;

/// Text properties after inheritance; sizes are composition px, before any scale.
pub(in crate::render) struct TextProps<'a> {
    pub(in crate::render) family: &'a str,
    pub(in crate::render) size: f32,
    pub(in crate::render) weight: u16,
    /// `font-style: italic` or `oblique`.
    pub(in crate::render) italic: bool,
    /// `font-kerning: normal`: pair kerning from the font.
    pub(in crate::render) kerning: bool,
    /// Extra advance after every glyph, in composition px.
    pub(in crate::render) letter_spacing: f32,
    pub(in crate::render) color: Color,
    pub(in crate::render) align: &'a str,
    /// Distance between baselines.
    pub(in crate::render) line_height: f32,
    pub(in crate::render) nowrap: bool,
    pub(in crate::render) shadow: Option<Shadow>,
    /// Inherited `-webkit-text-fill-color`, if any (the fill is `color` otherwise).
    pub(in crate::render) fill_color: Option<Color>,
    /// Inherited `-webkit-text-stroke` width (composition px) and color (`None` is `currentColor`).
    pub(in crate::render) stroke_width: f32,
    pub(in crate::render) stroke_color: Option<Color>,
}

/// What fills a glyph before its own fill color: a background clipped to the text.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::render) enum Underlay {
    Solid(Color),
    /// A gradient laid over the element's padding box (`run: false`) or over the run's own extent
    /// on each line (`run: true`).
    Gradient {
        gradient: Gradient,
        run: bool,
    },
}

/// Paint and font of one run of glyphs: the element's own text (index 0) or one inline span.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::render) struct RunStyle<'a> {
    pub(in crate::render) family: &'a str,
    pub(in crate::render) weight: u16,
    pub(in crate::render) italic: bool,
    pub(in crate::render) letter_spacing: f32,
    pub(in crate::render) underlay: Option<Underlay>,
    pub(in crate::render) fill: Color,
    pub(in crate::render) stroke_width: f32,
    pub(in crate::render) stroke: Color,
}

impl<'a> RunStyle<'a> {
    /// The face this run is drawn with.
    pub(in crate::render) fn font(&self) -> FontRequest<'a> {
        FontRequest {
            family: self.family,
            weight: self.weight,
            italic: self.italic,
        }
    }

    /// Whether this run paints exactly like plain text of `fill` (no underlay or stroke).
    pub(in crate::render) fn is_plain(&self) -> bool {
        self.underlay.is_none() && self.stroke_width <= 0.0
    }
}

fn underlay(background: &Background, run: bool) -> Underlay {
    match background {
        Background::Solid(color) => Underlay::Solid(*color),
        other => Underlay::Gradient {
            gradient: other
                .to_gradient()
                .expect("non-solid backgrounds are gradients"),
            run,
        },
    }
}

/// Index into `run_styles` of the byte at `offset`: 0 for the element's text, `i + 1` for span `i`.
pub(in crate::render) fn style_index(spans: &[TextSpan], offset: usize) -> usize {
    let i = spans.partition_point(|span| span.end <= offset);
    match spans.get(i) {
        Some(span) if span.start <= offset => i + 1,
        _ => 0,
    }
}

impl<'a> Renderer<'a> {
    /// Inherited font, color, alignment, and wrapping for a text element.
    pub(in crate::render) fn text_props(&self, idx: usize) -> TextProps<'a> {
        let composition: &'a Composition = self.composition;
        let (mut weight, mut spacing, mut italic, mut kerning) = (None, None, None, None);
        let (mut family, mut font_size, mut color, mut align) = (None, None, None, None);
        let (mut line_height, mut white_space, mut shadow) = (None, None, None);
        let (mut fill_color, mut stroke_width, mut stroke_color) = (None, None, None);
        let mut current = Some(idx);
        while let Some(i) = current {
            let node = &composition.elements[i];
            family = family.or(node.style.font_family.as_deref());
            font_size = font_size.or(node.style.font_size);
            weight = weight.or(node.style.font_weight);
            italic = italic.or(node.style.font_italic);
            kerning = kerning.or(node.style.font_kerning);
            spacing = spacing.or(node.style.letter_spacing);
            color = color.or(node.style.color);
            align = align.or(node.style.text_align.as_deref());
            line_height = line_height.or(node.style.line_height);
            white_space = white_space.or(node.style.white_space);
            shadow = shadow.or(node.style.text_shadow);
            fill_color = fill_color.or(node.style.text_fill_color);
            stroke_width = stroke_width.or(node.style.text_stroke_width);
            stroke_color = stroke_color.or(node.style.text_stroke_color);
            current = node.parent;
        }
        let size = font_size.unwrap_or(match composition.elements[idx].tag.as_str() {
            "h1" => 64.0,
            "h2" => 48.0,
            "h3" => 36.0,
            _ => 24.0,
        });
        TextProps {
            family: family.unwrap_or("sans-serif"),
            size,
            weight: weight.unwrap_or(400),
            italic: italic.unwrap_or(false),
            kerning: kerning.unwrap_or(false),
            letter_spacing: spacing.unwrap_or(0.0),
            color: color.unwrap_or(Color::WHITE),
            align: align.unwrap_or("left"),
            line_height: match line_height.unwrap_or(LineHeight::Multiple(1.0)) {
                LineHeight::Multiple(multiple) => size * multiple,
                LineHeight::Px(px) => px,
            },
            nowrap: white_space == Some(WhiteSpace::Nowrap),
            shadow: shadow.flatten(),
            fill_color,
            stroke_width: stroke_width.unwrap_or(0.0),
            stroke_color,
        }
    }

    /// Run styles of text element `idx`: its own text first, then one per inline span.
    pub(in crate::render) fn run_styles(&self, idx: usize) -> Vec<RunStyle<'a>> {
        let composition: &'a Composition = self.composition;
        let element = &composition.elements[idx];
        let props = self.text_props(idx);
        let base = RunStyle {
            family: props.family,
            weight: props.weight,
            italic: props.italic,
            letter_spacing: props.letter_spacing,
            underlay: element
                .style
                .background
                .as_ref()
                .filter(|_| element.style.background_clip_text)
                .map(|background| underlay(background, false)),
            fill: props.fill_color.unwrap_or(props.color),
            stroke_width: props.stroke_width,
            stroke: props.stroke_color.unwrap_or(props.color),
        };
        let mut styles = Vec::with_capacity(element.spans.len() + 1);
        for span in &element.spans {
            let s = &span.style;
            let color = s.color.unwrap_or(props.color);
            styles.push(RunStyle {
                family: s.font_family.as_deref().unwrap_or(base.family),
                weight: s.font_weight.unwrap_or(base.weight),
                italic: s.font_italic.unwrap_or(base.italic),
                letter_spacing: s.letter_spacing.unwrap_or(base.letter_spacing),
                underlay: match &s.text_background {
                    Some(background) => Some(underlay(background, true)),
                    None => base.underlay.clone(),
                },
                fill: s.text_fill_color.or(props.fill_color).unwrap_or(color),
                stroke_width: s.text_stroke_width.unwrap_or(base.stroke_width),
                // `currentColor` follows the run's own color.
                stroke: s.text_stroke_color.or(props.stroke_color).unwrap_or(color),
            });
        }
        styles.insert(0, base);
        styles
    }
}
