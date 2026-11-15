//! The pixel surface everything is painted on: RGBA bytes, the area touched since the last clear,
//! and alpha blending. Rotated subtrees paint on their own temporary `Surface`.

use crate::model::Color;
use crate::render::geometry::PixelRect;

/// An RGBA pixel buffer plus the bookkeeping painting needs.
pub(in crate::render) struct Surface {
    pub(in crate::render) pixels: Vec<u8>,
    pub(in crate::render) width: u32,
    pub(in crate::render) height: u32,
    /// Pixels painted since the last clear, so the next frame only resets what changed.
    pub(in crate::render) painted: Option<PixelRect>,
    /// Color the surface started with; layers use black or white.
    pub(in crate::render) backdrop: Color,
    /// Output pixels per layout unit.
    pub(in crate::render) scale: f32,
    /// Cached vignette brightness table of the frame-wide finishing pass.
    pub(in crate::render) finish_lut: Option<super::finish::FinishLut>,
}

impl Surface {
    /// A zeroed surface; the caller fills it before the first frame.
    pub(in crate::render) fn blank(width: u32, height: u32, scale: f32, backdrop: Color) -> Self {
        Self {
            pixels: vec![0; width as usize * height as usize * 4],
            width,
            height,
            painted: None,
            backdrop,
            scale,
            finish_lut: None,
        }
    }

    /// A surface filled with an opaque `backdrop`.
    pub(in crate::render) fn filled(width: u32, height: u32, scale: f32, backdrop: Color) -> Self {
        let mut surface = Self::blank(width, height, scale, backdrop);
        surface
            .pixels
            .as_chunks_mut::<4>()
            .0
            .fill([backdrop.r, backdrop.g, backdrop.b, 255]);
        surface
    }

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
