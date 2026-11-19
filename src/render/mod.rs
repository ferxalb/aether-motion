//! Software renderer: turns a parsed composition into RGBA frames, one pure function of time.
//!
//! Submodules: `draw` (scene traversal), `layout` (flex sizing and placement), `paint` (the `Surface`, fills, gradients, rotation and `overflow` layers,
//! filters and blend modes), `text` (fonts, measurement, glyph painting), `shutter` (motion blur), `allocation` (checked buffer sizes),
//! `geometry` and `frame_state` (shared small types). `paint/video.rs` draws `<video>` frames.
//! A frame is drawn by `draw_scene` and finished by `finish_frame` (root color grade, vignette, grain).

mod allocation;
mod draw;
mod frame_state;
mod geometry;
mod layer_blur;
mod layout;
mod paint;
pub mod probe;
mod shutter;
mod text;

use crate::model::{Composition, Content, Element};
use crate::video::VideoPlayer;
use anyhow::{anyhow, Context, Result};
use fontdb::Database;
use frame_state::FrameState;
use geometry::Rect;
pub use layout::split_wrap::split_wrap_breaks;
pub(crate) use paint::svg;
use paint::{CachedImage, ImageKey, Surface};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use text::{FontStore, TextBlock, TextRun};

/// Highest frame rate accepted for stills and renders (`--fps`, the MCP `fps` of `aether_snapshot`).
pub const MAX_FPS: u32 = 240;

/// Frame rate that snapshots without an explicit `fps` assume for motion-blur shutter windows.
pub const SNAPSHOT_FPS: u32 = shutter::SNAPSHOT_FPS;

