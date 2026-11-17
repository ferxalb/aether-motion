//! Vector path geometry shared by parsing and rendering: path measurement (`measure.rs`) for
//! `data-hf-motion-path` and the stroke lengths of `data-hf-draw`, and the per-frame stroke reveal
//! of an inline `<svg>` (`draw_on.rs`).

pub mod draw_on;
pub mod measure;

pub use draw_on::DrawOnSvg;
pub use measure::PathMeasure;
