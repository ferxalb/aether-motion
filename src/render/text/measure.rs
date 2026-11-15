//! Text measurement in layout units without rasterizing.

impl<'a> Renderer<'a> {
    /// Width and height of one line of text in layout units, without rasterizing.
    pub(in crate::render) fn measure_text(&mut self, text: &str, idx: usize, zoom: f32) -> Result<(f32, f32)> {
        let (family, base, _, _) = self.text_props(idx);
        let scale = self.scale;
        let size = base * scale * zoom;
        let font = self.font(family)?;
        let advance: f32 = text
            .chars()
            .map(|ch| font.metrics(ch, size).advance_width)
            .sum();
        Ok((advance / scale, size / scale))
    }
}
