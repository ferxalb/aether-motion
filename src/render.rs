use crate::model::{Background, Color, Composition, Content, Element, Shadow};
use anyhow::{anyhow, Context, Result};
use fontdb::{Database, Family, Query};
use fontdue::{Font, FontSettings};
use image::{imageops, Rgba, RgbaImage};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// A flex item before its main size grows into the free space.
struct FlexItem {
    idx: usize,
    main: f32,
    cross: Option<f32>,
    grow: f32,
}

/// Direct children that follow the parent's flex layout instead of positioning themselves.
fn in_flow(composition: &Composition, idx: usize) -> bool {
    let element = &composition.elements[idx];
    element
        .parent
        .is_some_and(|parent| composition.elements[parent].style.display.as_deref() == Some("flex"))
        && element.style.position.as_deref() != Some("absolute")
}

/// Signed distance from `(x, y)` to a rounded rectangle; negative inside.
fn rounded_distance(x: f32, y: f32, [x0, y0, x1, y1]: [f32; 4], radius: f32) -> f32 {
    let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let radius = radius.clamp(0.0, hx.min(hy).max(0.0));
    let qx = (x - (x0 + hx)).abs() - (hx - radius);
    let qy = (y - (y0 + hy)).abs() - (hy - radius);
    // Only the corner arcs need a square root.
    let outside = if qx > 0.0 && qy > 0.0 {
        qx.hypot(qy)
    } else {
        qx.max(qy)
    };
    outside - radius
}

/// Complementary error function (Abramowitz & Stegun 7.1.26, error below 1.5e-7).
fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_72 + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let tail = poly * (-z * z).exp();
    if x >= 0.0 {
        tail
    } else {
        2.0 - tail
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
struct FrameState {
    alpha: f32,
    x: f32,
    y: f32,
    scale: f32,
}

impl FrameState {
    fn at(element: &Element, time: f64, alpha: f32) -> Self {
        if alpha <= 0.0 {
            return Self::default();
        }
        let pose = element.motion.pose(time - element.start);
        Self {
            alpha: alpha * pose.opacity,
            x: pose.x,
            y: pose.y,
            scale: pose.scale,
        }
    }
}

struct Glyph {
    metrics: fontdue::Metrics,
    coverage: Vec<u8>,
}

struct TextRun {
    glyphs: Vec<Glyph>,
    advance: f32,
    size: f32,
    color: Color,
    center_x: bool,
    center_y: bool,
    right: bool,
}

#[derive(Default, Debug)]
pub struct RenderStats {
    pub rasterized: usize,
    pub reused: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum OutputSize {
    FullHd,
    Uhd,
}

impl std::str::FromStr for OutputSize {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "1080p" => Ok(Self::FullHd),
            "4k" => Ok(Self::Uhd),
            _ => Err(anyhow!("size must be 1080p or 4k")),
        }
    }
}

#[derive(Clone, Copy)]
struct PixelRect {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl PixelRect {
    fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

struct CachedImage {
    pixels: RgbaImage,
    opaque: bool,
}

type ImageKey<'a> = (&'a Path, u32, u32, &'a str);

pub struct Renderer<'a> {
    composition: &'a Composition,
    width: u32,
    height: u32,
    scale: f32,
    pixels: Vec<u8>,
    painted: Option<PixelRect>,
    initialized: bool,
    previous: Vec<FrameState>,
    current: Vec<FrameState>,
    children: Vec<Vec<usize>>,
    text_runs: HashMap<(usize, u32), TextRun>,
    text_cache_bytes: usize,
    images: HashMap<ImageKey<'a>, CachedImage>,
    image_order: VecDeque<ImageKey<'a>>,
    image_cache_bytes: usize,
    fonts: HashMap<String, Font>,
    font_db: Database,
    stats: RenderStats,
}

pub(crate) fn check_image(path: &Path) -> Result<()> {
    let (width, height) = image::image_dimensions(path)
        .with_context(|| format!("could not read image {}", path.display()))?;
    if width as u64 * height as u64 > 16_000_000 {
        return Err(anyhow!(
            "image exceeds 16 megapixels: {}; reduce its size",
            path.display()
        ));
    }
    Ok(())
}

impl<'a> Renderer<'a> {
    pub fn with_output_size(composition: &'a Composition, size: OutputSize) -> Result<Self> {
        if u64::from(composition.width) * 9 != u64::from(composition.height) * 16 {
            return Err(anyhow!(
                "size requires a 16:9 composition; use --scale for other aspect ratios"
            ));
        }
        let (width, height) = match size {
            OutputSize::FullHd => (1920, 1080),
            OutputSize::Uhd => (3840, 2160),
        };
        Self::with_dimensions(
            composition,
            width as f32 / composition.width as f32,
            width,
            height,
            None,
        )
    }

