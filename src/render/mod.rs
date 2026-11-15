//! Software renderer: turns a parsed composition into RGBA frames, one pure function of time.
//!
//! Submodules: `draw` (scene traversal), `layout` (flex sizing and placement), `paint` (the `Surface`, fills, gradients, rotation and `overflow` layers),
//! `text` (fonts, measurement, glyph painting), `geometry` and `frame_state` (shared small types).

mod draw;
mod frame_state;
mod geometry;
mod layout;
mod paint;
mod text;

use crate::model::{Composition, Content, Element};
use anyhow::{anyhow, Context, Result};
use fontdb::Database;
use frame_state::FrameState;
use geometry::Rect;
use paint::{CachedImage, ImageKey, Surface};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use text::{FontStore, TextBlock, TextRun};

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

pub struct Renderer<'a> {
    composition: &'a Composition,
    scale: f32,
    surface: Surface,
    initialized: bool,
    previous: Vec<FrameState>,
    current: Vec<FrameState>,
    children: Vec<Vec<usize>>,
    text_runs: HashMap<(usize, u32, i64, i32), TextRun>,
    text_blocks: HashMap<(usize, i64), Arc<TextBlock>>,
    text_cache_bytes: usize,
    images: HashMap<ImageKey<'a>, CachedImage>,
    image_order: VecDeque<ImageKey<'a>>,
    image_cache_bytes: usize,
    fonts: FontStore,
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
            self.surface.width,
            self.surface.height,
            Some(self.fonts.index()),
        )
    }

    pub(in crate::render) fn with_dimensions(
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
            .ok_or_else(|| anyhow!("dimensions are too large"))?;
        if size > 128 * 1024 * 1024 {
            return Err(anyhow!("frame requires more than 128 MiB; use --scale"));
        }
        let fonts = shared_fonts.map_or_else(
            || {
                FontStore::new(
                    composition
                        .elements
                        .iter()
                        .any(|e| matches!(e.content, Content::Text(_))),
                )
            },
            FontStore::with_index,
        );
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
            scale,
            surface: Surface::blank(width, height, scale, composition.background),
            initialized: false,
            previous: Vec::with_capacity(composition.elements.len()),
            current: Vec::with_capacity(composition.elements.len()),
            children,
            text_runs: HashMap::new(),
            text_blocks: HashMap::new(),
            text_cache_bytes: 0,
            images: HashMap::new(),
            image_order: VecDeque::new(),
            image_cache_bytes: 0,
            fonts,
            stats: RenderStats::default(),
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.surface.width, self.surface.height)
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

    pub(in crate::render) fn active_opacity(element: &Element, time: f64) -> f32 {
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
        Ok((&self.surface.pixels, self.stats.rasterized))
    }

    pub fn render_at(&mut self, time: f64) -> Result<&[u8]> {
        if !time.is_finite() || time < 0.0 || time >= self.composition.duration {
            return Err(anyhow!("time is outside the composition duration"));
        }
        self.render_time(time)
    }

    pub(in crate::render) fn render_time(&mut self, time: f64) -> Result<&[u8]> {
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
            return Ok(&self.surface.pixels);
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
        Ok(&self.surface.pixels)
    }

    pub(in crate::render) fn reset_background(&mut self) {
        let color = self.composition.background;
        let rgba = [color.r, color.g, color.b, 255];
        if !self.initialized {
            self.surface.pixels.as_chunks_mut::<4>().0.fill(rgba);
            self.initialized = true;
        }
        if let Some(rect) = self.surface.painted.take() {
            for row in self.surface.rows(rect) {
                row.as_chunks_mut::<4>().0.fill(rgba);
            }
        }
    }
}
