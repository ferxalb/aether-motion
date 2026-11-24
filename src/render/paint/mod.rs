//! Pixel painting for the renderer: the `Surface` buffer, one submodule per visual feature, and
//! `kernels` (branch-free per-row kernels with runtime CPU dispatch and bit-identical results).

mod backdrop;
mod border;
mod clip;
mod coverage;
mod effects;
mod fill;
pub(super) mod filmic;
mod finish;
mod float_ops;
mod gradient;
mod images;
mod inset;
mod kernels;
mod masked;
mod packed_image;
mod project;
pub(in crate::render) use project::{PlaneLayer, PlaneSetup};
mod resample;
mod reveal;
mod rotate;
mod scene3d;
mod shader;
mod shadow;
pub(in crate::render) use shader::ShaderState;
mod shape;
mod surface;
pub(crate) mod svg;
mod svg_levels;
mod tile_cache;
mod video;

pub(super) use effects::bloom::BloomScratch;
pub(super) use effects::color::ColorChain;
pub(super) use gradient::GradientPaint;
pub(super) use images::{CachedImage, ImageKey, ImageSrc, SequenceSources};
pub(super) use surface::Surface;
