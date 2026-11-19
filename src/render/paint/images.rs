//! Decoded and fitted image cache, with exact transparent-row skipping and opaque-span copies.
//!
//! One budget (`CACHE_LIMIT` per renderer) holds exact fits and, while vector artwork zooms, the
//! ladder levels and resampled fits of `svg_levels.rs` (keyed by [`Fitting`], so cached and uncached
//! draws give the same pixels). Vector entries are LZ4-packed under pressure (`packed_image.rs`).

use super::packed_image::{PackedImage, BAND_BYTES};
use crate::model::Color;
use crate::profile::{self, ImageMetric, Phase};
use crate::render::check_image;
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::{svg, svg_levels};
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::{anyhow, Context, Result};
use image::{imageops, Rgba, RgbaImage};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

const CACHE_LIMIT: usize = 64 * 1024 * 1024;

pub(in crate::render) struct CachedImage {
    pixels: ImagePixels,
    opaque: bool,
    rows: Vec<(u32, u32, bool)>,
}

enum ImagePixels {
    Raw(RgbaImage),
    Packed(PackedImage),
}

impl CachedImage {
    fn bytes(&self) -> usize {
        let bytes = match &self.pixels {
            ImagePixels::Raw(pixels) => pixels.as_raw().len(),
            ImagePixels::Packed(pixels) => pixels.bytes,
        };
        bytes + self.rows.capacity() * size_of::<(u32, u32, bool)>()
    }

    fn pack(&mut self) {
        if let ImagePixels::Raw(raw) = &self.pixels {
            if raw.as_raw().len() >= 512 * 1024 {
                if let Some(packed) = profile::time(Phase::ImagePack, || {
                    PackedImage::pack(raw.as_raw(), raw.width() as usize * 4)
                }) {
                    profile::image(ImageMetric::Packed, 1);
                    self.pixels = ImagePixels::Packed(packed);
                }
            }
        }
    }
}

/// Where a bitmap comes from: a file on disk or inline `<svg>` markup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::render) enum ImageSrc<'a> {
    File(&'a Path),
    Sequence(&'a Path),
    Inline(&'a str),
}

/// How a cached bitmap was made. It is part of the key, so the pixels drawn never depend on which
/// entries happen to be cached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::render) enum Fitting {
    /// Rasterized, or fitted from the decoded source, directly at the key's size.
    Exact,
    /// Resampled down from the exact raster of the same source at this ladder-level size
    /// (`svg_levels.rs`), for vector artwork whose scale is animating.
    Level(u32, u32),
}

pub(in crate::render) type ImageKey<'a> = (ImageSrc<'a>, u32, u32, &'a str, Fitting);

/// What `cache_image` produced: an entry now in the cache, or a resampled bitmap that is drawn
/// once because keeping it would have evicted an exact raster.
enum Cached {
    Stored,
    Transient(CachedImage),
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

impl ImageSrc<'_> {
    fn is_vector(self) -> bool {
        match self {
            Self::Inline(_) => true,
            Self::File(path) => is_svg(path),
            Self::Sequence(_) => false,
        }
    }
}

fn pack_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AETHER_IMAGE_PACK").as_deref() != Ok("off"))
}

fn raster_bytes(w: u32, h: u32) -> Result<usize> {
    (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| anyhow!("rasterized image is too large"))
}

