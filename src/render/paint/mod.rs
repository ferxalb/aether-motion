//! Pixel painting for the renderer: the `Surface` buffer and one submodule per visual feature.

mod backdrop;
mod border;
mod clip;
mod coverage;
mod effects;
mod fill;
mod finish;
mod float_ops;
mod gradient;
mod images;
mod inset;
mod masked;
mod resample;
mod reveal;
mod rotate;
mod shadow;
mod shape;
mod surface;
pub(crate) mod svg;
mod tile_cache;
mod video;

pub(super) use effects::color::ColorChain;
pub(super) use gradient::GradientPaint;
pub(super) use images::{CachedImage, ImageKey, ImageSrc};
pub(super) use surface::Surface;
