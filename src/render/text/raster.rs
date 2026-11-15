//! Glyph rasterization, run caching, and painting of laid-out text lines.

use crate::model::{Color, Content};
use crate::render::geometry::{clip_pixels, PixelRect, Rect};
use crate::render::layout::in_flow;
use crate::render::text::block::TextBlock;
use crate::render::Renderer;
use anyhow::Result;

pub(in crate::render) struct Glyph {
    metrics: fontdue::Metrics,
    coverage: Vec<u8>,
}

/// Glyphs of one line: `glyphs[first..last]` and their total advance, in pixels.
struct RunLine {
    first: usize,
    last: usize,
    advance: f32,
}

/// Rasterized text block at one paint size, ready to blit.
pub(in crate::render) struct TextRun {
    glyphs: Vec<Glyph>,
    lines: Vec<RunLine>,
    /// Font size and baseline distance in pixels.
    size: f32,
    line_height: f32,
    color: Color,
    center_x: bool,
    center_y: bool,
    right: bool,
}

impl<'a> Renderer<'a> {
    fn prepare_text(&mut self, idx: usize, zoom: f32, block: &TextBlock) -> Result<TextRun> {
        let props = self.text_props(idx);
        let size = props.size * self.scale * zoom;
        let element = &self.composition.elements[idx];
        let text = match &element.content {
            Content::Text(text) => text.as_str(),
            _ => "",
        };
        let parent_style = element
            .parent
            .map(|parent| &self.composition.elements[parent]);
        // Flex items are placed by the layout; only out-of-flow children keep the legacy centering.
        let parent_flex = parent_style.and_then(|p| p.style.display.as_deref()) == Some("flex")
            && !in_flow(self.composition, idx);
        let center_x = props.align == "center"
            || (parent_flex
                && parent_style.and_then(|p| p.style.justify_content.as_deref()) == Some("center"));
        let center_y = parent_flex
            && parent_style.and_then(|p| p.style.align_items.as_deref()) == Some("center");
        let font = self.fonts.get(props.family)?;
        let mut glyphs = Vec::new();
        let mut lines = Vec::with_capacity(block.lines.len());
        for line in &block.lines {
            let first = glyphs.len();
            glyphs.extend(text[line.start..line.end].chars().map(|ch| {
                let (metrics, coverage) = font.rasterize(ch, size);
                Glyph { metrics, coverage }
            }));
            let advance = glyphs[first..]
                .iter()
                .map(|glyph| glyph.metrics.advance_width)
                .sum();
            lines.push(RunLine {
                first,
                last: glyphs.len(),
                advance,
            });
        }
        Ok(TextRun {
            glyphs,
            lines,
            size,
            line_height: block.line_height * self.scale * zoom,
            color: props.color,
            center_x,
            center_y,
            right: props.align == "right",
        })
    }

    /// Paints the text of element `idx` inside `rect` (layout units), wrapped to its width.
    pub(in crate::render) fn draw_text(
        &mut self,
        idx: usize,
        rect: Rect,
        alpha: f32,
        zoom: f32,
    ) -> Result<()> {
        let block = self.text_block(idx, Some(rect.w / zoom))?;
        let key = (idx, zoom.to_bits(), block.limit_key);
        let uncached;
        let run = if let Some(run) = self.text_runs.get(&key) {
            run
        } else {
            let prepared = self.prepare_text(idx, zoom, &block)?;
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
        let total = run.line_height * run.lines.len() as f32;
        let top = rect.y * scale
            + if run.center_y {
                (rect.h * scale - total) / 2.0
            } else {
                0.0
            };
        let mut painted = self.painted;
        for (index, line) in run.lines.iter().enumerate() {
            let pen = rect.x * scale
                + if run.center_x {
                    (rect.w * scale - line.advance) / 2.0
                } else if run.right {
                    rect.w * scale - line.advance
                } else {
                    0.0
                };
            // Leading is split evenly above and below the glyphs.
            let baseline = top
                + index as f32 * run.line_height
                + (run.line_height - run.size) / 2.0
                + run.size;
            paint_line(
                &mut self.pixels,
                (self.width, self.height),
                &mut painted,
                &run.glyphs[line.first..line.last],
                (pen, baseline),
                (run.color, alpha),
            );
        }
        self.painted = painted;
        Ok(())
    }
}

/// Blits one line of glyphs, starting at pen x and sitting on the baseline y.
fn paint_line(
    pixels: &mut [u8],
    (width, height): (u32, u32),
    painted: &mut Option<PixelRect>,
    glyphs: &[Glyph],
    (mut pen, baseline): (f32, f32),
    (color, alpha): (Color, f32),
) {
    for glyph in glyphs {
        let metrics = glyph.metrics;
        let gx = (pen + metrics.xmin as f32).round() as i32;
        let gy = (baseline - metrics.ymin as f32 - metrics.height as f32).round() as i32;
        pen += metrics.advance_width;
        let Some(bounds) = clip_pixels(
            (width, height),
            gx,
            gy,
            gx.saturating_add(metrics.width as i32),
            gy.saturating_add(metrics.height as i32),
        ) else {
            continue;
        };
        *painted = Some(painted.map_or(bounds, |area| area.union(bounds)));
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let offset = (y as i64 - i64::from(gy)) as usize * metrics.width
                    + (x as i64 - i64::from(gx)) as usize;
                let coverage = glyph.coverage[offset] as f32 / 255.0;
                if coverage > 0.0 {
                    Renderer::blend_at(
                        pixels,
                        width,
                        height,
                        x as i32,
                        y as i32,
                        color,
                        alpha * coverage,
                    );
                }
            }
        }
    }
}
