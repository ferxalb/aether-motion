//! Explicit and intrinsic sizing of a single element inside a layout.

impl<'a> Renderer<'a> {
    /// Explicit or intrinsic size of an element in layout units; `None` means unknown.
    pub(in crate::render) fn item_size(
        &mut self,
        idx: usize,
        ps: f32,
        avail: (f32, f32),
    ) -> Result<(Option<f32>, Option<f32>)> {
        let composition: &'a Composition = self.composition;
        let element = &composition.elements[idx];
        let mut width = element.style.width.map(|n| n.resolve_scaled(avail.0, ps));
        let mut height = element.style.height.map(|n| n.resolve_scaled(avail.1, ps));
        if width.is_some() && height.is_some() {
            return Ok((width, height));
        }
        let intrinsic = match &element.content {
            Content::Text(text) => Some(self.measure_text(text, idx, ps)?),
            Content::Container if element.style.display.as_deref() == Some("flex") => {
                self.flex_intrinsic(idx, ps)?
            }
            _ => None,
        };
        if let Some((w, h)) = intrinsic {
            width = width.or(Some(w));
            height = height.or(Some(h));
        }
        Ok((width, height))
    }
}
