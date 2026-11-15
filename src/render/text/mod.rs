//! Text support: font lookup, inherited properties, wrapping and block layout, and glyph painting, and `text-shadow`.

mod block;
mod fonts;
mod props;
mod raster;
mod shadow;
mod wrap;

pub(super) use block::TextBlock;
pub(super) use fonts::FontStore;
pub(super) use raster::TextRun;