impl<'a> Renderer<'a> {
    fn cache_image(&mut self, key: &ImageKey<'a>) -> Result<Cached> {
        if self.images.contains_key(key) {
            profile::image(ImageMetric::Hit, 1);
            return Ok(Cached::Stored);
        }
        profile::image(ImageMetric::Miss, 1);
        let (src, w, h, fit, fitting) = *key;
        let pixel_bytes = raster_bytes(w, h)?;
        if pixel_bytes > CACHE_LIMIT {
            return Err(anyhow!("rasterized image exceeds 64 MiB; use --scale"));
        }
        let row_bytes = h as usize * size_of::<(u32, u32, bool)>();
        let cache_rows = row_bytes <= CACHE_LIMIT - pixel_bytes;
        let requested = pixel_bytes + if cache_rows { row_bytes } else { 0 };
        if requested > CACHE_LIMIT - self.image_cache_bytes
            && !self.pack_svg_images
            && pack_enabled()
        {
            self.pack_svg_images = true;
            // Compact surviving vectors before allocating the next potentially large raster.
            for key in &self.image_order {
                if key.0.is_vector() {
                    let image = self
                        .images
                        .get_mut(key)
                        .expect("cache order tracks entries");
                    let old_bytes = image.bytes();
                    image.pack();
                    self.image_cache_bytes -= old_bytes - image.bytes();
                }
            }
        }
        let fitted = match fitting {
            Fitting::Level(lw, lh) => {
                let level = (src, lw, lh, fit, Fitting::Exact);
                match self.cache_image(&level)? {
                    Cached::Stored => {
                        let level = &self.images[&level];
                        resample_cached(level, [lw, lh], [w, h], &mut self.image_band)
                    }
                    Cached::Transient(level) => {
                        resample_cached(&level, [lw, lh], [w, h], &mut self.image_band)
                    }
                }
            }
            Fitting::Exact => {
                profile::image(ImageMetric::LargestRasterBytes, pixel_bytes);
                profile::image(ImageMetric::RasterBytes, pixel_bytes);
                match src {
                    ImageSrc::Inline(markup) => {
                        svg::rasterize(&svg::parse_svg(markup.as_bytes())?, w, h, fit)?
                    }
                    ImageSrc::File(path) if is_svg(path) => {
                        let tree = svg::parse_svg(&svg::read_limited(path)?)
                            .with_context(|| format!("in {}", path.display()))?;
                        svg::rasterize(&tree, w, h, fit)?
                    }
                    ImageSrc::File(path) => fit_pixels(&decode(path)?, w, h, fit)?,
                    ImageSrc::Sequence(path) => {
                        let source = self.sequence_sources.get(path)?;
                        fit_with(source, w, h, fit, resize_sequence)?
                    }
                }
            }
        };
        let resampled = matches!(fitting, Fitting::Level(..));
        let mut image = CachedImage::new(fitted, cache_rows);
        // Resampled fits only live in spare budget for a frame or two: packing them costs more
        // than it saves.
        if self.pack_svg_images && src.is_vector() && !resampled {
            image.pack();
        }
        let bytes = image.bytes();
        // Resampled bitmaps are cheap to remake and go first; they never push out an exact raster
        // (a ladder level is worth several frames of resampling), so they are drawn once instead.
        while bytes > CACHE_LIMIT - self.image_cache_bytes || self.images.len() >= 4096 {
            let oldest = match self.resampled_order.pop_front() {
                Some(oldest) => oldest,
                None if resampled => return Ok(Cached::Transient(image)),
                None => self.image_order.pop_front().expect("new image fits alone"),
            };
            self.image_cache_bytes -= self
                .images
                .remove(&oldest)
                .expect("cache order tracks entries")
                .bytes();
            profile::image(ImageMetric::Eviction, 1);
        }
        self.image_cache_bytes += bytes;
        profile::image(ImageMetric::PeakCacheBytes, self.image_cache_bytes);
        if resampled {
            self.resampled_order.push_back(*key);
        } else {
            self.image_order.push_back(*key);
        }
        self.images.insert(*key, image);
        Ok(Cached::Stored)
    }

