//! Stable glyph rasterization and run caching; fractional translation and zoom resample fixed coverage.

use crate::model::{Color, Content};
use crate::render::geometry::{PixelRect, Rect};
use crate::render::layout::in_flow;
#[cfg(test)]
use crate::render::paint::Surface;
use crate::render::text::block::TextBlock;
use crate::render::text::props::TextProps;
use crate::render::text::shadow::{BlockLayout, ShadowMask};
use crate::render::Renderer;
use anyhow::Result;

pub(in crate::render) struct Glyph {
    pub(super) metrics: fontdue::Metrics,
    pub(super) coverage: Vec<u8>,
}

/// Glyphs of one line: `glyphs[first..last]` and their total advance, in pixels.
pub(super) struct RunLine {
    pub(super) first: usize,
    pub(super) last: usize,
    pub(super) advance: f32,
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
    shadow: Option<ShadowMask>,
}

/// Horizontal start of a line inside a block `width` pixels wide.
pub(super) fn pen_offset(center_x: bool, right: bool, width: f32, advance: f32) -> f32 {
    if center_x {
        (width - advance) / 2.0
    } else if right {
        width - advance
    } else {
        0.0
    }
}

/// Baseline of line `index` below the block top; leading is split evenly above and below.
pub(super) fn baseline_offset(index: usize, line_height: f32, size: f32) -> f32 {
    index as f32 * line_height + (line_height - size) / 2.0 + size
}

