//! A small memory-bounded cache of rasterized layers (gradients, shadows) that are drawn again at
//! the same size on later frames. Only the second sighting of a key builds a tile, so a one-off
//! draw (a snapshot) never pays for one.

use std::collections::HashMap;
use std::hash::Hash;

/// Memory one surface may spend on cached tiles.
const BUDGET_BYTES: usize = 64 * 1024 * 1024;

enum Slot<T> {
    /// Drawn once directly; a repeat builds the tile.
    Seen,
    Ready(T),
    /// Over budget: always drawn directly.
    Direct,
}

pub(in crate::render) struct TileCache<K, T> {
    slots: HashMap<K, Slot<T>>,
    bytes: usize,
}

impl<K, T> Default for TileCache<K, T> {
    fn default() -> Self {
        Self {
            slots: HashMap::new(),
            bytes: 0,
        }
    }
}

impl<K: Hash + Eq + Copy, T> TileCache<K, T> {
    /// The tile for `key` (`cost` bytes, made by `build`), or `None` when this draw should go
    /// through the direct path.
    pub fn get_or_build(
        &mut self,
        key: K,
        cost: usize,
        build: impl FnOnce() -> T,
    ) -> Option<&T> {
        match self.slots.get(&key) {
            None => {
                self.slots.insert(key, Slot::Seen);
                return None;
            }
            Some(Slot::Direct) => return None,
            Some(Slot::Seen) => {
                if self.bytes + cost > BUDGET_BYTES {
                    self.slots.insert(key, Slot::Direct);
                    return None;
                }
                self.bytes += cost;
                self.slots.insert(key, Slot::Ready(build()));
            }
            Some(Slot::Ready(_)) => {}
        }
        match self.slots.get(&key) {
            Some(Slot::Ready(tile)) => Some(tile),
            _ => None,
        }
    }
}