    /// Draws a fitted image. `zoom` is the element's combined CSS scale while it (or an ancestor) is
    /// changing scale: vector artwork then reuses a ladder-level raster resampled to this frame's
    /// size instead of being rasterized again at every passing size. At rest, or with `None`, the
    /// image is fitted exactly, as before. Vector fits too large to cache are rasterized for their
    /// visible part only.
    pub(in crate::render) fn draw_image(
        &mut self,
        src: ImageSrc<'a>,
        rect: Rect,
        fit: &'a str,
        alpha: f32,
        zoom: Option<f32>,
    ) -> Result<()> {
        let w = (rect.w * self.scale).round().max(1.0) as u32;
        let h = (rect.h * self.scale).round().max(1.0) as u32;
        let x0 = (rect.x * self.scale).round() as i32;
        let y0 = (rect.y * self.scale).round() as i32;
        if src.is_vector() && raster_bytes(w, h)? > CACHE_LIMIT {
            return self.draw_visible_svg(src, [x0, y0], [w, h], fit, alpha);
        }
        let fitting = match zoom.filter(|_| src.is_vector()) {
            Some(scale) => {
                let size = [rect.w * self.scale, rect.h * self.scale];
                let [lw, lh] = svg_levels::level_size(size, scale, [w, h]);
                if [lw, lh] != [w, h] && raster_bytes(lw, lh)? <= CACHE_LIMIT {
                    Fitting::Level(lw, lh)
                } else {
                    Fitting::Exact
                }
            }
            None => Fitting::Exact,
        };
        let key = (src, w, h, fit, fitting);
        let transient = match self.cache_image(&key)? {
            Cached::Stored => None,
            Cached::Transient(image) => Some(image),
        };
        let image = transient.as_ref().unwrap_or_else(|| &self.images[&key]);
        blit(
            &mut self.surface,
            image,
            [x0, y0],
            [w, h],
            alpha,
            &mut self.image_band,
        );
        Ok(())
    }

    /// Rasterizes and draws only the part of a `w` x `h` vector fit at `origin` that falls on the
    /// surface, for fits beyond the cache's per-image limit (deep zooms at 4K). The part is drawn
    /// at the exact fit, so it matches the whole raster; it is not cached.
    fn draw_visible_svg(
        &mut self,
        src: ImageSrc<'a>,
        [x0, y0]: [i32; 2],
        [w, h]: [u32; 2],
        fit: &'a str,
        alpha: f32,
    ) -> Result<()> {
        let Some(bounds) = clip_pixels(
            (self.surface.width, self.surface.height),
            x0,
            y0,
            x0.saturating_add(w as i32),
            y0.saturating_add(h as i32),
        ) else {
            return Ok(());
        };
        let region = [
            (bounds.x0 as i64 - i64::from(x0)) as u32,
            (bounds.y0 as i64 - i64::from(y0)) as u32,
            (bounds.x1 - bounds.x0) as u32,
            (bounds.y1 - bounds.y0) as u32,
        ];
        let bytes = raster_bytes(region[2], region[3])?;
        if bytes > CACHE_LIMIT {
            return Err(anyhow!(
                "visible part of a rasterized image exceeds 64 MiB; use --scale"
            ));
        }
        profile::image(ImageMetric::Miss, 1);
        profile::image(ImageMetric::Visible, 1);
        profile::image(ImageMetric::RasterBytes, bytes);
        let tree = match src {
            ImageSrc::Inline(markup) => svg::parse_svg(markup.as_bytes())?,
            ImageSrc::File(path) => svg::parse_svg(&svg::read_limited(path)?)
                .with_context(|| format!("in {}", path.display()))?,
            ImageSrc::Sequence(_) => unreachable!("image sequences are raster images"),
        };
        let pixels = svg::rasterize_region(&tree, w, h, fit, region)?;
        let image = CachedImage::new(pixels, true);
        blit(
            &mut self.surface,
            &image,
            [bounds.x0 as i32, bounds.y0 as i32],
            [region[2], region[3]],
            alpha,
            &mut self.image_band,
        );
        Ok(())
    }
}

