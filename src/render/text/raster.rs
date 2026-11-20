//! Stable glyph rasterization and run caching; fractional translation and zoom resample fixed coverage.

use crate::model::{Color, Content};
use crate::render::geometry::{PixelRect, Rect};
use crate::render::layout::in_flow;
use crate::render::paint::GradientPaint;
#[cfg(test)]
use crate::render::paint::Surface;
use crate::render::text::block::TextBlock;
use crate::render::text::oblique::{slant, SKEW};
use crate::render::text::props::{style_index, TextProps, Underlay};
use crate::render::text::shadow::{BlockLayout, ShadowMask};
use crate::render::text::stroke::{stroke_glyph, StrokeMask};
use crate::render::Renderer;
use anyhow::Result;

pub(in crate::render) struct Glyph {
    pub(super) metrics: fontdue::Metrics,
    pub(super) coverage: Vec<u8>,
    /// Index into [`TextRun::paints`].
    pub(super) style: usize,
    /// `-webkit-text-stroke` coverage, when the glyph's run has a stroke.
    pub(super) stroke: Option<StrokeMask>,
}

/// How one run of glyphs is painted (see `props::RunStyle`).
pub(super) struct GlyphPaint {
    underlay: Option<Underlay>,
    fill: Color,
    stroke: Color,
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
    /// One entry per run style: the element's own text first, then each inline span.
    paints: Vec<GlyphPaint>,
    /// Plain text of one color: painted exactly as before rich text existed.
    plain: bool,
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
        let spans = &element.spans;
        let styles = self.run_styles(idx);
        let mut skews = Vec::with_capacity(styles.len());
        for style in &styles {
            // Loads the face, and says whether it must be slanted.
            let synthetic = self.fonts.synthetic_oblique(style.font())?;
            skews.push(if synthetic { SKEW } else { 0.0 });
        }
        let kerning = self.text_kerning(idx, props.kerning, &styles)?;
        let face_data = styles
            .iter()
            .map(|style| {
                (style.stroke_width > 0.0)
                    .then(|| self.fonts.face_data(style.font()))
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        let faces: Vec<Option<ttf_parser::Face>> = face_data
            .iter()
            .map(|data| {
                data.as_ref()
                    .and_then(|(bytes, index)| ttf_parser::Face::parse(bytes, *index).ok())
            })
            .collect();
        let fonts: Vec<&fontdue::Font> = styles
            .iter()
            .map(|style| {
                self.fonts
                    .loaded(style.font())
                    .expect("fonts were loaded above")
            })
            .collect();
        let unit = self.scale * zoom;
        let mut glyphs = Vec::new();
        let mut lines = Vec::with_capacity(block.lines.len());
        for line in &block.lines {
            let first = glyphs.len();
            for (offset, ch) in text[line.start..line.end].char_indices() {
                let s = style_index(spans, line.start + offset);
                let (mut metrics, mut coverage) = fonts[s].rasterize(ch, size);
                metrics.advance_width += if s == 0 {
                    spacing
                } else {
                    styles[s].letter_spacing * unit
                };
                if let Some(kerning) = &kerning {
                    metrics.advance_width += kerning[line.start + offset] * size;
                }
                if skews[s] != 0.0 {
                    coverage = slant(&mut metrics, &coverage);
                }
                let stroke = faces[s].as_ref().and_then(|face| {
                    stroke_glyph(face, ch, size, styles[s].stroke_width * unit, skews[s])
                });
                glyphs.push(Glyph {
                    metrics,
                    coverage,
                    style: s,
                    stroke,
                });
            }
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
        let plain = styles.len() == 1 && styles[0].is_plain();
        let paints = styles
            .into_iter()
            .map(|style| GlyphPaint {
                underlay: style.underlay,
                fill: style.fill,
                stroke: style.stroke,
            })
            .collect();
        Ok(TextRun {
            glyphs,
            lines,
            size,
            line_height,
            paints,
            plain,
            center_x,
            center_y,
            right,
            shadow,
        })
    }

    /// Paints the text of element `idx` inside `rect` (layout units), wrapped to its width.
    /// `paint_box` (layout units) is the area a `background-clip: text` gradient spans.
    pub(in crate::render) fn draw_text(
        &mut self,
        idx: usize,
        rect: Rect,
        paint_box: Rect,
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
        let key = (
            self.text_keys[idx] as usize,
            1.0_f32.to_bits(),
            block.limit_key,
            width_px,
        );
        let uncached;
        let run = if let Some(run) = self.text_runs.get(&key) {
            run
        } else {
            let prepared = self.prepare_text(idx, 1.0, &block, &props, width_px)?;
            let bytes = prepared
                .glyphs
                .iter()
                .map(|glyph| {
                    std::mem::size_of::<Glyph>()
                        + glyph.coverage.len()
                        + glyph.stroke.as_ref().map_or(0, |s| s.coverage.len())
                })
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
            let (sx, sy) = self.layer_shift;
            let clip = self.layer_clip;
            let rect = [
                (x0 / scale + sx).max(clip[0]),
                (top / scale + sy).max(clip[1]),
                (x1 / scale + sx).min(clip[2]),
                ((top + total) / scale + sy).min(clip[3]),
            ];
            if let (true, true, Some(boxes)) = (
                x0 < x1,
                rect[0] < rect[2] && rect[1] < rect[3],
                self.probe.as_mut(),
            ) {
                boxes.push(crate::render::probe::TextBox { element: idx, rect });
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
        let element_box = [
            paint_box.x * scale,
            paint_box.y * scale,
            (paint_box.x + paint_box.w) * scale,
            (paint_box.y + paint_box.h) * scale,
        ];
        for (index, line) in run.lines.iter().enumerate() {
            let pen = rect.x * scale
                + pen_offset(run.center_x, run.right, rect.w * scale, line.advance * zoom);
            let baseline = top + baseline_offset(index, run.line_height, run.size) * zoom;
            let glyphs = &run.glyphs[line.first..line.last];
            let dimensions = (self.surface.width, self.surface.height);
            if run.plain {
                paint_scaled_line(
                    &mut self.surface.pixels,
                    dimensions,
                    &mut painted,
                    glyphs,
                    (pen, baseline),
                    (run.paints[0].fill, alpha),
                    zoom,
                );
                continue;
            }
            let line_top = top + index as f32 * run.line_height * zoom;
            let rich = RichLine {
                pen,
                baseline,
                band: (line_top, line_top + run.line_height * zoom),
                element_box,
                alpha,
                zoom,
            };
            rich.paint(
                &mut self.surface.pixels,
                dimensions,
                &mut painted,
                glyphs,
                &run.paints,
            );
        }
        self.surface.painted = painted;
        Ok(())
    }
}

/// Placement of one line of rich text in output pixels.
struct RichLine {
    pen: f32,
    baseline: f32,
    /// Top and bottom of the line box, the height of a run's own gradient.
    band: (f32, f32),
    /// Padding box of the element, the extent of its `background-clip: text` gradient.
    element_box: [f32; 4],
    alpha: f32,
    zoom: f32,
}

impl RichLine {
    /// Paints each glyph's underlay and fill, then every stroke on top, as CSS does for a text run.
    fn paint(
        &self,
        pixels: &mut [u8],
        dimensions: (u32, u32),
        painted: &mut Option<PixelRect>,
        glyphs: &[Glyph],
        paints: &[GlyphPaint],
    ) {
        let zoom = self.zoom;
        // A run's own gradient spans the run's extent on this line.
        let mut extents: Vec<Option<(f32, f32)>> = vec![None; paints.len()];
        let mut pen = self.pen;
        for glyph in glyphs {
            let next = pen + glyph.metrics.advance_width * zoom;
            let extent = &mut extents[glyph.style];
            *extent = Some(extent.map_or((pen, next), |(a, b)| (a.min(pen), b.max(next))));
            pen = next;
        }
        let gradients: Vec<Option<GradientPaint>> = paints
            .iter()
            .zip(&extents)
            .map(|(paint, extent)| match (&paint.underlay, extent) {
                (Some(Underlay::Gradient { gradient, run }), Some((x0, x1))) => {
                    let edges = if *run {
                        [*x0, self.band.0, *x1, self.band.1]
                    } else {
                        self.element_box
                    };
                    Some(GradientPaint::new(gradient, edges))
                }
                _ => None,
            })
            .collect();
        let mut pen = self.pen;
        for glyph in glyphs {
            let metrics = glyph.metrics;
            let paint = &paints[glyph.style];
            let mask = (&glyph.coverage[..], (metrics.width, metrics.height));
            let origin = (
                pen + metrics.xmin as f32 * zoom,
                self.baseline - (metrics.ymin as f32 + metrics.height as f32) * zoom,
            );
            match (&paint.underlay, &gradients[glyph.style]) {
                (Some(Underlay::Solid(color)), _) => super::mask::paint_mask(
                    pixels,
                    dimensions,
                    painted,
                    mask.0,
                    mask.1,
                    origin,
                    (*color, self.alpha),
                    zoom,
                ),
                (_, Some(gradient)) => super::mask::paint_mask_with(
                    pixels,
                    dimensions,
                    painted,
                    mask.0,
                    mask.1,
                    origin,
                    (|x, y| gradient.color_at(x, y), self.alpha),
                    zoom,
                ),
                _ => {}
            }
            if paint.fill.a > 0 {
                super::mask::paint_mask(
                    pixels,
                    dimensions,
                    painted,
                    mask.0,
                    mask.1,
                    origin,
                    (paint.fill, self.alpha),
                    zoom,
                );
            }
            pen += metrics.advance_width * zoom;
        }
        let mut pen = self.pen;
        for glyph in glyphs {
            if let Some(stroke) = &glyph.stroke {
                super::mask::paint_mask(
                    pixels,
                    dimensions,
                    painted,
                    &stroke.coverage,
                    (stroke.width, stroke.height),
                    (
                        pen + stroke.left as f32 * zoom,
                        self.baseline + stroke.top as f32 * zoom,
                    ),
                    (paints[glyph.style].stroke, self.alpha),
                    zoom,
                );
            }
            pen += glyph.metrics.advance_width * zoom;
        }
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
                    paints: Vec::new(),
                    plain: true,
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
            style: 0,
            stroke: None,
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
