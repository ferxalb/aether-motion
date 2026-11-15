//! Text support: font lookup, inherited properties, wrapping and block layout, and glyph painting.

mod block;
mod fonts;
mod props;
mod raster;
mod wrap;

pub(super) use block::TextBlock;
pub(super) use fonts::FontStore;
pub(super) use raster::TextRun;
