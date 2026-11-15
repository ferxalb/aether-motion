//! Resolution of inherited text properties (family, size, color, alignment, wrapping).

use crate::model::{Color, Composition, LineHeight, WhiteSpace};
use crate::render::Renderer;

/// Text properties after inheritance; sizes are composition px, before any scale.
pub(in crate::render) struct TextProps<'a> {
    pub(in crate::render) family: &'a str,
    pub(in crate::render) size: f32,
    pub(in crate::render) color: Color,
    pub(in crate::render) align: &'a str,
    /// Distance between baselines.
    pub(in crate::render) line_height: f32,
    pub(in crate::render) nowrap: bool,
}

impl<'a> Renderer<'a> {
    /// Inherited font, color, alignment, and wrapping for a text element.
    pub(in crate::render) fn text_props(&self, idx: usize) -> TextProps<'a> {
        let composition: &'a Composition = self.composition;
        let (mut family, mut font_size, mut color, mut align) = (None, None, None, None);
        let (mut line_height, mut white_space) = (None, None);
        let mut current = Some(idx);
        while let Some(i) = current {
            let node = &composition.elements[i];
            family = family.or(node.style.font_family.as_deref());
            font_size = font_size.or(node.style.font_size);
            color = color.or(node.style.color);
            align = align.or(node.style.text_align.as_deref());
            line_height = line_height.or(node.style.line_height);
            white_space = white_space.or(node.style.white_space);
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
            color: color.unwrap_or(Color::WHITE),
            align: align.unwrap_or("left"),
            line_height: match line_height.unwrap_or(LineHeight::Multiple(1.0)) {
                LineHeight::Multiple(multiple) => size * multiple,
                LineHeight::Px(px) => px,
            },
            nowrap: white_space == Some(WhiteSpace::Nowrap),
        }
    }
}
