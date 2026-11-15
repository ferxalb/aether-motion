//! Decoded image cache and `object-fit` painting for `<img>` content.

use crate::model::Color;
use crate::render::check_image;
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::{anyhow, Context, Result};
use image::{imageops, Rgba, RgbaImage};
use std::path::Path;

pub(in crate::render) struct CachedImage {
    pixels: RgbaImage,
    opaque: bool,
}

pub(in crate::render) type ImageKey<'a> = (&'a Path, u32, u32, &'a str);

impl<'a> Renderer<'a> {
    pub(in crate::render) fn cache_image(&mut self, key: &ImageKey<'a>) -> Result<()> {
        let (path, w, h, fit) = *key;
        if !self.images.contains_key(key) {
            let bytes = (w as usize)
                .checked_mul(h as usize)
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| anyhow!("rasterized image is too large"))?;
            const CACHE_LIMIT: usize = 64 * 1024 * 1024;
            if bytes > CACHE_LIMIT {
                return Err(anyhow!("rasterized image exceeds 64 MiB; use --scale"));
            }
            check_image(path)?;
            let image = image::open(path)
                .with_context(|| format!("could not open image {}", path.display()))?
                .to_rgba8();
            let fitted = match fit {
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
                    let resized = imageops::resize(&image, nw, nh, imageops::FilterType::Triangle);
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
            };
            while self.image_cache_bytes + bytes > CACHE_LIMIT {
                if let Some(oldest) = self.image_order.pop_front() {
                    if let Some(old) = self.images.remove(&oldest) {
                        self.image_cache_bytes -= old.pixels.as_raw().len();
                    }
                } else {
                    break;
                }
            }
            self.image_cache_bytes += fitted.as_raw().len();
            self.image_order.push_back(*key);
            let opaque = fitted.pixels().all(|pixel| pixel[3] == 255);
            self.images.insert(
                *key,
                CachedImage {
                    pixels: fitted,
                    opaque,
                },
            );
        }
        Ok(())
    }

    pub(in crate::render) fn draw_image(
        &mut self,
        path: &'a Path,
        rect: Rect,
        fit: &'a str,
        alpha: f32,
    ) -> Result<()> {
        let w = (rect.w * self.scale).round().max(1.0) as u32;
        let h = (rect.h * self.scale).round().max(1.0) as u32;
        let key = (path, w, h, fit);
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
            if image.opaque && alpha >= 1.0 {
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
