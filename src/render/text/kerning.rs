//! Pair kerning for `font-kerning: normal`, read from a font's OpenType data.
//!
//! The adjustment of a glyph pair comes from the GPOS `kern` feature (pair adjustment lookups, both
//! the glyph-pair and the class-pair formats, applied like a shaping engine: in each lookup the first
//! subtable that covers the pair wins, and the lookups add up), or, for fonts without GPOS kerning,
//! from the legacy `kern` table (horizontal format 0 and 2 subtables). Values are in em units, so the
//! same pair scales exactly with the font size; contextual, vertical, and device adjustments are not
//! applied. Results are memoized per character pair, and kerning is a pure function of the font
//! bytes, so it stays deterministic across machines that resolve the same font.

use crate::model::{Composition, Content};
use crate::render::text::props::{style_index, RunStyle};
use crate::render::Renderer;
use anyhow::Result;
use std::collections::HashMap;
use ttf_parser::gpos::{PairAdjustment, PositioningSubtable};
use ttf_parser::{Face, GlyphId, Tag};

/// Distinct character pairs remembered per font before the memo restarts.
const MAX_PAIRS: usize = 16_384;

/// Kerning of one font face.
pub(in crate::render) struct Kerning {
    /// GPOS lookups of the `kern` feature (sorted, deduplicated), or empty.
    lookups: Vec<u16>,
    /// Whether to fall back to the legacy `kern` table.
    legacy: bool,
    units_per_em: f32,
    memo: HashMap<(char, char), f32>,
}

impl Kerning {
    /// Reads which kerning data `face` has; cheap, nothing is evaluated until a pair is asked for.
    pub(in crate::render) fn new(face: &Face) -> Self {
        let kern = Tag::from_bytes(b"kern");
        let mut lookups: Vec<u16> = face
            .tables()
            .gpos
            .map(|gpos| {
                gpos.features
                    .into_iter()
                    .filter(|feature| feature.tag == kern)
                    .flat_map(|feature| feature.lookup_indices.into_iter())
                    .collect()
            })
            .unwrap_or_default();
        lookups.sort_unstable();
        lookups.dedup();
        Self {
            legacy: lookups.is_empty() && face.tables().kern.is_some(),
            lookups,
            units_per_em: f32::from(face.units_per_em().max(1)),
            memo: HashMap::new(),
        }
    }

    /// Whether the font has any kerning data.
    pub(in crate::render) fn is_empty(&self) -> bool {
        self.lookups.is_empty() && !self.legacy
    }

    /// Advance adjustment between `left` and `right`, in em (multiply by the font size in px).
    pub(in crate::render) fn pair(&mut self, face: &Face, left: char, right: char) -> f32 {
        if self.is_empty() {
            return 0.0;
        }
        if let Some(&value) = self.memo.get(&(left, right)) {
            return value;
        }
        let value = match (face.glyph_index(left), face.glyph_index(right)) {
            (Some(a), Some(b)) => f32::from(self.units(face, a, b)) / self.units_per_em,
            _ => 0.0,
        };
        if self.memo.len() >= MAX_PAIRS {
            self.memo.clear();
        }
        self.memo.insert((left, right), value);
        value
    }

    /// Adjustment in font units.
    fn units(&self, face: &Face, left: GlyphId, right: GlyphId) -> i16 {
        if self.legacy {
            return legacy_pair(face, left, right);
        }
        let Some(gpos) = face.tables().gpos else {
            return 0;
        };
        let mut total = 0i16;
        for &index in &self.lookups {
            let Some(lookup) = gpos.lookups.get(index) else {
                continue;
            };
            for subtable in lookup.subtables.into_iter::<PositioningSubtable>() {
                let PositioningSubtable::Pair(pair) = subtable else {
                    continue;
                };
                if let Some(value) = pair_value(&pair, left, right) {
                    total = total.saturating_add(value);
                    break;
                }
            }
        }
        total
    }
}