/// Resamples a cached ladder level (raw or packed) to `target`, reading its rows in order.
fn resample_cached(
    level: &CachedImage,
    [lw, lh]: [u32; 2],
    target: [u32; 2],
    scratch: &mut Vec<u8>,
) -> RgbaImage {
    profile::time(Phase::ImageResample, || {
        svg_levels::resample_from_level([lw, lh], target, |row| {
            level.for_each_row([lw, lh], scratch, row)
        })
    })
}

impl CachedImage {
    /// Wraps fitted pixels, with per-row coverage spans when `cache_rows` is set.
    fn new(fitted: RgbaImage, cache_rows: bool) -> Self {
        let w = fitted.width() as usize;
        let opaque = fitted.pixels().all(|pixel| pixel[3] == 255);
        let rows = if cache_rows {
            fitted
                .as_raw()
                .chunks_exact(w * 4)
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
        Self {
            pixels: ImagePixels::Raw(fitted),
            opaque,
            rows,
        }
    }

    /// Calls `f` with every row of a `w` x `h` image, top to bottom, and the span of pixels that
    /// may be visible in it (packed bands are unpacked into `scratch` one at a time).
    fn for_each_row(
        &self,
        [w, h]: [u32; 2],
        scratch: &mut Vec<u8>,
        f: &mut dyn FnMut(usize, &[u8], (usize, usize)),
    ) {
        let stride = w as usize * 4;
        let span = |y: usize| {
            self.rows
                .get(y)
                .map_or((0, w as usize), |&(first, last, _)| (first as usize, last as usize))
        };
        match &self.pixels {
            ImagePixels::Raw(raw) => {
                for (y, row) in raw.as_raw().chunks_exact(stride).enumerate() {
                    f(y, row, span(y));
                }
            }
            ImagePixels::Packed(packed) => {
                if scratch.len() < BAND_BYTES {
                    scratch.resize(BAND_BYTES, 0);
                }
                let per_band = packed.rows_per_band;
                for band in 0..(h as usize).div_ceil(per_band) {
                    let first = band * per_band;
                    let count = (h as usize - first).min(per_band);
                    let pixels = &mut scratch[..count * stride];
                    packed.unpack(band, pixels);
                    for (i, row) in pixels.chunks_exact(stride).enumerate() {
                        f(first + i, row, span(first + i));
                    }
                }
            }
        }
    }
}

impl<'a> Renderer<'a> {
    /// Draws an inline `<svg>` with `data-hf-draw`: while strokes are still drawing, the markup for
    /// this instant is rasterized directly (it changes every frame); afterwards the finished
    /// drawing goes through the image cache (and its zoom ladder while `zoom` is set).
    pub(in crate::render) fn draw_drawn_svg(
        &mut self,
        idx: usize,
        drawing: &'a crate::vector::DrawOnSvg,
        rect: Rect,
        fit: &'a str,
        alpha: f32,
        zoom: Option<f32>,
    ) -> Result<()> {
        let elapsed = self.time - self.composition.elements[idx].start;
        let Some(markup) = drawing.markup_at(elapsed) else {
            return self.draw_image(ImageSrc::Inline(&drawing.markup), rect, fit, alpha, zoom);
        };
        let w = (rect.w * self.scale).round().max(1.0) as u32;
        let h = (rect.h * self.scale).round().max(1.0) as u32;
        if raster_bytes(w, h)? > CACHE_LIMIT {
            return Err(anyhow!("rasterized image exceeds 64 MiB; use --scale"));
        }
        let tree = svg::parse_svg(markup.as_bytes()).context("data-hf-draw")?;
        let image = CachedImage::new(svg::rasterize(&tree, w, h, fit)?, false);
        let x0 = (rect.x * self.scale).round() as i32;
        let y0 = (rect.y * self.scale).round() as i32;
        blit(
            &mut self.surface,
            &image,
            [x0, y0],
            [w, h],
            alpha,
            &mut self.image_band,
        );
        Ok(())
    }
}

/// Decoded cels share a separate FIFO budget so zoom-driven fits can reuse their sources.
#[derive(Default)]
pub(in crate::render) struct SequenceSources<'a> {
    images: HashMap<&'a Path, RgbaImage>,
    order: VecDeque<&'a Path>,
    bytes: usize,
}

