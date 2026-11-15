//! Glyph rasterization, run caching, and text painting.

use crate::model::Color;
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::layout::in_flow;
use crate::render::Renderer;
use anyhow::Result;

pub(in crate::render) struct Glyph {
    metrics: fontdue::Metrics,
    coverage: Vec<u8>,
}

pub(in crate::render) struct TextRun {
    glyphs: Vec<Glyph>,
    advance: f32,
    size: f32,
    color: Color,
    center_x: bool,
    center_y: bool,
    right: bool,
}

impl<'a> Renderer<'a> {
    pub(in crate::render) fn prepare_text(
        &mut self,
        text: &str,
        idx: usize,
        zoom: f32,
    ) -> Result<TextRun> {
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
        let font = self.fonts.get(family)?;
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

    pub(in crate::render) fn draw_text(
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
            let Some(bounds) = clip_pixels(
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