impl<'a> Renderer<'a> {
    /// Rasterizes the block; `width_px` is the box width in paint pixels, used to align shadow lines.
    fn prepare_text(
        &mut self,
        idx: usize,
        zoom: f32,
        block: &TextBlock,
        props: &TextProps,
        width_px: i32,
    ) -> Result<TextRun> {
        let size = props.size * self.scale * zoom;
        let spacing = props.letter_spacing * self.scale * zoom;
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
        let font = self.fonts.get(props.family, props.weight)?;
        let mut glyphs = Vec::new();
        let mut lines = Vec::with_capacity(block.lines.len());
        for line in &block.lines {
            let first = glyphs.len();
            glyphs.extend(text[line.start..line.end].chars().map(|ch| {
                let (mut metrics, coverage) = font.rasterize(ch, size);
                metrics.advance_width += spacing;
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
        let line_height = block.line_height * self.scale * zoom;
        let right = props.align == "right";
        let shadow = match props.shadow {
            Some(shadow) => {
                let layout = BlockLayout {
                    width: width_px as f32,
                    size,
                    line_height,
                    center_x,
                    right,
                };
                ShadowMask::build(&glyphs, &lines, &layout, shadow, self.scale * zoom)?
            }
            None => None,
        };
        Ok(TextRun {
            glyphs,
            lines,
            size,
            line_height,
            color: props.color,
            center_x,
            center_y,
            right,
            shadow,
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
        let props = self.text_props(idx);
        // Only shadows depend on the box width (through alignment), so only they key on it.
        let width_px = if props.shadow.is_some() {
            (rect.w * self.scale / zoom).round() as i32
        } else {
            0
        };
        // Rasterize at the composition's output resolution, independent of animated zoom.
        // Re-rasterizing every zoom step changes glyph metrics discretely and makes type wobble.
        let key = (idx, 1.0_f32.to_bits(), block.limit_key, width_px);
        let uncached;
        let run = if let Some(run) = self.text_runs.get(&key) {
            run
        } else {
            let prepared = self.prepare_text(idx, 1.0, &block, &props, width_px)?;
            let bytes = prepared
                .glyphs
                .iter()
                .map(|glyph| std::mem::size_of::<Glyph>() + glyph.coverage.len())
                .sum::<usize>()
                + prepared.shadow.as_ref().map_or(0, ShadowMask::bytes);
            if self.text_runs.len() < 4096
                && self.text_cache_bytes.saturating_add(bytes) <= 32 * 1024 * 1024
            {
                self.text_cache_bytes += bytes;
                self.text_runs.entry(key).or_insert(prepared)
            } else {
                uncached = prepared;
                &uncached
            }
        };
        let scale = self.scale;
        let total = run.line_height * zoom * run.lines.len() as f32;
        let top = rect.y * scale
            + if run.center_y {
                (rect.h * scale - total) / 2.0
            } else {
                0.0
            };
        if self.probe.is_some() && self.layer_depth == 0 {
            let (mut x0, mut x1) = (f32::INFINITY, f32::NEG_INFINITY);
            for line in &run.lines {
                let pen = rect.x * scale
                    + pen_offset(run.center_x, run.right, rect.w * scale, line.advance * zoom);
                x0 = x0.min(pen);
                x1 = x1.max(pen + line.advance * zoom);
            }
            if let (true, Some(boxes)) = (x0 < x1, self.probe.as_mut()) {
                boxes.push(crate::render::probe::TextBox {
                    element: idx,
                    rect: [x0 / scale, top / scale, x1 / scale, (top + total) / scale],
                });
            }
        }
        let mut painted = self.surface.painted;
        if let Some(shadow) = &run.shadow {
            let origin = (rect.x * scale, top);
            shadow.paint(
                &mut self.surface.pixels,
                (self.surface.width, self.surface.height),
                &mut painted,
                origin,
                alpha,
                zoom,
            );
        }
        for (index, line) in run.lines.iter().enumerate() {
            let pen = rect.x * scale
                + pen_offset(run.center_x, run.right, rect.w * scale, line.advance * zoom);
            let baseline = top + baseline_offset(index, run.line_height, run.size) * zoom;
            paint_scaled_line(
                &mut self.surface.pixels,
                (self.surface.width, self.surface.height),
                &mut painted,
                &run.glyphs[line.first..line.last],
                (pen, baseline),
                (run.color, alpha),
                zoom,
            );
        }
        self.surface.painted = painted;
        Ok(())
    }
}

/// Blits one line of glyphs, starting at pen x and sitting on the baseline y.
#[cfg(test)]
fn paint_line(
    pixels: &mut [u8],
    (width, height): (u32, u32),
    painted: &mut Option<PixelRect>,
    glyphs: &[Glyph],
    (pen, baseline): (f32, f32),
    (color, alpha): (Color, f32),
) {
    paint_scaled_line(
        pixels,
        (width, height),
        painted,
        glyphs,
        (pen, baseline),
        (color, alpha),
        1.0,
    );
}

/// Keeps glyph shapes and advances stable during zooms; resamples their coverage at fractional positions.
fn paint_scaled_line(
    pixels: &mut [u8],
    dimensions: (u32, u32),
    painted: &mut Option<PixelRect>,
    glyphs: &[Glyph],
    (mut pen, baseline): (f32, f32),
    color: (Color, f32),
    zoom: f32,
) {
    for glyph in glyphs {
        let metrics = glyph.metrics;
        super::mask::paint_mask(
            pixels,
            dimensions,
            painted,
            &glyph.coverage,
            (metrics.width, metrics.height),
            (
                pen + metrics.xmin as f32 * zoom,
                baseline - (metrics.ymin as f32 + metrics.height as f32) * zoom,
            ),
            color,
            zoom,
        );
        pen += metrics.advance_width * zoom;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_run_entries_cannot_bypass_the_metadata_limit() {
        let path = std::env::temp_dir().join(format!(
            "aether-text-cache-limit-{}.html",
            std::process::id()
        ));
        std::fs::write(&path,r#"<main data-composition-id="test" data-width="16" data-height="16" data-duration="1"><div style="font-size:8px">A</div></main>"#).unwrap();
        let composition = crate::parse::parse_file(&path).unwrap();
        let idx = composition
            .elements
            .iter()
            .position(|e| matches!(e.content, Content::Text(_)))
            .unwrap();
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        for n in 0..4096 {
            renderer.text_runs.insert(
                (usize::MAX, n, 0, 0),
                TextRun {
                    glyphs: Vec::new(),
                    lines: Vec::new(),
                    size: 1.0,
                    line_height: 1.0,
                    color: Color::WHITE,
                    center_x: false,
                    center_y: false,
                    right: false,
                    shadow: None,
                },
            );
        }
        renderer
            .draw_text(
                idx,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                },
                1.0,
                1.0,
            )
            .unwrap();
        assert_eq!(renderer.text_runs.len(), 4096);
        assert_eq!(renderer.text_cache_bytes, 0);
        assert!(renderer.surface.painted.is_some());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn clipped_glyph_rows_match_coordinate_blending() {
        let glyph = Glyph {
            metrics: fontdue::Metrics {
                width: 32,
                height: 16,
                xmin: -3,
                ymin: 0,
                advance_width: 32.0,
                ..Default::default()
            },
            coverage: (0..512).map(|i| i as u8).collect(),
        };
        for alpha in [0.0, 0.17, 1.0] {
            for opacity in [0, 137, 255] {
                let color = Color {
                    r: 37,
                    g: 183,
                    b: 247,
                    a: opacity,
                };
                let mut actual: Vec<u8> = (0..24 * 12 * 4).map(|i| i as u8).collect();
                let mut expected = actual.clone();
                let mut painted = None;
                paint_line(
                    &mut actual,
                    (24, 12),
                    &mut painted,
                    std::slice::from_ref(&glyph),
                    (0.0, 14.0),
                    (color, alpha),
                );
                for y in 0..16 {
                    for x in 0..32 {
                        let coverage = glyph.coverage[y * 32 + x] as f32 / 255.0;
                        if coverage > 0.0 {
                            Surface::blend_at(
                                &mut expected,
                                24,
                                12,
                                x as i32 - 3,
                                y as i32 - 2,
                                color,
                                alpha * coverage,
                            );
                        }
                    }
                }
                assert_eq!(actual, expected, "alpha {alpha}, color opacity {opacity}");
            }
        }
    }
}
