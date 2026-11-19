//! Deterministic SVG rasterization (file or inline markup) into an RGBA bitmap with `resvg`.
//!
//! Scope: static vector graphics (shapes, paths, gradients, clip paths, masks, filters, strokes).
//! Text, external images, and scripts inside an SVG are not rendered; the document is parsed with
//! no font database and no resource resolver, so output depends only on the markup and the size.

use crate::profile::{self, Phase};
use anyhow::{anyhow, Context, Result};
use image::RgbaImage;
use resvg::{tiny_skia, usvg};

/// Largest SVG source accepted, to bound parse time and memory.
const MAX_SVG_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn svg_options() -> usvg::Options<'static> {
    // Network and file access from inside an SVG would break determinism.
    usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    }
}

pub(crate) fn parse_svg(data: &[u8]) -> Result<usvg::Tree> {
    if data.len() as u64 > MAX_SVG_BYTES {
        return Err(anyhow!("SVG exceeds 8 MiB; simplify it"));
    }
    profile::time(Phase::SvgParse, || {
        usvg::Tree::from_data(data, &svg_options()).map_err(|e| anyhow!("invalid SVG: {e}"))
    })
}

/// Validates an SVG file and returns its intrinsic size.
pub(crate) fn check_svg_file(path: &std::path::Path) -> Result<(f32, f32)> {
    let data = read_limited(path)?;
    let tree = parse_svg(&data).with_context(|| format!("in {}", path.display()))?;
    Ok((tree.size().width(), tree.size().height()))
}

pub(crate) fn read_limited(path: &std::path::Path) -> Result<Vec<u8>> {
    let len = std::fs::metadata(path)
        .with_context(|| format!("could not read image {}", path.display()))?
        .len();
    if len > MAX_SVG_BYTES {
        return Err(anyhow!("SVG exceeds 8 MiB: {}", path.display()));
    }
    std::fs::read(path).with_context(|| format!("could not read image {}", path.display()))
}

/// Rasterizes `tree` into a `w` x `h` bitmap using `fit` (`fill`, `contain`, or `cover`).
/// Vectors are drawn at the target resolution, so output stays sharp at any `--scale`.
pub(crate) fn rasterize(tree: &usvg::Tree, w: u32, h: u32, fit: &str) -> Result<RgbaImage> {
    profile::time(Phase::SvgRaster, || rasterize_pixels(tree, w, h, fit))
}

fn rasterize_pixels(tree: &usvg::Tree, w: u32, h: u32, fit: &str) -> Result<RgbaImage> {
    let size = tree.size();
    let (sx, sy) = (w as f32 / size.width(), h as f32 / size.height());
    let (sx, sy) = match fit {
        "fill" => (sx, sy),
        "contain" => (sx.min(sy), sx.min(sy)),
        "cover" => (sx.max(sy), sx.max(sy)),
        _ => return Err(anyhow!("unsupported object-fit: {fit}")),
    };
    let tx = (w as f32 - size.width() * sx) / 2.0;
    let ty = (h as f32 - size.height() * sy) / 2.0;
    let mut pixmap =
        tiny_skia::Pixmap::new(w, h).ok_or_else(|| anyhow!("SVG raster size is invalid"))?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_row(sx, 0.0, 0.0, sy, tx, ty),
        &mut pixmap.as_mut(),
    );
    // tiny-skia stores premultiplied alpha; the image cache expects straight RGBA.
    let mut data = pixmap.take();
    for px in data.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(w, h, data).ok_or_else(|| anyhow!("SVG raster buffer mismatch"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="red"/></svg>"##;

    #[test]
    fn rasterizes_deterministically_and_honors_fit() {
        let tree = parse_svg(RED.as_bytes()).unwrap();
        let a = rasterize(&tree, 20, 10, "fill").unwrap();
        assert_eq!(a.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(a, rasterize(&tree, 20, 10, "fill").unwrap());
        let c = rasterize(&tree, 20, 10, "contain").unwrap();
        assert_eq!(c.get_pixel(0, 5).0[3], 0);
        assert_eq!(c.get_pixel(10, 5).0, [255, 0, 0, 255]);
    }

    #[test]
    fn rejects_broken_markup() {
        assert!(parse_svg(b"<svg").is_err());
    }
}
