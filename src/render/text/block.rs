//! Text layout in composition px: where lines break and how large the text block is.
//!
//! Breaking is decided at the unscaled font size, never at the painted size, so lines stay put
//! under `--scale`, `--size`, and animated CSS `scale`. Results are cached per element and limit.

use crate::model::Content;
use crate::render::text::props::style_index;
use crate::render::text::wrap::{break_lines, Line};
use crate::render::Renderer;
use anyhow::Result;
use std::sync::Arc;

/// Limits are keyed in 1/64 px; `None` (no limit) uses this sentinel.
const NO_LIMIT: i64 = i64::MAX;
/// Entries kept before the cache restarts; keys only multiply when widths animate.
const MAX_BLOCKS: usize = 4096;

pub(in crate::render) struct TextBlock {
    pub(in crate::render) lines: Vec<Line>,
    /// Width of the text on a single line.
    pub(in crate::render) max_content: f32,
    pub(in crate::render) line_height: f32,
    pub(in crate::render) nowrap: bool,
    /// Cache key of the wrap limit; also part of the painted-run key.
    pub(in crate::render) limit_key: i64,
}

impl TextBlock {
    pub(in crate::render) fn height(&self) -> f32 {
        self.lines.len() as f32 * self.line_height
    }

    /// Border-box-independent width of a shrink-to-fit block: content size, capped by `limit`.
    pub(in crate::render) fn fit_width(&self, limit: Option<f32>) -> f32 {
        match limit {
            Some(limit) if !self.nowrap => self.max_content.min(limit),
            _ => self.max_content,
        }
    }
}

impl<'a> Renderer<'a> {
    /// Lays out the text of element `idx` for a content width of `limit` composition px.
    pub(in crate::render) fn text_block(
        &mut self,
        idx: usize,
        limit: Option<f32>,
    ) -> Result<Arc<TextBlock>> {
        let props = self.text_props(idx);
        let quantized = limit
            .filter(|_| !props.nowrap)
            .map(|limit| (limit.max(0.0) * 64.0).round() as i64);
        let key = (idx, quantized.unwrap_or(NO_LIMIT));
        if let Some(block) = self.text_blocks.get(&key) {
            return Ok(Arc::clone(block));
        }
        let text = match &self.composition.elements[idx].content {
            Content::Text(text) => text.as_str(),
            _ => "",
        };
        let spans = &self.composition.elements[idx].spans;
        let styles = self.run_styles(idx);
        for style in &styles {
            self.fonts.get(style.family, style.weight)?;
        }
        let fonts: Vec<&fontdue::Font> = styles
            .iter()
            .map(|style| {
                self.fonts
                    .loaded(style.family, style.weight)
                    .expect("fonts were loaded above")
            })
            .collect();
        let advance = |offset: usize, ch: char| {
            let s = style_index(spans, offset);
            fonts[s].metrics(ch, props.size).advance_width + styles[s].letter_spacing
        };
        // One 1/64 px of slack keeps a block that exactly fills its box from re-wrapping.
        let slack_limit = quantized.map(|q| q.saturating_add(1) as f32 / 64.0);
        let mut max_content = 0.0_f32;
        let mut base = 0;
        for paragraph in text.split('\n') {
            let width: f32 = paragraph
                .char_indices()
                .map(|(i, ch)| advance(base + i, ch))
                .sum();
            max_content = max_content.max(width);
            base += paragraph.len() + 1;
        }
        let block = Arc::new(TextBlock {
            lines: break_lines(text, slack_limit, advance),
            max_content,
            line_height: props.line_height,
            nowrap: props.nowrap,
            limit_key: key.1,
        });
        if self.text_blocks.len() >= MAX_BLOCKS {
            self.text_blocks.clear();
        }
        self.text_blocks.insert(key, Arc::clone(&block));
        Ok(block)
    }
}
