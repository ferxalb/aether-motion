//! Uniform solid borders drawn inside the box.

impl<'a> Renderer<'a> {
    /// Solid ring between the rounded box and the same box inset by `width`.
    pub(in crate::render) fn draw_border(&mut self, rect: Rect, radius: f32, width: f32, color: Color, alpha: f32) {
        let k = self.scale;
        let outer = [
            rect.x * k,
            rect.y * k,
            (rect.x + rect.w) * k,
            (rect.y + rect.h) * k,
        ];
        let width = (width * k).min((outer[2] - outer[0]).min(outer[3] - outer[1]) / 2.0);
        let inner = [
            outer[0] + width,
            outer[1] + width,
            outer[2] - width,
            outer[3] - width,
        ];
        let (outer_radius, inner_radius) = (radius * k, (radius * k - width).max(0.0));
        let Some(bounds) = Self::clip_pixels(
            (self.width, self.height),
            outer[0].floor() as i32,
            outer[1].floor() as i32,
            outer[2].ceil() as i32,
            outer[3].ceil() as i32,
        ) else {
            return;
        };
        self.mark_painted(bounds);
        let strength = alpha * f32::from(color.a) / 255.0;
        let blend_color = Color { a: 255, ..color };
        let (center_y, half_y) = ((inner[1] + inner[3]) / 2.0, (inner[3] - inner[1]) / 2.0);
        // On straight rows everything well inside the inner box is untouched.
        let inside_from = (inner[0] + 0.5).ceil() as usize;
        let inside_to = (inner[2] - 1.5).floor().max(0.0) as usize;
        let stride = self.width as usize;
        for y in bounds.y0..bounds.y1 {
            let py = y as f32 + 0.5;
            let straight = (py - center_y).abs() <= half_y - inner_radius - 1.0;
            let mut x = bounds.x0;
            while x < bounds.x1 {
                if straight && x >= inside_from && x <= inside_to {
                    x = inside_to + 1;
                    continue;
                }
                let px = x as f32 + 0.5;
                let outside = (0.5 - rounded_distance(px, py, outer, outer_radius)).clamp(0.0, 1.0);
                let hole = (0.5 - rounded_distance(px, py, inner, inner_radius)).clamp(0.0, 1.0);
                let a = strength * outside * (1.0 - hole);
                if a > 0.002 {
                    let offset = (y * stride + x) * 4;
                    Self::blend_pixel(&mut self.pixels[offset..offset + 4], blend_color, a);
                }
                x += 1;
            }
        }
    }
}
