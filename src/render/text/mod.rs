//! Text layout, fonts, stable glyph coverage, subpixel mask sampling, text shadows, rich-text runs
//! (`props.rs`), glyph strokes (`stroke.rs`), pair kerning (`kerning.rs`), and the synthetic
//! oblique of italic text without an italic face (`oblique.rs`).

mod block;
mod fonts;
mod kerning;
mod mask;
mod oblique;
mod props;
mod raster;
mod shadow;
mod stroke;
mod wrap;

pub(super) use block::TextBlock;
pub(super) use fonts::FontStore;
pub(super) use raster::TextRun;
