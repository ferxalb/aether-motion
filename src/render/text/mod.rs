//! Text layout, fonts, stable glyph coverage, subpixel mask sampling, text shadows, rich-text runs
//! (`props.rs`), and glyph strokes (`stroke.rs`).

mod block;
mod fonts;
mod mask;
mod props;
mod raster;
mod shadow;
mod stroke;
mod wrap;

pub(super) use block::TextBlock;
pub(super) use fonts::FontStore;
pub(super) use raster::TextRun;
