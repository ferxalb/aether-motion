//! Coverage-mask sampling for stable subpixel placement and continuous text scaling.

use crate::model::Color;
use crate::render::geometry::{clip_pixels, PixelRect};
use crate::render::paint::Surface;

/// Samples a fixed raster with bilinear coverage and a transparent border.
/// Blend each destination once, so translucent glyph edges do not accumulate opacity.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_mask(
    pixels: &mut [u8],
    dimensions: (u32, u32),
    painted: &mut Option<PixelRect>,
    mask: &[u8],
    (mw, mh): (usize, usize),
    (left, top): (f32, f32),
    (color, alpha): (Color, f32),
    zoom: f32,
) {
    if mw == 0 || mh == 0 || alpha <= 0.0 {
        return;
    }
    let Some(bounds) = clip_pixels(
        dimensions,
        (left - zoom).floor() as i32,
        (top - zoom).floor() as i32,
        (left + (mw as f32 + 1.0) * zoom).ceil() as i32,
        (top + (mh as f32 + 1.0) * zoom).ceil() as i32,
    ) else {
        return;
    };
    *painted = Some(painted.map_or(bounds, |area| area.union(bounds)));
    let texel = |x: i32, y: i32| {
        if x < 0 || y < 0 || x as usize >= mw || y as usize >= mh {
            0.0
        } else {
            mask[y as usize * mw + x as usize] as f32
        }
    };
    for y in bounds.y0..bounds.y1 {
        let sy = (y as f32 + 0.5 - top) / zoom - 0.5;
        let iy = sy.floor() as i32;
        let fy = sy - sy.floor();
        for x in bounds.x0..bounds.x1 {
            let sx = (x as f32 + 0.5 - left) / zoom - 0.5;
            let ix = sx.floor() as i32;
            let fx = sx - sx.floor();
            let coverage = (texel(ix, iy) * (1.0 - fx) + texel(ix + 1, iy) * fx) * (1.0 - fy)
                + (texel(ix, iy + 1) * (1.0 - fx) + texel(ix + 1, iy + 1) * fx) * fy;
            let a = ((alpha * (coverage / 255.0)) * color.a as f32 / 255.0).clamp(0.0, 1.0);
            if a > 0.0 {
                let offset = (y * dimensions.0 as usize + x) * 4;
                Surface::blend_pixel(&mut pixels[offset..offset + 4], color, a);
            }
        }
    }
}
