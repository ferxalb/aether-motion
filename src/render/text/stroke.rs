//! `-webkit-text-stroke`: glyph outlines (read with `ttf-parser`) stroked with `tiny-skia` into
//! coverage masks at the paint size. The stroke is centered on the outline, as in CSS, and is placed
//! from the same pen and baseline as the fill, so the two line up exactly.

use resvg::tiny_skia::{Mask, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// A stroked glyph: coverage rows and the offset of its top-left pixel from the pen on the baseline
/// (y grows down).
pub(in crate::render) struct StrokeMask {
    pub(in crate::render) coverage: Vec<u8>,
    pub(in crate::render) width: usize,
    pub(in crate::render) height: usize,
    pub(in crate::render) left: i32,
    pub(in crate::render) top: i32,
}

/// Builds a path in pixels (y down, origin at the pen on the baseline).
struct Outline {
    builder: PathBuilder,
    scale: f32,
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(x * self.scale, -y * self.scale);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(x * self.scale, -y * self.scale);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let s = self.scale;
        self.builder.quad_to(x1 * s, -y1 * s, x * s, -y * s);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let s = self.scale;
        self.builder
            .cubic_to(x1 * s, -y1 * s, x2 * s, -y2 * s, x * s, -y * s);
    }
    fn close(&mut self) {
        self.builder.close();
    }
}

/// Strokes `ch` of the face at `size` px with a `width` px line; `None` for glyphs with no outline
/// (spaces) or a zero width.
pub(in crate::render) fn stroke_glyph(
    face: &ttf_parser::Face,
    ch: char,
    size: f32,
    width: f32,
) -> Option<StrokeMask> {
    if width <= 0.0 {
        return None;
    }
    let glyph = face.glyph_index(ch)?;
    let mut outline = Outline {
        builder: PathBuilder::new(),
        scale: size / f32::from(face.units_per_em()),
    };
    face.outline_glyph(glyph, &mut outline)?;
    let path = outline.builder.finish()?;
    let bounds = path.bounds();
    let reach = width / 2.0 + 1.0;
    let left = (bounds.left() - reach).floor() as i32;
    let top = (bounds.top() - reach).floor() as i32;
    let w = ((bounds.right() + reach).ceil() as i32 - left).max(1) as u32;
    let h = ((bounds.bottom() + reach).ceil() as i32 - top).max(1) as u32;
    if u64::from(w) * u64::from(h) > 64 * 1024 * 1024 {
        return None;
    }
    let mut pixmap = Pixmap::new(w, h)?;
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = true;
    let stroke = Stroke {
        width,
        ..Stroke::default()
    };
    let shift = Transform::from_translate(-left as f32, -top as f32);
    pixmap.stroke_path(&path, &paint, &stroke, shift, None::<&Mask>);
    let coverage = pixmap
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| px[3])
        .collect();
    Some(StrokeMask {
        coverage,
        width: w as usize,
        height: h as usize,
        left,
        top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_outline_glyphs_and_skips_spaces() {
        let data = include_bytes!("../../../assets/fonts/Roboto-Bold.ttf");
        let face = ttf_parser::Face::parse(data, 0).unwrap();
        let mask = stroke_glyph(&face, 'O', 100.0, 4.0).unwrap();
        // The stroke rings the letter: the middle of the counter is empty, the edges are covered.
        let at = |x: usize, y: usize| mask.coverage[y * mask.width + x];
        let (cx, cy) = (mask.width / 2, mask.height / 2);
        assert_eq!(at(cx, cy), 0);
        assert!((0..mask.width).any(|x| at(x, cy) == 255));
        assert!(mask.top < 0, "glyphs sit above the baseline");
        assert!(stroke_glyph(&face, ' ', 100.0, 4.0).is_none());
        assert!(stroke_glyph(&face, 'O', 100.0, 0.0).is_none());
    }
}
