//! Resolution of inherited text properties (family, size, color, alignment).

impl<'a> Renderer<'a> {
    /// Inherited font family, base size (comp px), color and alignment for a text element.
    pub(in crate::render) fn text_props(&self, idx: usize) -> (&'a str, f32, Color, &'a str) {
        let composition: &'a Composition = self.composition;
        let (mut family, mut font_size, mut color, mut align) = (None, None, None, None);
        let mut current = Some(idx);
        while let Some(i) = current {
            let node = &composition.elements[i];
            family = family.or(node.style.font_family.as_deref());
            font_size = font_size.or(node.style.font_size);
            color = color.or(node.style.color);
            align = align.or(node.style.text_align.as_deref());
            current = node.parent;
        }
        let size = font_size.unwrap_or(match composition.elements[idx].tag.as_str() {
            "h1" => 64.0,
            "h2" => 48.0,
            "h3" => 36.0,
            _ => 24.0,
        });
        (
            family.unwrap_or("sans-serif"),
            size,
            color.unwrap_or(Color::WHITE),
            align.unwrap_or("left"),
        )
    }
}
