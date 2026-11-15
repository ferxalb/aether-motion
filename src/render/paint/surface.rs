//! Pixel surface access and alpha blending helpers.

impl<'a> Renderer<'a> {
    pub(in crate::render) fn rows(&mut self, rect: PixelRect) -> impl Iterator<Item = &mut [u8]> {
        let stride = self.width as usize * 4;
        self.pixels[rect.y0 * stride..rect.y1 * stride]
            .chunks_exact_mut(stride)
            .map(move |row| &mut row[rect.x0 * 4..rect.x1 * 4])
    }

    pub(in crate::render) fn mark_painted(&mut self, rect: PixelRect) {
        self.painted = Some(self.painted.map_or(rect, |painted| painted.union(rect)));
    }

    pub(in crate::render) fn blend_at(
        pixels: &mut [u8],
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        color: Color,
        alpha: f32,
    ) {
        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
            return;
        }
        let a = (alpha * color.a as f32 / 255.0).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let offset = (y as usize * width as usize + x as usize) * 4;
        let dest = &mut pixels[offset..offset + 4];
        Self::blend_pixel(dest, color, a);
    }

    #[inline]
    pub(in crate::render) fn blend_pixel(dest: &mut [u8], color: Color, a: f32) {
        dest[0] = (color.r as f32 * a + dest[0] as f32 * (1.0 - a)).round() as u8;
        dest[1] = (color.g as f32 * a + dest[1] as f32 * (1.0 - a)).round() as u8;
        dest[2] = (color.b as f32 * a + dest[2] as f32 * (1.0 - a)).round() as u8;
        dest[3] = 255;
    }

    pub(in crate::render) fn over_background(base: Color, color: Color, alpha: f32) -> [u8; 4] {
        let mut pixel = [base.r, base.g, base.b, 255];
        Self::blend_pixel(&mut pixel, color, alpha);
        pixel
    }
}
