//! Pixel painting for the renderer: the `Surface` buffer and one submodule per visual feature.

mod border;
mod fill;
mod images;
mod rotate;
mod shadow;
mod shape;
mod surface;

pub(super) use images::{CachedImage, ImageKey};
pub(super) use surface::Surface;
