use crate::model::{Color, Composition, Content, Element};
use anyhow::{anyhow, Context, Result};
use fontdb::{Database, Family, Query};
use fontdue::{Font, FontSettings};
use image::{imageops, Rgba, RgbaImage};
use std::collections::{hash_map::DefaultHasher, HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Default, Debug)]
pub struct RenderStats {
    pub rasterized: usize,
    pub reused: usize,
}

pub struct Renderer<'a> {
    composition: &'a Composition,
    width: u32,
    height: u32,
    scale: f32,
    pixels: Vec<u8>,
    last_signature: Option<u64>,
    images: HashMap<(PathBuf, u32, u32, String), RgbaImage>,
    image_order: VecDeque<(PathBuf, u32, u32, String)>,
    image_cache_bytes: usize,
    fonts: HashMap<String, Font>,
    font_db: Database,
    stats: RenderStats,
}

impl<'a> Renderer<'a> {
    pub fn new(composition: &'a Composition, scale: f32) -> Result<Self> {
        if !scale.is_finite() || scale <= 0.0 || scale > 4.0 {
            return Err(anyhow!("--scale debe estar entre 0 y 4"));
        }
        let width = (composition.width as f32 * scale).round() as u32;
        let height = (composition.height as f32 * scale).round() as u32;
        if width == 0 || height == 0 {
            return Err(anyhow!("escala demasiado pequeña"));
        }
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| anyhow!("dimensiones demasiado grandes"))?;
        if size > 128 * 1024 * 1024 {
            return Err(anyhow!("frame requiere más de 128 MiB; usa --scale"));
        }
        let mut font_db = Database::new();
        if composition
            .elements
            .iter()
            .any(|e| matches!(e.content, Content::Text(_)))
        {
            font_db.load_system_fonts();
        }
        Ok(Self {
            composition,
            width,
            height,
            scale,
            pixels: vec![0; size],
            last_signature: None,
            images: HashMap::new(),
            image_order: VecDeque::new(),
            image_cache_bytes: 0,
            fonts: HashMap::new(),
            font_db,
            stats: RenderStats::default(),
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn stats(&self) -> &RenderStats {
        &self.stats
    }
    pub fn frame_count(&self, fps: u32) -> usize {
        (self.composition.duration * fps as f64).ceil() as usize
    }

    fn active_opacity(element: &Element, time: f64) -> f32 {
        if time + 1e-10 < element.start || time >= element.end - 1e-10 {
            return 0.0;
        }
        let mut alpha = element.style.opacity.unwrap_or(1.0);
        if element.fade_in > 0.0 {
            alpha *= ((time - element.start) / element.fade_in).clamp(0.0, 1.0) as f32;
        }
        if element.fade_out > 0.0 {
            alpha *= ((element.end - time) / element.fade_out).clamp(0.0, 1.0) as f32;
        }
        alpha
    }

    fn signature(&self, time: f64) -> u64 {
        let mut hash = DefaultHasher::new();
        for (idx, element) in self.composition.elements.iter().enumerate() {
            let alpha = Self::active_opacity(element, time);
            if alpha > 0.0 {
                idx.hash(&mut hash);
                alpha.to_bits().hash(&mut hash);
            }
        }
        hash.finish()
    }

    pub fn render_frame(&mut self, frame: usize, fps: u32) -> Result<&[u8]> {
        if fps == 0 {
            return Err(anyhow!("fps debe ser mayor que cero"));
        }
        if frame >= self.frame_count(fps) {
            return Err(anyhow!("frame fuera de la duración"));
        }
        let time = frame as f64 / fps as f64;
        let signature = self.signature(time);
        if self.last_signature == Some(signature) {
            self.stats.reused += 1;
            return Ok(&self.pixels);
        }
        self.last_signature = None;
        self.clear(self.composition.background);
        let root = Rect {
            x: 0.0,
            y: 0.0,
            w: self.composition.width as f32,
            h: self.composition.height as f32,
        };
        self.draw_node(0, root, 1.0, time, None)?;
        self.last_signature = Some(signature);
        self.stats.rasterized += 1;
        Ok(&self.pixels)
    }

    fn clear(&mut self, color: Color) {
        for pixel in self.pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[color.r, color.g, color.b, 255]);
        }
    }