/// Checks a frame rate given as `name` (`--fps` on the CLI, `fps` in MCP): a whole number from 1 to
/// [`MAX_FPS`]. Shared by the CLI and MCP so both accept and reject the same values.
pub fn validate_fps(fps: u64, name: &str) -> Result<u32> {
    match u32::try_from(fps) {
        Ok(fps) if (1..=MAX_FPS).contains(&fps) => Ok(fps),
        _ => Err(anyhow!(
            "{name} must be a whole number from 1 to {MAX_FPS} (got {fps}); use the --fps of the export, such as 30 or 60"
        )),
    }
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

impl OutputSize {
    /// Output `(width, height)` for a composition of the given size: 16:9 landscape or 9:16 portrait.
    ///
    /// The preset names the short edge of a portrait frame and the long edge of a landscape one, so
    /// `1080p` is 1920x1080 or 1080x1920 and `4k` is 3840x2160 or 2160x3840.
    pub fn dimensions(self, width: u32, height: u32) -> Result<(u32, u32)> {
        let (long, short) = match self {
            Self::FullHd => (1920, 1080),
            Self::Uhd => (3840, 2160),
        };
        let (w, h) = (u64::from(width), u64::from(height));
        if w * 9 == h * 16 {
            Ok((long, short))
        } else if w * 16 == h * 9 {
            Ok((short, long))
        } else {
            Err(anyhow!(
                "size requires a 16:9 or 9:16 composition; use --scale for other aspect ratios"
            ))
        }
    }
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
    /// One decoder per `<video>` element (by element index), created when its first frame is drawn.
    videos: HashMap<usize, VideoPlayer>,
    fonts: FontStore,
    stats: RenderStats,
    /// Text boxes recorded while drawing, when a layout probe is running (see `probe.rs`).
    probe: Option<Vec<probe::TextBox>>,
    /// Number of offscreen layers currently being drawn into; their coordinates are layer-local.
    layer_depth: u32,
    /// Offset (composition px) from the current offscreen layer's origin to the frame, while probing.
    layer_shift: (f32, f32),
    /// Visible window `[left, top, right, bottom]` (composition px) of the layers being probed.
    layer_clip: [f32; 4],
    /// Number of offscreen layers (of any kind) currently being drawn into; blend modes need 0.
    isolated: u32,
    /// The root's `filter` color functions, graded onto every finished frame.
    grade: paint::ColorChain,
    /// Per-channel sums of the sub-frame samples while a motion-blurred frame is accumulated.
    shutter_sums: Vec<u16>,
    /// Composition time of the states in `current` (animated clip paths are evaluated at it).
    time: f64,
    /// Element whose clip-path or mask layer is being drawn, so it is not masked twice.
    mask_guard: Option<usize>,
    /// Reduced halo planes of `data-hf-bloom`, reused across frames.
    bloom_scratch: paint::BloomScratch,
    /// Frame rate of the frame being drawn; per-element shutter windows are fractions of it.
    fps: u32,
    /// Elements with `data-hf-motion-blur`, their subtrees, sample plans, and scratch buffers.
    layer_blur: layer_blur::LayerBlur,
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
        let (width, height) = size.dimensions(composition.width, composition.height)?;
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
        allocation::rgba_bytes(width as usize, height as usize, "frame")?;
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
            videos: HashMap::new(),
            fonts,
            stats: RenderStats::default(),
            probe: None,
            layer_depth: 0,
            layer_shift: (0.0, 0.0),
            layer_clip: [
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::INFINITY,
            ],
            isolated: 0,
            grade: paint::ColorChain::new(&composition.grade),
            shutter_sums: Vec::new(),
            time: 0.0,
            mask_guard: None,
            bloom_scratch: paint::BloomScratch::default(),
            fps: shutter::SNAPSHOT_FPS,
            layer_blur: layer_blur::LayerBlur::new(composition),
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

    pub fn composition(&self) -> &'a Composition {
        self.composition
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
        let time = frame as f64 / fps as f64;
        crate::profile::time(crate::profile::Phase::Raster, || {
            self.render_shuttered(time, fps).map(|_| ())
        })?;
        Ok((&self.surface.pixels, self.stats.rasterized))
    }

    /// A still at `time` seconds, the path behind `aether_snapshot`. Without `fps` it is
    /// [`Self::render_at`] (shutter windows assume [`SNAPSHOT_FPS`]). With `fps`, a time on a frame
    /// boundary (`time * fps` within 1e-6 of a whole frame `N`) is drawn exactly as
    /// `render_frame(N, fps)`, the CLI's `render --frame N --fps F` path, and any other time uses
    /// `fps` for the motion-blur shutter window.
    pub fn render_still(&mut self, time: f64, fps: Option<u32>) -> Result<&[u8]> {
        let Some(fps) = fps else {
            return self.render_at(time);
        };
        let fps = validate_fps(u64::from(fps), "fps")?;
        if !time.is_finite() || time < 0.0 || time >= self.composition.duration {
            return Err(anyhow!("time is outside the composition duration"));
        }
        let frames = time * f64::from(fps);
        let nearest = frames.round();
        if (frames - nearest).abs() <= 1e-6 && (nearest as usize) < self.frame_count(fps) {
            return self.render_frame(nearest as usize, fps);
        }
        crate::profile::time(crate::profile::Phase::Raster, || {
            self.render_shuttered(time, fps).map(|_| ())
        })?;
        Ok(&self.surface.pixels)
    }

    pub fn render_at(&mut self, time: f64) -> Result<&[u8]> {
        if !time.is_finite() || time < 0.0 || time >= self.composition.duration {
            return Err(anyhow!("time is outside the composition duration"));
        }
        crate::profile::time(crate::profile::Phase::Raster, || {
            self.render_shuttered(time, shutter::SNAPSHOT_FPS)
                .map(|_| ())
        })?;
        Ok(&self.surface.pixels)
    }

    /// Fills `current` with every element's state at `time`.
    pub(in crate::render) fn sample_states(&mut self, time: f64) {
        self.time = time;
        let mut states = std::mem::take(&mut self.current);
        states_at(self.composition, time, &mut states);
        self.current = states;
    }

    /// Draws the frame at `time` with no root shutter (per-element motion blur still applies).
    pub(in crate::render) fn render_time(&mut self, time: f64) -> Result<&[u8]> {
        self.sample_states(time);
        let blurring = self.plan_layer_blur(time);
        // Grain is reseeded every frame, so identical element states no longer mean identical pixels.
        // A blurred element moving inside its shutter window also changes pixels.
        if !blurring && self.previous == self.current && self.composition.grain <= 0.0 {
            self.stats.reused += 1;
            return Ok(&self.surface.pixels);
        }
        self.previous.clear();
        self.draw_scene()?;
        self.finish_frame(time);
        std::mem::swap(&mut self.previous, &mut self.current);
        if blurring {
            // The pixels average several instants, so they match no single set of states.
            self.previous.clear();
        }
        self.stats.rasterized += 1;
        Ok(&self.surface.pixels)
    }

    /// Clears what the last frame painted and draws every element in its `current` state.
    pub(in crate::render) fn draw_scene(&mut self) -> Result<()> {
        self.reset_background();
        let root = Rect {
            x: 0.0,
            y: 0.0,
            w: self.composition.width as f32,
            h: self.composition.height as f32,
        };
        self.draw_node(0, root, 1.0, None)
    }

    /// Frame-wide passes after drawing: the root's color grade, then vignette and grain.
    pub(in crate::render) fn finish_frame(&mut self, time: f64) {
        if !self.grade.is_identity() {
            self.grade.grade_frame(&mut self.surface.pixels);
            self.surface.mark_all_painted();
        }
        let seed = (time * 1000.0).round() as u32;
        self.surface
            .apply_finish(self.composition.vignette, self.composition.grain, seed);
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

/// Fills `states` with every element's state at `time` (parents come before their children).
pub(in crate::render) fn states_at(
    composition: &Composition,
    time: f64,
    states: &mut Vec<FrameState>,
) {
    states.clear();
    for element in &composition.elements {
        let parent_alpha = element.parent.map_or(1.0, |idx| states[idx].alpha);
        let alpha = if parent_alpha > 0.0 {
            parent_alpha * Renderer::active_opacity(element, time)
        } else {
            0.0
        };
        let state = FrameState::at(element, time, alpha);
        states.push(if state.alpha > 0.0 {
            state
        } else {
            FrameState::default()
        });
    }
}

#[cfg(test)]
mod size_tests {
    use super::OutputSize;

    #[test]
    fn presets_follow_the_orientation_of_the_composition() {
        assert_eq!(
            OutputSize::FullHd.dimensions(1920, 1080).unwrap(),
            (1920, 1080)
        );
        assert_eq!(
            OutputSize::FullHd.dimensions(1080, 1920).unwrap(),
            (1080, 1920)
        );
        assert_eq!(OutputSize::Uhd.dimensions(1280, 720).unwrap(), (3840, 2160));
        assert_eq!(OutputSize::Uhd.dimensions(540, 960).unwrap(), (2160, 3840));
    }

    #[test]
    fn other_aspect_ratios_are_rejected() {
        let error = OutputSize::FullHd.dimensions(1000, 1000).unwrap_err();
        assert!(error.to_string().contains("9:16"), "{error}");
        assert!(OutputSize::Uhd.dimensions(1080, 1080).is_err());
    }
}
