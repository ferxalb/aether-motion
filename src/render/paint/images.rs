//! Decoded and fitted image cache, with exact transparent-row skipping and opaque-span copies.

use crate::model::Color;
use crate::render::check_image;
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::svg;
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::{anyhow, Context, Result};
use image::{imageops, Rgba, RgbaImage};
use std::path::Path;

pub(in crate::render) struct CachedImage {
    pixels: RgbaImage,
    opaque: bool,
    rows: Vec<(u32, u32, bool)>,
}

/// Where a bitmap comes from: a file on disk or inline `<svg>` markup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::render) enum ImageSrc<'a> {
    File(&'a Path),
    Inline(&'a str),
}

pub(in crate::render) type ImageKey<'a> = (ImageSrc<'a>, u32, u32, &'a str);

fn is_svg(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

impl<'a> Renderer<'a> {
    pub(in crate::render) fn cache_image(&mut self, key: &ImageKey<'a>) -> Result<()> {
        let (src, w, h, fit) = *key;
        if !self.images.contains_key(key) {
            let pixel_bytes = (w as usize)
                .checked_mul(h as usize)
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| anyhow!("rasterized image is too large"))?;
            const CACHE_LIMIT: usize = 64 * 1024 * 1024;
            if pixel_bytes > CACHE_LIMIT {
                return Err(anyhow!("rasterized image exceeds 64 MiB; use --scale"));
            }
            // Metadata is optional: a legal image must not become too large merely
            // because row acceleration was added.
            let row_bytes = h as usize * std::mem::size_of::<(u32, u32, bool)>();
            let cache_rows = row_bytes <= CACHE_LIMIT - pixel_bytes;
            let bytes = pixel_bytes + if cache_rows { row_bytes } else { 0 };
            let vector = match src {
                ImageSrc::Inline(markup) => Some(svg::parse_svg(markup.as_bytes())?),
                ImageSrc::File(path) if is_svg(path) => Some(
                    svg::parse_svg(&svg::read_limited(path)?)
                        .with_context(|| format!("in {}", path.display()))?,
                ),
                ImageSrc::File(_) => None,
            };
            let image = match (&vector, src) {
                (Some(_), _) => RgbaImage::new(1, 1),
                (None, ImageSrc::File(path)) => {
                    check_image(path)?;
                    image::open(path)
                        .with_context(|| format!("could not open image {}", path.display()))?
                        .to_rgba8()
                }
                (None, ImageSrc::Inline(_)) => unreachable!("inline sources are always vector"),
            };
            let fitted = if let Some(tree) = &vector {
                svg::rasterize(tree, w, h, fit)?
            } else {
                match fit {
                    "fill" => imageops::resize(&image, w, h, imageops::FilterType::Triangle),
                    "contain" | "cover" => {
                        let sx = w as f32 / image.width() as f32;
                        let sy = h as f32 / image.height() as f32;
                        let factor = if fit == "contain" {
                            sx.min(sy)
                        } else {
                            sx.max(sy)
                        };
                        let nw = ((image.width() as f32 * factor).round() as u32).max(1);
                        let nh = ((image.height() as f32 * factor).round() as u32).max(1);
                        if u64::from(nw) * u64::from(nh) > (CACHE_LIMIT / 4) as u64 {
                            return Err(anyhow!(
                                "intermediate image exceeds 64 MiB; adjust object-fit or --scale"
                            ));
                        }
                        let resized =
                            imageops::resize(&image, nw, nh, imageops::FilterType::Triangle);
                        let mut canvas = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
                        imageops::overlay(
                            &mut canvas,
                            &resized,
                            (w as i64 - nw as i64) / 2,
                            (h as i64 - nh as i64) / 2,
                        );
                        canvas
                    }
                    _ => return Err(anyhow!("unsupported object-fit: {fit}")),
                }
            };
            while bytes > CACHE_LIMIT.saturating_sub(self.image_cache_bytes)
                || self.images.len() >= 4096
            {
                if let Some(oldest) = self.image_order.pop_front() {
                    if let Some(old) = self.images.remove(&oldest) {
                        self.image_cache_bytes -= old.pixels.as_raw().len()
                            + old.rows.len() * std::mem::size_of::<(u32, u32, bool)>();
                    }
                } else {
                    break;
                }
            }
            self.image_cache_bytes += bytes;
            self.image_order.push_back(*key);
            let opaque = fitted.pixels().all(|pixel| pixel[3] == 255);
            let rows = if cache_rows {
                fitted
                    .as_raw()
                    .chunks_exact(w as usize * 4)
                    .map(|row| {
                        let row = row.as_chunks::<4>().0;
                        let first = row.iter().position(|p| p[3] != 0).unwrap_or(0);
                        let last = row.iter().rposition(|p| p[3] != 0).map_or(0, |i| i + 1);
                        (
                            first as u32,
                            last as u32,
                            row[first..last].iter().all(|p| p[3] == 255),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
            self.images.insert(
                *key,
                CachedImage {
                    pixels: fitted,
                    opaque,
                    rows,
                },
            );
        }
        Ok(())
    }

    pub(in crate::render) fn draw_image(
        &mut self,
        src: ImageSrc<'a>,
        rect: Rect,
        fit: &'a str,
        alpha: f32,
    ) -> Result<()> {
        let w = (rect.w * self.scale).round().max(1.0) as u32;
        let h = (rect.h * self.scale).round().max(1.0) as u32;
        let key = (src, w, h, fit);
        self.cache_image(&key)?;
        let x0 = (rect.x * self.scale).round() as i32;
        let y0 = (rect.y * self.scale).round() as i32;
        let Some(bounds) = clip_pixels(
            (self.surface.width, self.surface.height),
            x0,
            y0,
            x0.saturating_add(w as i32),
            y0.saturating_add(h as i32),
        ) else {
            return Ok(());
        };
        self.surface.mark_painted(bounds);
        let image = &self.images[&key];
        let stride = self.surface.width as usize * 4;
        let source_stride = w as usize * 4;
        let source_x = (bounds.x0 as i64 - i64::from(x0)) as usize * 4;
        let source_y = (bounds.y0 as i64 - i64::from(y0)) as usize;
        let row_len = (bounds.x1 - bounds.x0) * 4;
        for (i, row) in self.surface.pixels[bounds.y0 * stride..bounds.y1 * stride]
            .chunks_exact_mut(stride)
            .enumerate()
        {
            let source_offset = (source_y + i) * source_stride + source_x;
            let source = &image.pixels.as_raw()[source_offset..source_offset + row_len];
            let dest = &mut row[bounds.x0 * 4..bounds.x1 * 4];
            let (first, last, row_opaque) =
                image
                    .rows
                    .get(source_y + i)
                    .copied()
                    .unwrap_or((0, w, image.opaque));
            let start = (first as usize * 4).saturating_sub(source_x).min(row_len);
            let end = (last as usize * 4).saturating_sub(source_x).min(row_len);
            if start >= end {
                continue;
            }
            let source = &source[start..end];
            let dest = &mut dest[start..end];
            if (image.opaque && alpha >= 1.0) || (row_opaque && alpha == 1.0) {
                dest.copy_from_slice(source);
            } else {
                for (dest, &[r, g, b, a]) in dest
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(source.as_chunks::<4>().0)
                {
                    if a != 0 {
                        let color = Color { r, g, b, a };
                        Surface::blend_pixel(dest, color, alpha * f32::from(a) / 255.0);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_file;

    #[test]
    fn tiny_images_cannot_grow_metadata_past_the_entry_limit() {
        let stem =
            std::env::temp_dir().join(format!("aether-image-cache-limit-{}", std::process::id()));
        let path = stem.with_extension("png");
        let html = stem.with_extension("html");
        RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 255]))
            .save(&path)
            .unwrap();
        std::fs::write(&html,r#"<main data-composition-id="test" data-width="8" data-height="8" data-duration="1"></main>"#).unwrap();
        let composition = parse_file(&html).unwrap();
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        for width in 1..=4096 {
            let key = (ImageSrc::File(path.as_path()), width, 1, "fill");
            renderer.images.insert(
                key,
                CachedImage {
                    pixels: RgbaImage::new(1, 1),
                    opaque: true,
                    rows: Vec::new(),
                },
            );
            renderer.image_order.push_back(key);
            renderer.image_cache_bytes += 4;
        }
        let key = (ImageSrc::File(path.as_path()), 4097, 1, "fill");
        renderer.cache_image(&key).unwrap();
        assert_eq!(renderer.images.len(), 4096);
        assert_eq!(renderer.image_order.len(), 4096);
        assert!(!renderer
            .images
            .contains_key(&(ImageSrc::File(path.as_path()), 1, 1, "fill")));
        assert!(renderer.images.contains_key(&key));
        assert_eq!(
            renderer.image_cache_bytes,
            4095 * 4 + 4097 * 4 + std::mem::size_of::<(u32, u32, bool)>()
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(html).unwrap();
    }
}