    fn draw_node(
        &mut self,
        idx: usize,
        parent: Rect,
        parent_alpha: f32,
        time: f64,
        parent_style: Option<&Element>,
    ) -> Result<()> {
        let element = &self.composition.elements[idx];
        let alpha = parent_alpha * Self::active_opacity(element, time);
        if alpha <= 0.0 {
            return Ok(());
        }
        let rect = if idx == 0 {
            parent
        } else {
            Rect {
                x: parent.x
                    + element
                        .style
                        .left
                        .map(|n| n.resolve(parent.w))
                        .unwrap_or(0.0),
                y: parent.y
                    + element
                        .style
                        .top
                        .map(|n| n.resolve(parent.h))
                        .unwrap_or(0.0),
                w: element
                    .style
                    .width
                    .map(|n| n.resolve(parent.w))
                    .unwrap_or(parent.w),
                h: element
                    .style
                    .height
                    .map(|n| n.resolve(parent.h))
                    .unwrap_or(parent.h),
            }
        };
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return Ok(());
        }
        if let Some(color) = element.style.background {
            self.fill(rect, color, alpha);
        }
        match &element.content {
            Content::Image(path) => self.draw_image(
                path,
                rect,
                element.style.object_fit.as_deref().unwrap_or("fill"),
                alpha,
            )?,
            Content::Text(text) => self.draw_text(text, rect, idx, parent_style, alpha)?,
            Content::Container => {}
        }
        let mut children = element.children.clone();
        children.sort_by_key(|idx| {
            let e = &self.composition.elements[*idx];
            (e.track, e.style.z_index.unwrap_or(0), *idx)
        });
        for child in children {
            self.draw_node(child, rect, alpha, time, Some(element))?;
        }
        Ok(())
    }

    fn blend_at(
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
        dest[0] = (color.r as f32 * a + dest[0] as f32 * (1.0 - a)).round() as u8;
        dest[1] = (color.g as f32 * a + dest[1] as f32 * (1.0 - a)).round() as u8;
        dest[2] = (color.b as f32 * a + dest[2] as f32 * (1.0 - a)).round() as u8;
        dest[3] = 255;
    }

    fn blend(&mut self, x: i32, y: i32, color: Color, alpha: f32) {
        Self::blend_at(
            &mut self.pixels,
            self.width,
            self.height,
            x,
            y,
            color,
            alpha,
        );
    }

    fn fill(&mut self, rect: Rect, color: Color, alpha: f32) {
        let x0 = (rect.x * self.scale).floor().max(0.0) as i32;
        let y0 = (rect.y * self.scale).floor().max(0.0) as i32;
        let x1 = ((rect.x + rect.w) * self.scale)
            .ceil()
            .min(self.width as f32) as i32;
        let y1 = ((rect.y + rect.h) * self.scale)
            .ceil()
            .min(self.height as f32) as i32;
        if alpha >= 1.0 && color.a == 255 {
            for y in y0..y1 {
                for x in x0..x1 {
                    let offset = (y as usize * self.width as usize + x as usize) * 4;
                    self.pixels[offset..offset + 4]
                        .copy_from_slice(&[color.r, color.g, color.b, 255]);
                }
            }
        } else {
            for y in y0..y1 {
                for x in x0..x1 {
                    self.blend(x, y, color, alpha);
                }
            }
        }
    }

    fn image_for(&mut self, path: &PathBuf, w: u32, h: u32, fit: &str) -> Result<&RgbaImage> {
        let key = (path.clone(), w, h, fit.to_string());
        if !self.images.contains_key(&key) {
            let bytes = (w as usize)
                .checked_mul(h as usize)
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| anyhow!("imagen rasterizada demasiado grande"))?;
            const CACHE_LIMIT: usize = 64 * 1024 * 1024;
            if bytes > CACHE_LIMIT {
                return Err(anyhow!("imagen rasterizada excede 64 MiB; usa --scale"));
            }
            let (source_w, source_h) = image::image_dimensions(path)
                .with_context(|| format!("no se pudo leer imagen {}", path.display()))?;
            if source_w as u64 * source_h as u64 > 16_000_000 {
                return Err(anyhow!(
                    "imagen de más de 16 megapíxeles: {}; redúcela",
                    path.display()
                ));
            }
            let image = image::open(path)
                .with_context(|| format!("no se pudo abrir imagen {}", path.display()))?
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
                _ => return Err(anyhow!("object-fit no soportado: {fit}")),
            };
            while self.image_cache_bytes + bytes > CACHE_LIMIT {
                if let Some(oldest) = self.image_order.pop_front() {
                    if let Some(old) = self.images.remove(&oldest) {
                        self.image_cache_bytes -= old.as_raw().len();
                    }
                } else {
                    break;
                }
            }
            self.image_cache_bytes += fitted.as_raw().len();
            self.image_order.push_back(key.clone());
            self.images.insert(key.clone(), fitted);
        }
        Ok(&self.images[&key])
    }

    fn draw_image(&mut self, path: &PathBuf, rect: Rect, fit: &str, alpha: f32) -> Result<()> {
        let w = (rect.w * self.scale).round().max(1.0) as u32;
        let h = (rect.h * self.scale).round().max(1.0) as u32;
        self.image_for(path, w, h, fit)?;
        let key = (path.clone(), w, h, fit.to_string());
        let image = &self.images[&key];
        let x0 = (rect.x * self.scale).round() as i32;
        let y0 = (rect.y * self.scale).round() as i32;
        for (x, y, pixel) in image.enumerate_pixels() {
            Self::blend_at(
                &mut self.pixels,
                self.width,
                self.height,
                x0 + x as i32,
                y0 + y as i32,
                Color {
                    r: pixel[0],
                    g: pixel[1],
                    b: pixel[2],
                    a: pixel[3],
                },
                alpha,
            );
        }
        Ok(())
    }

    fn font(&mut self, family: &str) -> Result<&Font> {
        if !self.fonts.contains_key(family) {
            let id = family.split(',').find_map(|item| {
                let item = item.trim().trim_matches(['\'', '"']);
                let requested = match item.to_ascii_lowercase().as_str() {
                    "sans-serif" => Family::SansSerif,
                    "serif" => Family::Serif,
                    "monospace" => Family::Monospace,
                    _ => Family::Name(item),
                };
                let list = [requested];
                self.font_db.query(&Query {
                    families: &list,
                    ..Default::default()
                })
            });
            let fallback = [Family::SansSerif];
            let id = id
                .or_else(|| {
                    self.font_db.query(&Query {
                        families: &fallback,
                        ..Default::default()
                    })
                })
                .ok_or_else(|| anyhow!("no hay tipografía del sistema disponible"))?;
            let font = self
                .font_db
                .with_face_data(id, |data, index| {
                    Font::from_bytes(
                        // Font must own these bytes after fontdb's callback returns.
                        #[allow(clippy::unnecessary_to_owned)]
                        data.to_vec(),
                        FontSettings {
                            collection_index: index,
                            ..Default::default()
                        },
                    )
                })
                .ok_or_else(|| anyhow!("no se pudo leer tipografía"))?
                .map_err(|error| anyhow!("tipografía inválida: {error}"))?;
            self.fonts.insert(family.to_string(), font);
        }
        Ok(&self.fonts[family])
    }

    fn draw_text(
        &mut self,
        text: &str,
        rect: Rect,
        idx: usize,
        parent_style: Option<&Element>,
        alpha: f32,
    ) -> Result<()> {
        let element = &self.composition.elements[idx];
        let (mut family, mut font_size, mut color, mut align) = (None, None, None, None);
        let mut current = Some(idx);
        while let Some(i) = current {
            let node = &self.composition.elements[i];
            family = family.or(node.style.font_family.as_deref());
            font_size = font_size.or(node.style.font_size);
            color = color.or(node.style.color);
            align = align.or(node.style.text_align.as_deref());
            current = node.parent;
        }
        let family = family.unwrap_or("sans-serif");
        let size = font_size.unwrap_or(match element.tag.as_str() {
            "h1" => 64.0,
            "h2" => 48.0,
            "h3" => 36.0,
            _ => 24.0,
        }) * self.scale;
        let color = color.unwrap_or(Color::WHITE);
        let align = align.unwrap_or("left");
        let parent_flex = parent_style.and_then(|p| p.style.display.as_deref()) == Some("flex");
        let center_x = align == "center"
            || (parent_flex
                && parent_style.and_then(|p| p.style.justify_content.as_deref()) == Some("center"));
        let center_y = parent_flex
            && parent_style.and_then(|p| p.style.align_items.as_deref()) == Some("center");
        let scale = self.scale;
        let font = self.font(family)?;
        let advance: f32 = text
            .chars()
            .map(|c| font.metrics(c, size).advance_width)
            .sum();
        let x = rect.x * scale
            + if center_x {
                (rect.w * scale - advance) / 2.0
            } else if align == "right" {
                rect.w * scale - advance
            } else {
                0.0
            };
        let baseline = rect.y * scale
            + if center_y {
                (rect.h * scale + size) / 2.0
            } else {
                size
            };
        let mut glyphs = Vec::new();
        let mut pen = x;
        for ch in text.chars() {
            let (metrics, bitmap) = font.rasterize(ch, size);
            let gx = pen + metrics.xmin as f32;
            let gy = baseline - metrics.ymin as f32 - metrics.height as f32;
            glyphs.push((
                gx.round() as i32,
                gy.round() as i32,
                metrics.width,
                metrics.height,
                bitmap,
            ));
            pen += metrics.advance_width;
        }
        for (gx, gy, w, h, bitmap) in glyphs {
            for y in 0..h {
                for x in 0..w {
                    let coverage = bitmap[y * w + x] as f32 / 255.0;
                    if coverage > 0.0 {
                        self.blend(gx + x as i32, gy + y as i32, color, alpha * coverage);
                    }
                }
            }
        }
        Ok(())
    }
}