/// The first glyph's x-advance adjustment of one pair subtable, or `None` when it does not apply.
fn pair_value(pair: &PairAdjustment, left: GlyphId, right: GlyphId) -> Option<i16> {
    match pair {
        PairAdjustment::Format1 { coverage, sets } => {
            let set = sets.get(coverage.get(left)?)?;
            set.get(right).map(|(first, _)| first.x_advance)
        }
        PairAdjustment::Format2 {
            coverage,
            classes,
            matrix,
        } => {
            coverage.get(left)?;
            let pair = (classes.0.get(left), classes.1.get(right));
            matrix.get(pair).map(|(first, _)| first.x_advance)
        }
    }
}

/// Horizontal kerning from the legacy `kern` table (summed over its horizontal subtables).
fn legacy_pair(face: &Face, left: GlyphId, right: GlyphId) -> i16 {
    let Some(kern) = face.tables().kern else {
        return 0;
    };
    kern.subtables
        .into_iter()
        .filter(|subtable| subtable.horizontal && !subtable.variable)
        .filter_map(|subtable| subtable.glyphs_kerning(left, right))
        .fold(0i16, i16::saturating_add)
}

impl<'a> Renderer<'a> {
    /// Kerning in em after each character of text element `idx`, by byte offset, for its run
    /// `styles`; `None` unless `enabled` (the element's inherited `font-kerning: normal`), so other
    /// text lays out exactly as before. Pairs kern within one run; whitespace never kerns; a `data-hf-split` letter
    /// kerns against the next letter of its word (`LayoutHints::kern_after`).
    pub(in crate::render) fn text_kerning(
        &mut self,
        idx: usize,
        enabled: bool,
        styles: &[RunStyle<'a>],
    ) -> Result<Option<Vec<f32>>> {
        if !enabled {
            return Ok(None);
        }
        let composition: &'a Composition = self.composition;
        let element = &composition.elements[idx];
        let Content::Text(text) = &element.content else {
            return Ok(None);
        };
        let spans = &element.spans;
        let mut out = vec![0.0; text.len()];
        let mut chars = text.char_indices().peekable();
        while let Some((offset, ch)) = chars.next() {
            let run = style_index(spans, offset);
            let next = match chars.peek() {
                Some(&(at, next)) => (style_index(spans, at) == run).then_some(next),
                None => element.layout.kern_after,
            };
            if let Some(next) = next.filter(|next| !next.is_whitespace() && !ch.is_whitespace()) {
                out[offset] = self.fonts.kerning(styles[run].font(), ch, next)?;
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROBOTO: &[u8] = include_bytes!("../../../assets/fonts/Roboto-Regular.ttf");

    #[test]
    fn embedded_font_kerning_matches_a_shaping_engine() {
        let face = Face::parse(ROBOTO, 0).unwrap();
        let mut kerning = Kerning::new(&face);
        assert!(!kerning.is_empty());
        // Chromium (HarfBuzz) advance difference of each pair at 1000 px with `font-kerning:
        // normal` versus `none`, for this exact font file.
        let reference = [
            ("AV", -42.48),
            ("To", -48.34),
            ("Ty", 0.0),
            ("Yo", -31.738),
            ("Te", 0.0),
            ("LT", -134.277),
            ("VA", -36.621),
            ("oo", 0.0),
            ("P.", -158.203),
            ("Wa", -16.113),
            ("AT", -62.988),
            ("Av", -24.414),
            ("r.", -60.059),
            ("FA", -83.008),
        ];
        for (pair, expected) in reference {
            let mut chars = pair.chars();
            let (a, b) = (chars.next().unwrap(), chars.next().unwrap());
            let value = kerning.pair(&face, a, b) * 1000.0;
            assert!(
                (value - expected).abs() < 0.01,
                "{pair}: {value} vs {expected}"
            );
        }
        // Memoized values are the same values; unmapped characters do not kern.
        assert_eq!(kerning.pair(&face, 'A', 'V'), kerning.pair(&face, 'A', 'V'));
        assert_eq!(kerning.pair(&face, 'A', '\u{10FFFF}'), 0.0);
    }
}