    pub fn new(composition: &'a Composition, scale: f32) -> Result<Self> {
        if !scale.is_finite() || scale <= 0.0 || scale > 4.0 {
            return Err(anyhow!("--scale must be between 0 and 4"));
        }
        let width = (composition.width as f32 * scale).round() as u32;
        let height = (composition.height as f32 * scale).round() as u32;
        Self::with_dimensions(composition, scale, width, height, None)
    }

    /// Independent renderer for the same composition and output size.
    /// It shares the already loaded system font index instead of scanning disks again,
    /// so it can render any frame in any order on another thread.
    pub fn fork(&self) -> Result<Self> {
        Self::with_dimensions(
            self.composition,
            self.scale,
            self.width,
            self.height,
            Some(self.font_db.clone()),
        )
    }

    fn with_dimensions(
        composition: &'a Composition,
        scale: f32,
        width: u32,
        height: u32,
        shared_fonts: Option<Database>,
    ) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(anyhow!("scale is too small"));
        }
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| anyhow!("dimensiones demasiado grandes"))?;
        if size > 128 * 1024 * 1024 {
            return Err(anyhow!("frame requires more than 128 MiB; use --scale"));
        }
        let font_db = shared_fonts.unwrap_or_else(|| {
            let mut db = Database::new();
            if composition
                .elements
                .iter()
                .any(|e| matches!(e.content, Content::Text(_)))
            {
                db.load_system_fonts();
            }
            db
        });
        let children = composition
            .elements
            .iter()
            .map(|element| {
                let mut children = element.children.clone();
                children.sort_unstable_by_key(|&idx| {
                    let child = &composition.elements[idx];
                    (child.track, child.style.z_index.unwrap_or(0), idx)
                });
                children
            })
            .collect();
        Ok(Self {
            composition,
            width,
            height,
            scale,
            pixels: vec![0; size],
            painted: None,
            initialized: false,
            previous: Vec::with_capacity(composition.elements.len()),
            current: Vec::with_capacity(composition.elements.len()),
            children,
            text_runs: HashMap::new(),
            text_cache_bytes: 0,
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
    pub(crate) fn add_stats(&mut self, rasterized: usize, reused: usize) {
        self.stats.rasterized += rasterized;
        self.stats.reused += reused;
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
        // Element opacity lives in `motion.base`, so animations can replace it.
        let mut alpha = 1.0;
        if element.fade_in > 0.0 {
            alpha *= ((time - element.start) / element.fade_in).clamp(0.0, 1.0) as f32;
        }
        if element.fade_out > 0.0 {
            alpha *= ((element.end - time) / element.fade_out).clamp(0.0, 1.0) as f32;
        }
        alpha
    }

    pub fn render_frame(&mut self, frame: usize, fps: u32) -> Result<&[u8]> {
        self.render_frame_with_revision(frame, fps)
            .map(|(pixels, _)| pixels)
    }

    pub(crate) fn render_frame_with_revision(
        &mut self,
        frame: usize,
        fps: u32,
    ) -> Result<(&[u8], usize)> {
        if fps == 0 {
            return Err(anyhow!("fps must be greater than zero"));
        }
        if frame >= self.frame_count(fps) {
            return Err(anyhow!("frame is outside the composition duration"));
        }
        self.render_time(frame as f64 / fps as f64)?;
        Ok((&self.pixels, self.stats.rasterized))
    }

    pub fn render_at(&mut self, time: f64) -> Result<&[u8]> {
        if !time.is_finite() || time < 0.0 || time >= self.composition.duration {
            return Err(anyhow!("time is outside the composition duration"));
        }
        self.render_time(time)
    }

    fn render_time(&mut self, time: f64) -> Result<&[u8]> {
        self.current.clear();
        for element in &self.composition.elements {
            let parent_alpha = element.parent.map_or(1.0, |idx| self.current[idx].alpha);
            let alpha = if parent_alpha > 0.0 {
                parent_alpha * Self::active_opacity(element, time)
            } else {
                0.0
            };
            let state = FrameState::at(element, time, alpha);
            self.current.push(if state.alpha > 0.0 {
                state
            } else {
                FrameState::default()
            });
        }
        if self.previous == self.current {
            self.stats.reused += 1;
            return Ok(&self.pixels);
        }
        self.previous.clear();
        self.reset_background();
        let root = Rect {
            x: 0.0,
            y: 0.0,
            w: self.composition.width as f32,
            h: self.composition.height as f32,
        };
        self.draw_node(0, root, 1.0, None)?;
        std::mem::swap(&mut self.previous, &mut self.current);
        self.stats.rasterized += 1;
        Ok(&self.pixels)
    }

    fn reset_background(&mut self) {
        let color = self.composition.background;
        let rgba = [color.r, color.g, color.b, 255];
        if !self.initialized {
            self.pixels.as_chunks_mut::<4>().0.fill(rgba);
            self.initialized = true;
        }
        if let Some(rect) = self.painted.take() {
            for row in self.rows(rect) {
                row.as_chunks_mut::<4>().0.fill(rgba);
            }
        }
    }

    fn rows(&mut self, rect: PixelRect) -> impl Iterator<Item = &mut [u8]> {
        let stride = self.width as usize * 4;
        self.pixels[rect.y0 * stride..rect.y1 * stride]
            .chunks_exact_mut(stride)
            .map(move |row| &mut row[rect.x0 * 4..rect.x1 * 4])
    }

    fn mark_painted(&mut self, rect: PixelRect) {
        self.painted = Some(self.painted.map_or(rect, |painted| painted.union(rect)));
    }

    fn clip_pixels(
        (width, height): (u32, u32),
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    ) -> Option<PixelRect> {
        let x0 = x0.clamp(0, width as i32) as usize;
        let y0 = y0.clamp(0, height as i32) as usize;
        let x1 = x1.clamp(0, width as i32) as usize;
        let y1 = y1.clamp(0, height as i32) as usize;
        (x0 < x1 && y0 < y1).then_some(PixelRect { x0, y0, x1, y1 })
    }

    fn draw_node(
        &mut self,
        idx: usize,
        parent: Rect,
        parent_scale: f32,
        assigned: Option<Rect>,
    ) -> Result<()> {
        let state = self.current[idx];
        let alpha = state.alpha;
        if alpha <= 0.0 {
            return Ok(());
        }
        let element = &self.composition.elements[idx];
        let ps = parent_scale;
        let mut rect = if idx == 0 {
            parent
        } else if let Some(slot) = assigned {
            // Static items ignore left/top; `position: relative` shifts them.
            let relative = element.style.position.as_deref() == Some("relative");
            let offset = |length: Option<crate::model::Length>, basis: f32| {
                length
                    .filter(|_| relative)
                    .map_or(0.0, |n| n.resolve_scaled(basis, ps))
            };
            Rect {
                x: slot.x + offset(element.style.left, parent.w),
                y: slot.y + offset(element.style.top, parent.h),
                ..slot
            }
        } else {
            let style = &element.style;
            Rect {
                x: parent.x + style.left.map_or(0.0, |n| n.resolve_scaled(parent.w, ps)),
                y: parent.y + style.top.map_or(0.0, |n| n.resolve_scaled(parent.h, ps)),
                w: style
                    .width
                    .map_or(parent.w, |n| n.resolve_scaled(parent.w, ps)),
                h: style
                    .height
                    .map_or(parent.h, |n| n.resolve_scaled(parent.h, ps)),
            }
        };
        rect.x += state.x * ps;
        rect.y += state.y * ps;
        // CSS scale grows around the element center; descendants inherit it.
        let mut cumulative = ps;
        if idx != 0 && state.scale != 1.0 {
            let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            rect.w *= state.scale;
            rect.h *= state.scale;
            rect.x = cx - rect.w / 2.0;
            rect.y = cy - rect.h / 2.0;
            cumulative *= state.scale;
        }
        let radius = element.style.border_radius * cumulative;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return Ok(());
        }
        if let Some(shadow) = element.style.shadow {
            self.draw_shadow(rect, radius, shadow, alpha, cumulative);
        }
        match element.style.background {
            Some(Background::Solid(color)) => {
                let base = self.composition.background;
                if idx != 0 || (color.r, color.g, color.b) != (base.r, base.g, base.b) {
                    self.fill(rect, color, alpha, radius);
                }
            }
            Some(Background::Linear {
                from,
                to,
                horizontal,
            }) => {
                self.fill_gradient(rect, [from, to], horizontal, alpha, radius);
            }
            None => {}
        }
        let [top, right, bottom, left] = element.style.padding.map(|p| p * cumulative);
        let content = Rect {
            x: rect.x + left,
            y: rect.y + top,
            w: (rect.w - left - right).max(0.0),
            h: (rect.h - top - bottom).max(0.0),
        };
        match &element.content {
            Content::Image(path) => self.draw_image(
                path,
                rect,
                element.style.object_fit.as_deref().unwrap_or("fill"),
                alpha,
            )?,
            Content::Text(text) => self.draw_text(text, content, idx, alpha, cumulative)?,
            Content::Container => {}
        }
        let layout = if element.style.display.as_deref() == Some("flex") {
            self.flex_layout(idx, content, cumulative)?
        } else {
            Vec::new()
        };
        for i in 0..self.children[idx].len() {
            let child = self.children[idx][i];
            let slot = layout.iter().find(|(c, _)| *c == child).map(|(_, r)| *r);
            self.draw_node(child, rect, cumulative, slot)?;
        }
        Ok(())
    }

    /// Explicit or intrinsic size of an element in layout units; `None` means unknown.
    fn item_size(
        &mut self,
        idx: usize,
        ps: f32,
        avail: (f32, f32),
    ) -> Result<(Option<f32>, Option<f32>)> {
        let composition: &'a Composition = self.composition;
        let element = &composition.elements[idx];
        let mut width = element.style.width.map(|n| n.resolve_scaled(avail.0, ps));
        let mut height = element.style.height.map(|n| n.resolve_scaled(avail.1, ps));
        if width.is_some() && height.is_some() {
            return Ok((width, height));
        }
        let intrinsic = match &element.content {
            Content::Text(text) => Some(self.measure_text(text, idx, ps)?),
            Content::Container if element.style.display.as_deref() == Some("flex") => {
                self.flex_intrinsic(idx, ps)?
            }
            _ => None,
        };
        if let Some((w, h)) = intrinsic {
            width = width.or(Some(w));
            height = height.or(Some(h));
        }
        Ok((width, height))
    }

    /// Content-sized dimensions of a flex container, padding included.
    fn flex_intrinsic(&mut self, idx: usize, ps: f32) -> Result<Option<(f32, f32)>> {
        let composition: &'a Composition = self.composition;
        let style = &composition.elements[idx].style;
        let kids: Vec<usize> = composition.elements[idx]
            .children
            .iter()
            .copied()
            .filter(|&child| in_flow(composition, child))
            .collect();
        if kids.is_empty() {
            return Ok(None);
        }
        let (mut main, mut cross) = (0.0_f32, 0.0_f32);
        for &kid in &kids {
            let (w, h) = self.item_size(kid, ps, (0.0, 0.0))?;
            let (m, c) = if style.column { (h, w) } else { (w, h) };
            main += m.unwrap_or(0.0);
            cross = cross.max(c.unwrap_or(0.0));
        }
        main += style.gap * ps * (kids.len() - 1) as f32;
        let [top, right, bottom, left] = style.padding.map(|p| p * ps);
        Ok(Some(if style.column {
            (cross + left + right, main + top + bottom)
        } else {
            (main + left + right, cross + top + bottom)
        }))
    }

    /// Places the in-flow children of a flex container inside `content`.
    fn flex_layout(&mut self, idx: usize, content: Rect, ps: f32) -> Result<Vec<(usize, Rect)>> {
        let composition: &'a Composition = self.composition;
        let style = &composition.elements[idx].style;
        let kids: Vec<usize> = composition.elements[idx]
            .children
            .iter()
            .copied()
            .filter(|&child| in_flow(composition, child))
            .collect();
        if kids.is_empty() {
            return Ok(Vec::new());
        }
        let column = style.column;
        let (main_space, cross_space) = if column {
            (content.h, content.w)
        } else {
            (content.w, content.h)
        };
        let gap = style.gap * ps;
        let stretch = !matches!(
            style.align_items.as_deref(),
            Some("flex-start" | "start" | "center" | "flex-end" | "end")
        );
        let mut items = Vec::with_capacity(kids.len());
        for &kid in &kids {
            let (w, h) = self.item_size(kid, ps, (content.w, content.h))?;
            let (main, cross) = if column { (h, w) } else { (w, h) };
            let kid_style = &composition.elements[kid].style;
            items.push(FlexItem {
                idx: kid,
                main: if kid_style.flex_basis_zero {
                    0.0
                } else {
                    main.unwrap_or(0.0)
                },
                cross,
                grow: kid_style.flex_grow,
            });
        }
        let gaps = gap * (items.len() - 1) as f32;
        let free = main_space - items.iter().map(|item| item.main).sum::<f32>() - gaps;
        let total_grow: f32 = items.iter().map(|item| item.grow).sum();
        let free = if free > 0.0 && total_grow > 0.0 {
            for item in &mut items {
                item.main += free * item.grow / total_grow;
            }
            0.0
        } else {
            free
        };
        let count = items.len() as f32;
        let spare = free.max(0.0);
        let (start, between) = match style.justify_content.as_deref() {
            Some("center") => (free / 2.0, 0.0),
            Some("flex-end" | "end") => (free, 0.0),
            Some("space-between") if items.len() > 1 => (0.0, spare / (count - 1.0)),
            Some("space-around") => (spare / count / 2.0, spare / count),
            Some("space-evenly") => (spare / (count + 1.0), spare / (count + 1.0)),
            _ => (0.0, 0.0),
        };
        let mut position = start;
        let mut placed = Vec::with_capacity(items.len());
        for item in items {
            let cross = item
                .cross
                .unwrap_or(if stretch { cross_space } else { 0.0 });
            let offset = match style.align_items.as_deref() {
                Some("center") => (cross_space - cross) / 2.0,
                Some("flex-end" | "end") => cross_space - cross,
                _ => 0.0,
            };
            let rect = if column {
                Rect {
                    x: content.x + offset,
                    y: content.y + position,
                    w: cross,
                    h: item.main,
                }
            } else {
                Rect {
                    x: content.x + position,
                    y: content.y + offset,
                    w: item.main,
                    h: cross,
                }
            };
            placed.push((item.idx, rect));
            position += item.main + gap + between;
        }
        Ok(placed)
    }

    /// Soft outer shadow: Gaussian falloff from the distance to the shadow's rounded rectangle.
    fn draw_shadow(&mut self, rect: Rect, radius: f32, shadow: Shadow, alpha: f32, zoom: f32) {
        let k = self.scale;
        let unit = zoom * k;
        let spread = shadow.spread * unit;
        let sigma = shadow.blur * unit / 2.0;
        let element = [
            rect.x * k,
            rect.y * k,
            (rect.x + rect.w) * k,
            (rect.y + rect.h) * k,
        ];
        let cast = [
            element[0] + shadow.x * unit - spread,
            element[1] + shadow.y * unit - spread,
            element[2] + shadow.x * unit + spread,
            element[3] + shadow.y * unit + spread,
        ];
        let tail = sigma * 3.0;
        let reach = tail.ceil() + 1.0;
        let Some(bounds) = Self::clip_pixels(
            (self.width, self.height),
            (cast[0] - reach).floor() as i32,
            (cast[1] - reach).floor() as i32,
            (cast[2] + reach).ceil() as i32,
            (cast[3] + reach).ceil() as i32,
        ) else {
            return;
        };
        self.mark_painted(bounds);
        let cast_radius = (radius * k + spread).max(0.0);
        let element_radius = radius * k;
        let strength = alpha * f32::from(shadow.color.a) / 255.0;
        // Gaussian edge profile sampled every 1/16 px; beyond +-3 sigma it is flat.
        const STEPS_PER_PX: f32 = 16.0;
        let table: Vec<f32> = if sigma < 0.01 {
            Vec::new()
        } else {
            let n = (tail * 2.0 * STEPS_PER_PX).ceil() as usize + 1;
            (0..=n)
                .map(|i| {
                    let d = -tail + i as f32 / STEPS_PER_PX;
                    0.5 * erfc(d / (sigma * std::f32::consts::SQRT_2))
                })
                .collect()
        };
        let coverage = |distance: f32| -> f32 {
            if table.is_empty() {
                (0.5 - distance).clamp(0.0, 1.0)
            } else if distance >= tail {
                0.0
            } else if distance <= -tail {
                1.0
            } else {
                let position = (distance + tail) * STEPS_PER_PX;
                let i = (position as usize).min(table.len() - 2);
                let fraction = position - i as f32;
                table[i] + (table[i + 1] - table[i]) * fraction
            }
        };
        let (center_y, half_y) = (
            (element[1] + element[3]) / 2.0,
            (element[3] - element[1]) / 2.0,
        );
        // Pixels well inside the element are hidden by it; skip them on its straight rows.
        let inside_from = (element[0] + 0.5).ceil() as usize;
        let inside_to = (element[2] - 1.5).floor().max(0.0) as usize;
        let (blend_color, stride) = (
            Color {
                a: 255,
                ..shadow.color
            },
            self.width as usize,
        );
        for y in bounds.y0..bounds.y1 {
            let py = y as f32 + 0.5;
            let straight = (py - center_y).abs() <= half_y - element_radius - 1.0;
            let mut x = bounds.x0;
            while x < bounds.x1 {
                if straight && x >= inside_from && x <= inside_to {
                    x = inside_to + 1;
                    continue;
                }
                let px = x as f32 + 0.5;
                let distance = rounded_distance(px, py, cast, cast_radius);
                let hidden = coverage(distance) * strength;
                if hidden > 0.002 {
                    // CSS clips the shadow under the element itself.
                    let outside =
                        (rounded_distance(px, py, element, element_radius) + 0.5).clamp(0.0, 1.0);
                    let a = hidden * outside;
                    if a > 0.002 {
                        let offset = (y * stride + x) * 4;
                        Self::blend_pixel(&mut self.pixels[offset..offset + 4], blend_color, a);
                    }
                }
                x += 1;
            }
        }
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
        Self::blend_pixel(dest, color, a);
    }

    #[inline]
    fn blend_pixel(dest: &mut [u8], color: Color, a: f32) {
        dest[0] = (color.r as f32 * a + dest[0] as f32 * (1.0 - a)).round() as u8;
        dest[1] = (color.g as f32 * a + dest[1] as f32 * (1.0 - a)).round() as u8;
        dest[2] = (color.b as f32 * a + dest[2] as f32 * (1.0 - a)).round() as u8;
        dest[3] = 255;
    }

    fn over_background(base: Color, color: Color, alpha: f32) -> [u8; 4] {
        let mut pixel = [base.r, base.g, base.b, 255];
        Self::blend_pixel(&mut pixel, color, alpha);
        pixel
    }

    fn fill(&mut self, rect: Rect, color: Color, alpha: f32, radius: f32) {
        let (edges, radius) = self.fill_edges(rect, radius);
        let [x0, y0, x1, y1] = edges;
        let a = (alpha * color.a as f32 / 255.0).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let Some(bounds) = Self::clip_pixels((self.width, self.height), x0, y0, x1, y1) else {
            return;
        };
        let uniform_background = self.painted.is_none();
        self.mark_painted(bounds);
        if radius > 0.0 {
            self.fill_rounded(bounds, edges, radius, color, alpha);
            return;
        }
        let base = self.composition.background;
        let rows = self.rows(bounds);
        if a >= 1.0 || uniform_background {
            let pixel = if a >= 1.0 {
                [color.r, color.g, color.b, 255]
            } else {
                Self::over_background(base, color, a)
            };
            for row in rows {
                row.as_chunks_mut::<4>().0.fill(pixel);
            }
        } else if (bounds.x1 - bounds.x0) * (bounds.y1 - bounds.y0) >= 1024 {
            // Uniform color and opacity make each channel a lookup on the destination byte.
            let table = [color.r, color.g, color.b].map(|source| {
                std::array::from_fn::<_, 256, _>(|dest| {
                    (source as f32 * a + dest as f32 * (1.0 - a)).round() as u8
                })
            });
            for row in rows {
                for pixel in row.as_chunks_mut::<4>().0 {
                    pixel[0] = table[0][pixel[0] as usize];
                    pixel[1] = table[1][pixel[1] as usize];
                    pixel[2] = table[2][pixel[2] as usize];
                }
            }
        } else {
            for row in rows {
                for pixel in row.as_chunks_mut::<4>().0 {
                    pixel[0] = (color.r as f32 * a + pixel[0] as f32 * (1.0 - a)).round() as u8;
                    pixel[1] = (color.g as f32 * a + pixel[1] as f32 * (1.0 - a)).round() as u8;
                    pixel[2] = (color.b as f32 * a + pixel[2] as f32 * (1.0 - a)).round() as u8;
                }
            }
        }
    }

    fn fill_edges(&self, rect: Rect, radius: f32) -> ([i32; 4], f32) {
        let x0 = (rect.x * self.scale).floor() as i32;
        let y0 = (rect.y * self.scale).floor() as i32;
        let x1 = ((rect.x + rect.w) * self.scale).ceil() as i32;
        let y1 = ((rect.y + rect.h) * self.scale).ceil() as i32;
        let radius = (radius * self.scale)
            .min(x1.saturating_sub(x0).min(y1.saturating_sub(y0)) as f32 * 0.5);
        ([x0, y0, x1, y1], radius)
    }

    fn fill_gradient(
        &mut self,
        rect: Rect,
        [from, to]: [Color; 2],
        horizontal: bool,
        alpha: f32,
        radius: f32,
    ) {
        let (edges, radius) = self.fill_edges(rect, radius);
        let [x0, y0, x1, y1] = edges;
        let Some(bounds) = Self::clip_pixels((self.width, self.height), x0, y0, x1, y1) else {
            return;
        };
        let uniform_background = self.painted.is_none();
        self.mark_painted(bounds);
        let float_edges = edges.map(|edge| edge as f32);
        let [start, end] = if horizontal {
            [float_edges[0], float_edges[2]]
        } else {
            [float_edges[1], float_edges[3]]
        };
        let color_at = |position: usize| {
            let t = (position as f32 + 0.5 - start) / (end - start).max(1.0);
            let channel =
                |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
            Color {
                r: channel(from.r, to.r),
                g: channel(from.g, to.g),
                b: channel(from.b, to.b),
                a: 255,
            }
        };
        let columns: Vec<_> = if horizontal {
            (bounds.x0..bounds.x1)
                .map(|x| {
                    let c = color_at(x);
                    [c.r, c.g, c.b, 255]
                })
                .collect()
        } else {
            Vec::new()
        };
        let base = self.composition.background;
        // On the first layer every uncovered pixel starts with the same color.
        let blended_columns = (horizontal && uniform_background && alpha < 1.0).then(|| {
            columns
                .iter()
                .map(|&[r, g, b, a]| Self::over_background(base, Color { r, g, b, a }, alpha))
                .collect::<Vec<_>>()
        });
        let interior_columns = blended_columns.as_deref().unwrap_or(&columns);
        let copy_interior = alpha >= 1.0 || uniform_background;
        for (y, row) in (bounds.y0..bounds.y1).zip(self.rows(bounds)) {
            let pixels = row.as_chunks_mut::<4>().0;
            let solid = color_at(y);
            let solid_pixel = if !horizontal && uniform_background && alpha < 1.0 {
                Self::over_background(base, solid, alpha)
            } else {
                [solid.r, solid.g, solid.b, 255]
            };
            if radius == 0.0 && copy_interior {
                if horizontal {
                    pixels.copy_from_slice(interior_columns);
                } else {
                    pixels.fill(solid_pixel);
                }
                continue;
            }
            let (left, right) = Self::rounded_span(float_edges, radius, y);
            let left_pixel = (left.floor() as usize).clamp(bounds.x0, bounds.x1) - bounds.x0;
            let right_pixel = (right.ceil() as usize).clamp(bounds.x0, bounds.x1) - bounds.x0;
            let inner_left = (left.ceil() as usize).clamp(bounds.x0, bounds.x1) - bounds.x0;
            let inner_right = (right.floor() as usize).clamp(bounds.x0, bounds.x1) - bounds.x0;
            let paint = |pixel: &mut [u8; 4], i: usize, coverage: f32| {
                let color = if horizontal {
                    let [r, g, b, a] = columns[i];
                    Color { r, g, b, a }
                } else {
                    solid
                };
                Self::blend_pixel(pixel, color, alpha * coverage);
            };
            if copy_interior && inner_left < inner_right {
                if horizontal {
                    pixels[inner_left..inner_right]
                        .copy_from_slice(&interior_columns[inner_left..inner_right]);
                } else {
                    pixels[inner_left..inner_right].fill(solid_pixel);
                }
            } else if inner_left < inner_right {
                for (i, pixel) in pixels
                    .iter_mut()
                    .enumerate()
                    .take(inner_right)
                    .skip(inner_left)
                {
                    paint(pixel, i, 1.0);
                }
            }
            for i in [left_pixel, right_pixel.saturating_sub(1)] {
                if i >= left_pixel && i < right_pixel && !(inner_left..inner_right).contains(&i) {
                    let x = (bounds.x0 + i) as f32;
                    let coverage = (right.min(x + 1.0) - left.max(x)).clamp(0.0, 1.0);
                    paint(&mut pixels[i], i, coverage);
                    if right_pixel - left_pixel == 1 {
                        break;
                    }
                }
            }
        }
    }

    fn rounded_span([x0, y0, x1, y1]: [f32; 4], radius: f32, y: usize) -> (f32, f32) {
        let center = y as f32 + 0.5;
        let dy = (y0 + radius - center).max(center - (y1 - radius)).max(0.0);
        let inset = radius - (radius * radius - dy * dy).max(0.0).sqrt();
        (x0 + inset, x1 - inset)
    }

    fn fill_rounded(
        &mut self,
        bounds: PixelRect,
        edges: [i32; 4],
        radius: f32,
        color: Color,
        alpha: f32,
    ) {
        let edges = edges.map(|edge| edge as f32);
        let width = self.width;
        let height = self.height;
        for y in bounds.y0..bounds.y1 {
            let (left, right) = Self::rounded_span(edges, radius, y);
            let inner_left = (left.ceil() as usize).clamp(bounds.x0, bounds.x1);
            let inner_right = (right.floor() as usize).clamp(inner_left, bounds.x1);
            for x in [inner_left.saturating_sub(1), inner_right] {
                if x >= bounds.x0 && x < bounds.x1 {
                    let coverage = (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0);
                    Self::blend_at(
                        &mut self.pixels,
                        width,
                        height,
                        x as i32,
                        y as i32,
                        color,
                        alpha * coverage,
                    );
                }
            }
            if inner_left == inner_right {
                continue;
            }
            let row = PixelRect {
                x0: inner_left,
                y0: y,
                x1: inner_right,
                y1: y + 1,
            };
            let a = alpha * color.a as f32 / 255.0;
            for row in self.rows(row) {
                let pixels = row.as_chunks_mut::<4>().0;
                if a >= 1.0 {
                    pixels.fill([color.r, color.g, color.b, 255]);
                } else {
                    for pixel in pixels {
                        Self::blend_pixel(pixel, color, a);
                    }
                }
            }
        }
    }

    fn cache_image(&mut self, key: &ImageKey<'a>) -> Result<()> {
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

    fn draw_image(&mut self, path: &'a Path, rect: Rect, fit: &'a str, alpha: f32) -> Result<()> {
        let w = (rect.w * self.scale).round().max(1.0) as u32;
        let h = (rect.h * self.scale).round().max(1.0) as u32;
        let key = (path, w, h, fit);
        self.cache_image(&key)?;
        let x0 = (rect.x * self.scale).round() as i32;
        let y0 = (rect.y * self.scale).round() as i32;
        let Some(bounds) = Self::clip_pixels(
            (self.width, self.height),
            x0,
            y0,
            x0.saturating_add(w as i32),
            y0.saturating_add(h as i32),
        ) else {
            return Ok(());
        };
        self.mark_painted(bounds);
        let image = &self.images[&key];
        let stride = self.width as usize * 4;
        let source_stride = w as usize * 4;
        let source_x = (bounds.x0 as i64 - i64::from(x0)) as usize * 4;
        let source_y = (bounds.y0 as i64 - i64::from(y0)) as usize;
        let row_len = (bounds.x1 - bounds.x0) * 4;
        for (i, row) in self.pixels[bounds.y0 * stride..bounds.y1 * stride]
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
                        Self::blend_pixel(dest, color, alpha * f32::from(a) / 255.0);
                    }
                }
            }
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
                .ok_or_else(|| anyhow!("no system font is available"))?;
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
                .ok_or_else(|| anyhow!("could not read font"))?
                .map_err(|error| anyhow!("invalid font: {error}"))?;
            self.fonts.insert(family.to_string(), font);
        }
        Ok(&self.fonts[family])
    }

    /// Inherited font family, base size (comp px), color and alignment for a text element.
    fn text_props(&self, idx: usize) -> (&'a str, f32, Color, &'a str) {
        let composition: &'a Composition = self.composition;
        let (mut family, mut font_size, mut color, mut align) = (None, None, None, None);
        let mut current = Some(idx);
        while let Some(i) = current {
            let node = &composition.elements[i];
            family = family.or(node.style.font_family.as_deref());
            font_size = font_size.or(node.style.font_size);
            color = color.or(node.style.color);
            align = align.or(node.style.text_align.as_deref());
            current = node.parent;
        }
        let size = font_size.unwrap_or(match composition.elements[idx].tag.as_str() {
            "h1" => 64.0,
            "h2" => 48.0,
            "h3" => 36.0,
            _ => 24.0,
        });
        (
            family.unwrap_or("sans-serif"),
            size,
            color.unwrap_or(Color::WHITE),
            align.unwrap_or("left"),
        )
    }

    /// Width and height of one line of text in layout units, without rasterizing.
    fn measure_text(&mut self, text: &str, idx: usize, zoom: f32) -> Result<(f32, f32)> {
        let (family, base, _, _) = self.text_props(idx);
        let scale = self.scale;
        let size = base * scale * zoom;
        let font = self.font(family)?;
        let advance: f32 = text
            .chars()
            .map(|ch| font.metrics(ch, size).advance_width)
            .sum();
        Ok((advance / scale, size / scale))
    }

    fn prepare_text(&mut self, text: &str, idx: usize, zoom: f32) -> Result<TextRun> {
        let (family, base, color, align) = self.text_props(idx);
        let size = base * self.scale * zoom;
        let element = &self.composition.elements[idx];
        let parent_style = element
            .parent
            .map(|parent| &self.composition.elements[parent]);
        // Flex items are placed by the layout; only out-of-flow children keep the legacy centering.
        let parent_flex = parent_style.and_then(|p| p.style.display.as_deref()) == Some("flex")
            && !in_flow(self.composition, idx);
        let center_x = align == "center"
            || (parent_flex
                && parent_style.and_then(|p| p.style.justify_content.as_deref()) == Some("center"));
        let center_y = parent_flex
            && parent_style.and_then(|p| p.style.align_items.as_deref()) == Some("center");
        let font = self.font(family)?;
        let glyphs: Vec<Glyph> = text
            .chars()
            .map(|ch| {
                let (metrics, coverage) = font.rasterize(ch, size);
                Glyph { metrics, coverage }
            })
            .collect();
        let advance = glyphs.iter().map(|glyph| glyph.metrics.advance_width).sum();
        Ok(TextRun {
            glyphs,
            advance,
            size,
            color,
            center_x,
            center_y,
            right: align == "right",
        })
    }

    fn draw_text(
        &mut self,
        text: &str,
        rect: Rect,
        idx: usize,
        alpha: f32,
        zoom: f32,
    ) -> Result<()> {
        let key = (idx, zoom.to_bits());
        let uncached;
        let run = if let Some(run) = self.text_runs.get(&key) {
            run
        } else {
            let prepared = self.prepare_text(text, idx, zoom)?;
            let bytes = prepared
                .glyphs
                .iter()
                .map(|glyph| std::mem::size_of::<Glyph>() + glyph.coverage.len())
                .sum::<usize>();
            if self.text_cache_bytes.saturating_add(bytes) <= 32 * 1024 * 1024 {
                self.text_cache_bytes += bytes;
                self.text_runs.entry(key).or_insert(prepared)
            } else {
                uncached = prepared;
                &uncached
            }
        };
        let scale = self.scale;
        let mut pen = rect.x * scale
            + if run.center_x {
                (rect.w * scale - run.advance) / 2.0
            } else if run.right {
                rect.w * scale - run.advance
            } else {
                0.0
            };
        let baseline = rect.y * scale
            + if run.center_y {
                (rect.h * scale + run.size) / 2.0
            } else {
                run.size
            };
        let mut painted = self.painted;
        for glyph in &run.glyphs {
            let metrics = glyph.metrics;
            let gx = (pen + metrics.xmin as f32).round() as i32;
            let gy = (baseline - metrics.ymin as f32 - metrics.height as f32).round() as i32;
            pen += metrics.advance_width;
            let Some(bounds) = Self::clip_pixels(
                (self.width, self.height),
                gx,
                gy,
                gx.saturating_add(metrics.width as i32),
                gy.saturating_add(metrics.height as i32),
            ) else {
                continue;
            };
            painted = Some(painted.map_or(bounds, |area| area.union(bounds)));
            for y in bounds.y0..bounds.y1 {
                for x in bounds.x0..bounds.x1 {
                    let offset = (y as i64 - i64::from(gy)) as usize * metrics.width
                        + (x as i64 - i64::from(gx)) as usize;
                    let coverage = glyph.coverage[offset] as f32 / 255.0;
                    if coverage > 0.0 {
                        Self::blend_at(
                            &mut self.pixels,
                            self.width,
                            self.height,
                            x as i32,
                            y as i32,
                            run.color,
                            alpha * coverage,
                        );
                    }
                }
            }
        }
        self.painted = painted;
        Ok(())
    }
}