impl<'a> SequenceSources<'a> {
    fn get(&mut self, path: &'a Path) -> Result<&RgbaImage> {
        if !self.images.contains_key(path) {
            let decoded = decode(path)?;
            let bytes = decoded.as_raw().len();
            while self.bytes + bytes > CACHE_LIMIT || self.images.len() >= 4096 {
                let oldest = self
                    .order
                    .pop_front()
                    .expect("decoded cel fits in empty cache");
                self.bytes -= self
                    .images
                    .remove(oldest)
                    .expect("cached source")
                    .as_raw()
                    .len();
            }
            self.bytes += bytes;
            self.images.insert(path, decoded);
            self.order.push_back(path);
        }
        Ok(&self.images[path])
    }
}

fn decode(path: &Path) -> Result<RgbaImage> {
    check_image(path)?;
    Ok(image::open(path)
        .with_context(|| format!("could not open image {}", path.display()))?
        .to_rgba8())
}

/// Filter premultiplied samples at 16-bit precision so transparent RGB cannot tint the edges.
fn resize_sequence(source: &RgbaImage, w: u32, h: u32) -> Result<RgbaImage> {
    if source.dimensions() == (w, h) {
        return Ok(source.clone());
    }
    const WORK_LIMIT: u64 = 128 * 1024 * 1024;
    if u64::from(source.width()) * u64::from(h) > WORK_LIMIT / 16 {
        return Err(anyhow!(
            "sequence resize scratch exceeds 128 MiB; use --scale or resize the source"
        ));
    }
    let premultiplied = image::ImageBuffer::from_fn(source.width(), source.height(), |x, y| {
        let [r, g, b, a] = source.get_pixel(x, y).0;
        let alpha = u16::from(a);
        Rgba([
            u16::from(r) * alpha,
            u16::from(g) * alpha,
            u16::from(b) * alpha,
            alpha * 255,
        ])
    });
    let resized = imageops::resize(&premultiplied, w, h, imageops::FilterType::Triangle);
    Ok(RgbaImage::from_fn(w, h, |x, y| {
        let [r, g, b, a] = resized.get_pixel(x, y).0.map(u32::from);
        let alpha = ((a + 127) / 255) as u8;
        if alpha == 0 {
            return Rgba([0; 4]);
        }
        let straight = |value: u32| ((value * 255 + a / 2) / a).min(255) as u8;
        Rgba([straight(r), straight(g), straight(b), alpha])
    }))
}

/// Resizes `image` to a `w` x `h` canvas following `object-fit` (`fill`, `contain`, or `cover`).
pub(in crate::render) fn fit_pixels(
    image: &RgbaImage,
    w: u32,
    h: u32,
    fit: &str,
) -> Result<RgbaImage> {
    fit_with(image, w, h, fit, |source, w, h| {
        Ok(imageops::resize(
            source,
            w,
            h,
            imageops::FilterType::Triangle,
        ))
    })
}

/// [`fit_pixels`] with a caller-chosen resize (video frames use a faster one).
pub(in crate::render) fn fit_with(
    image: &RgbaImage,
    w: u32,
    h: u32,
    fit: &str,
    resize: impl Fn(&RgbaImage, u32, u32) -> Result<RgbaImage>,
) -> Result<RgbaImage> {
    if !matches!(fit, "fill" | "contain" | "cover") {
        return Err(anyhow!("unsupported object-fit: {fit}"));
    }
    profile::time(Phase::ImageFit, || {
        Ok(match fit {
            "fill" => resize(image, w, h)?,
            _ => {
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
                let resized = resize(image, nw, nh)?;
                let mut canvas = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
                imageops::overlay(
                    &mut canvas,
                    &resized,
                    (w as i64 - nw as i64) / 2,
                    (h as i64 - nh as i64) / 2,
                );
                canvas
            }
        })
    })
}

