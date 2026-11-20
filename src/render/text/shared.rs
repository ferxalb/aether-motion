//! Text cache keys shared by the instances of a reused scene.
//!
//! An element's laid-out and rasterized text depends only on its own body (content, spans, tag,
//! style, kerning hint) and on the styles it inherits: those of its ancestors up to and including
//! the nearest scene root, where inheritance stops. Text placement also reads the direct parent's
//! flex alignment, which is on that chain except for a scene root, whose parent is its instance
//! host. So two elements whose bodies, and whose ancestors' bodies up to that boundary, are the
//! same shared bodies (see `model::Element`) produce the same text and can share cached layouts
//! and glyph rasters; every other element keeps a key of its own.

use crate::model::{Composition, ElementBody};
use std::collections::HashMap;

/// The text cache key of every element: the index of the first element with the same chain of
/// bodies (see the module documentation), or its own index.
pub(in crate::render) fn shared_text_keys(composition: &Composition) -> Vec<u32> {
    let elements = &composition.elements;
    // Interned (body, parent chain) pairs; `u32::MAX` ends a chain at a scene or composition root.
    let mut chains: HashMap<(*const ElementBody, u32), u32> = HashMap::new();
    let mut chain = Vec::with_capacity(elements.len());
    let mut keys = Vec::with_capacity(elements.len());
    for (idx, element) in elements.iter().enumerate() {
        let idx = idx as u32;
        let parent = match element.parent {
            _ if element.layout.scene_root => Some(u32::MAX),
            None => Some(u32::MAX),
            Some(parent) if parent < idx as usize => Some(chain[parent]),
            Some(_) => None,
        };
        let own = parent.map_or(idx, |parent| {
            *chains
                .entry((std::sync::Arc::as_ptr(&element.body), parent))
                .or_insert(idx)
        });
        chain.push(own);
        keys.push(if element.layout.scene_root { idx } else { own });
    }
    keys
}
