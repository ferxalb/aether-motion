//! Pixel painting primitives for the renderer, one submodule per visual feature.

mod border;
mod fill;
mod images;
mod rotate;
mod shadow;
mod shape;
mod surface;

pub(super) use images::{CachedImage, ImageKey};