impl CachedImage {
    /// Wraps a fitted video frame with no per-row metadata. `opaque` is false when `object-fit:
    /// contain` left transparent bars around the picture.
    pub(in crate::render) fn frame(pixels: RgbaImage, opaque: bool) -> Self {
        Self {
            pixels: ImagePixels::Raw(pixels),
            opaque,
            rows: Vec::new(),
        }
    }
}

/// Paints a fitted `w` x `h` image with its top-left corner at `(x0, y0)`, clipped to the surface.
pub(in crate::render) fn blit(
    surface: &mut Surface,
    image: &CachedImage,
    origin: [i32; 2],
    size: [u32; 2],
    alpha: f32,
    scratch: &mut Vec<u8>,
) {
    profile::time(Phase::ImageBlit, || {
        blit_pixels(surface, image, origin, size, alpha, scratch);
    });
}

fn blit_pixels(
    surface: &mut Surface,
    image: &CachedImage,
    [x0, y0]: [i32; 2],
    [w, h]: [u32; 2],
    alpha: f32,
    scratch: &mut Vec<u8>,
) {
    let Some(bounds) = clip_pixels(
        (surface.width, surface.height),
        x0,
        y0,
        x0.saturating_add(w as i32),
        y0.saturating_add(h as i32),
    ) else {
        return;
    };
    surface.mark_painted(bounds);
    let stride = surface.width as usize * 4;
    let source_stride = w as usize * 4;
    let source_x = (bounds.x0 as i64 - i64::from(x0)) as usize * 4;
    let source_y = (bounds.y0 as i64 - i64::from(y0)) as usize;
    let row_len = (bounds.x1 - bounds.x0) * 4;
    let mut loaded_band = None;
    for (i, row) in surface.pixels[bounds.y0 * stride..bounds.y1 * stride]
        .chunks_exact_mut(stride)
        .enumerate()
    {
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
        let y = source_y + i;
        let (pixels, local_y) = match &image.pixels {
            ImagePixels::Raw(raw) => (raw.as_raw().as_slice(), y),
            ImagePixels::Packed(packed) => {
                let band = y / packed.rows_per_band;
                if loaded_band != Some(band) {
                    let first = band * packed.rows_per_band;
                    let count = (h as usize - first).min(packed.rows_per_band);
                    if scratch.is_empty() {
                        scratch.resize(BAND_BYTES, 0);
                    }
                    packed.unpack(band, &mut scratch[..count * source_stride]);
                    loaded_band = Some(band);
                }
                (scratch.as_slice(), y % packed.rows_per_band)
            }
        };
        let offset = local_y * source_stride + source_x;
        let source = &pixels[offset + start..offset + end];
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_image_blit_matches_raw_with_clipping_and_alpha() {
        let size = [700, 240];
        let pixels = RgbaImage::from_fn(size[0], size[1], |x, y| {
            let a = [0, 85, 255][(x as usize / 54 + y as usize / 21) % 3];
            Rgba([120, 224, 78, a])
        });
        let raw = CachedImage::frame(pixels.clone(), false);
        let mut packed = CachedImage::frame(pixels, false);
        packed.pack();
        assert!(matches!(packed.pixels, ImagePixels::Packed(_)));
        let mut scratch = Vec::new();
        for alpha in [0.34, 1.0] {
            let mut expected = Surface::filled(400, 200, 1.0, Color::BLACK);
            let mut actual = Surface::filled(400, 200, 1.0, Color::BLACK);
            blit(&mut expected, &raw, [-157, -41], size, alpha, &mut scratch);
            blit(&mut actual, &packed, [-157, -41], size, alpha, &mut scratch);
            assert_eq!(actual.pixels, expected.pixels);
        }
    }
}
