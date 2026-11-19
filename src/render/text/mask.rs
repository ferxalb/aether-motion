//! Coverage-mask sampling for stable subpixel placement and continuous text scaling. Source columns
//! are computed once per paint (they do not depend on the row) with the same arithmetic as before,
//! so the result is unchanged; `tests/` and the unit test below compare it with the per-pixel form.

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
    size: (usize, usize),
    position: (f32, f32),
    (color, alpha): (Color, f32),
    zoom: f32,
) {
    paint_mask_with(
        pixels,
        dimensions,
        painted,
        mask,
        size,
        position,
        (|_, _| color, alpha),
        zoom,
    );
}

/// [`paint_mask`] with a color per destination pixel (gradient-filled text).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_mask_with(
    pixels: &mut [u8],
    dimensions: (u32, u32),
    painted: &mut Option<PixelRect>,
    mask: &[u8],
    (mw, mh): (usize, usize),
    (left, top): (f32, f32),
    (paint, alpha): (impl Fn(usize, usize) -> Color, f32),
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
    // The source column of a destination column does not depend on the row: evaluate it once, with
    // exactly the per-pixel expression, so the coverage is bit-for-bit the same.
    let columns: Vec<(i32, f32)> = (bounds.x0..bounds.x1)
        .map(|x| {
            let sx = (x as f32 + 0.5 - left) / zoom - 0.5;
            (sx.floor() as i32, sx - sx.floor())
        })
        .collect();
    let source_row = |y: i32| -> Option<&[u8]> {
        (y >= 0 && (y as usize) < mh).then(|| &mask[y as usize * mw..(y as usize + 1) * mw])
    };
    let texel = |row: Option<&[u8]>, x: i32| match row {
        Some(row) if x >= 0 && (x as usize) < mw => row[x as usize] as f32,
        _ => 0.0,
    };
    for y in bounds.y0..bounds.y1 {
        let sy = (y as f32 + 0.5 - top) / zoom - 0.5;
        let iy = sy.floor() as i32;
        let fy = sy - sy.floor();
        let (upper, lower) = (source_row(iy), source_row(iy + 1));
        if upper.is_none() && lower.is_none() {
            continue;
        }
        let line = y * dimensions.0 as usize;
        for (x, &(ix, fx)) in (bounds.x0..bounds.x1).zip(&columns) {
            let coverage = (texel(upper, ix) * (1.0 - fx) + texel(upper, ix + 1) * fx) * (1.0 - fy)
                + (texel(lower, ix) * (1.0 - fx) + texel(lower, ix + 1) * fx) * fy;
            if coverage <= 0.0 {
                continue;
            }
            let color = paint(x, y);
            let a = ((alpha * (coverage / 255.0)) * color.a as f32 / 255.0).clamp(0.0, 1.0);
            if a > 0.0 {
                let offset = (line + x) * 4;
                Surface::blend_pixel(&mut pixels[offset..offset + 4], color, a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The per-pixel form `paint_mask_with` replaced.
    #[allow(clippy::too_many_arguments)]
    fn reference(
        pixels: &mut [u8],
        width: u32,
        mask: &[u8],
        (mw, mh): (usize, usize),
        (left, top): (f32, f32),
        color: Color,
        alpha: f32,
        zoom: f32,
    ) {
        let height = (pixels.len() / 4) as u32 / width;
        let Some(bounds) = clip_pixels(
            (width, height),
            (left - zoom).floor() as i32,
            (top - zoom).floor() as i32,
            (left + (mw as f32 + 1.0) * zoom).ceil() as i32,
            (top + (mh as f32 + 1.0) * zoom).ceil() as i32,
        ) else {
            return;
        };
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
                if coverage <= 0.0 {
                    continue;
                }
                let a = ((alpha * (coverage / 255.0)) * color.a as f32 / 255.0).clamp(0.0, 1.0);
                if a > 0.0 {
                    let offset = (y * width as usize + x) * 4;
                    Surface::blend_pixel(&mut pixels[offset..offset + 4], color, a);
                }
            }
        }
    }

    #[test]
    fn column_precomputation_matches_the_per_pixel_form() {
        let (mw, mh) = (23, 11);
        let mask: Vec<u8> = (0..mw * mh).map(|i| ((i * 37) % 256) as u8).collect();
        let color = Color {
            r: 240,
            g: 30,
            b: 120,
            a: 200,
        };
        for (position, zoom) in [
            ((3.0, 2.0), 1.0),
            ((10.37, 4.81), 1.0),
            ((-5.5, -3.25), 1.7),
            ((40.2, 18.9), 0.63),
        ] {
            let mut fast = vec![17_u8; 64 * 32 * 4];
            let mut slow = fast.clone();
            let mut painted = None;
            paint_mask(
                &mut fast,
                (64, 32),
                &mut painted,
                &mask,
                (mw, mh),
                position,
                (color, 0.8),
                zoom,
            );
            reference(&mut slow, 64, &mask, (mw, mh), position, color, 0.8, zoom);
            assert_eq!(fast, slow, "{position:?} at {zoom}");
        }
    }
}
