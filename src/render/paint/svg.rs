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
    // Bound the read itself: the file may grow after the size check, and devices report 0 bytes.
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(
            std::fs::File::open(path)
                .with_context(|| format!("could not read image {}", path.display()))?,
            MAX_SVG_BYTES + 1,
        ),
        &mut bytes,
    )
    .with_context(|| format!("could not read image {}", path.display()))?;
    if bytes.len() as u64 > MAX_SVG_BYTES {
        return Err(anyhow!("SVG exceeds 8 MiB: {}", path.display()));
    }
    Ok(bytes)
}

/// Rasterizes `tree` into a `w` x `h` bitmap using `fit` (`fill`, `contain`, or `cover`).
/// Vectors are drawn at the target resolution, so output stays sharp at any `--scale`.
pub(crate) fn rasterize(tree: &usvg::Tree, w: u32, h: u32, fit: &str) -> Result<RgbaImage> {
    profile::time(Phase::SvgRaster, || {
        rasterize_pixels(tree, w, h, fit, [0, 0, w, h])
    })
}

/// Rasterizes only the `[x, y, width, height]` part (in pixels of the full `w` x `h` fit) of the
/// image [`rasterize`] would produce, for fits too large to hold whole. Content outside the region
/// still contributes to it (filters and layers are bounded by the artwork, not the region).
pub(crate) fn rasterize_region(
    tree: &usvg::Tree,
    w: u32,
    h: u32,
    fit: &str,
    region: [u32; 4],
) -> Result<RgbaImage> {
    profile::time(Phase::SvgRaster, || {
        rasterize_pixels(tree, w, h, fit, region)
    })
}

fn rasterize_pixels(
    tree: &usvg::Tree,
    w: u32,
    h: u32,
    fit: &str,
    [x, y, pw, ph]: [u32; 4],
) -> Result<RgbaImage> {
    let size = tree.size();
    let (sx, sy) = (w as f32 / size.width(), h as f32 / size.height());
    let (sx, sy) = match fit {
        "fill" => (sx, sy),
        "contain" => (sx.min(sy), sx.min(sy)),
        "cover" => (sx.max(sy), sx.max(sy)),
        _ => return Err(anyhow!("unsupported object-fit: {fit}")),
    };
    let tx = (w as f32 - size.width() * sx) / 2.0 - x as f32;
    let ty = (h as f32 - size.height() * sy) / 2.0 - y as f32;
    let mut pixmap =
        tiny_skia::Pixmap::new(pw, ph).ok_or_else(|| anyhow!("SVG raster size is invalid"))?;
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
    RgbaImage::from_raw(pw, ph, data).ok_or_else(|| anyhow!("SVG raster buffer mismatch"))
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

    /// Per-pixel largest premultiplied channel difference between `region` (rasterized alone) and
    /// the same part of the whole `333 x 201` raster.
    fn region_differences(art: &str, fit: &str, [x, y, w, h]: [u32; 4]) -> Vec<i32> {
        let tree = parse_svg(art.as_bytes()).unwrap();
        let whole = rasterize(&tree, 333, 201, fit).unwrap();
        let part = rasterize_region(&tree, 333, 201, fit, [x, y, w, h]).unwrap();
        let crop = image::imageops::crop_imm(&whole, x, y, w, h).to_image();
        // Compare premultiplied: a low-alpha pixel's straight color is coarse.
        let pm = |px: &image::Rgba<u8>, c: usize| {
            if c == 3 {
                i32::from(px[3])
            } else {
                i32::from(px[c]) * i32::from(px[3]) / 255
            }
        };
        part.pixels()
            .zip(crop.pixels())
            .map(|(p, q)| (0..4).map(|c| (pm(p, c) - pm(q, c)).abs()).max().unwrap())
            .collect()
    }

    #[test]
    fn a_region_matches_the_same_part_of_the_whole_raster() {
        const REGION: [u32; 4] = [101, 37, 150, 90];
        // The blurred circle's filter region crosses the region's top edge; every shape is
        // rasterized identically, so the region matches within rounding.
        let filtered = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">
            <defs><filter id="f"><feGaussianBlur stdDeviation="2"/></filter></defs>
            <rect x="40" y="16" width="16" height="10" fill="#4a8"/>
            <circle cx="51" cy="29" r="17" fill="#e84" opacity=".6" filter="url(#f)"/></svg>"##;
        // A hairline that crosses the region's left and right edges is clipped to the region,
        // which re-derives its edge segments; its antialiasing can then move by one step of
        // tiny-skia's 4x4 supersampling (16 of 255; measured: exactly 16 on 4 to 10 pixels).
        let stroke = r##"<path d="M3 50 L97 7" stroke="#123" stroke-width="0.6" fill="none"/>"##;
        let stroked = filtered.replace("<rect", &format!("{stroke}<rect"));
        let stroke_only = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 60">{stroke}</svg>"##
        );
        let [x, y, w, _] = REGION;
        for fit in ["fill", "contain", "cover"] {
            let control = region_differences(filtered, fit, REGION);
            let worst = control.iter().copied().max().unwrap();
            assert!(
                worst <= 1,
                "{fit}: without the clipped stroke, worst {worst}"
            );

            // Differences beyond rounding may only sit under the stroke, by at most one step.
            let tree = parse_svg(stroke_only.as_bytes()).unwrap();
            let line = rasterize(&tree, 333, 201, fit).unwrap();
            let diffs = region_differences(&stroked, fit, REGION);
            for (i, &d) in diffs.iter().enumerate() {
                let (px, py) = (x + i as u32 % w, y + i as u32 / w);
                let under_stroke = line.get_pixel(px, py)[3] > 0;
                assert!(
                    d <= 1 || (under_stroke && d <= 16),
                    "{fit}: ({px}, {py}) differs by {d}, under the stroke: {under_stroke}"
                );
            }
        }
    }

    #[test]
    fn rejects_broken_markup() {
        assert!(parse_svg(b"<svg").is_err());
    }
}
