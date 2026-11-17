//! Parsers for individual CSS value types (lengths, colors, backgrounds, clip paths and masks, filters, transforms, shadows).

pub(in crate::parse) mod background;
pub(in crate::parse) mod clip;
pub mod color;
pub(in crate::parse) mod filter;
pub(in crate::parse) mod length;
pub(in crate::parse) mod shadow;
pub(in crate::parse) mod tokens;
pub(in crate::parse) mod transform;
