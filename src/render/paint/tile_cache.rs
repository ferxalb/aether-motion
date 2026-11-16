//! A small memory-bounded cache of rasterized layers (gradients, shadows) that are drawn again at
//! the same size on later frames. Only the second sighting of a key builds a tile, so a one-off
//! draw (a snapshot) never pays for one. Metadata is capped; exhaustion returns an error
//! rather than changing which tiles are admitted and potentially changing rendered pixels.

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::hash::Hash;

/// Memory one surface may spend on cached tiles.
const BUDGET_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

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
    ) -> Result<Option<&T>> {
        match self.slots.get(&key) {
            None => {
                if self.slots.len() >= MAX_ENTRIES {
                    bail!("tile cache metadata exceeds 4096 entries; reduce animated size variation or split the composition");
                }
                self.slots.insert(key, Slot::Seen);
                return Ok(None);
            }
            Some(Slot::Direct) => return Ok(None),
            Some(Slot::Seen) => {
                if cost > BUDGET_BYTES.saturating_sub(self.bytes) {
                    self.slots.insert(key, Slot::Direct);
                    return Ok(None);
                }
                self.bytes += cost;
                self.slots.insert(key, Slot::Ready(build()));
            }
            Some(Slot::Ready(_)) => {}
        }
        Ok(match self.slots.get(&key) {
            Some(Slot::Ready(tile)) => Some(tile),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_keys_are_bounded_without_evicting_or_changing_existing_slots() {
        let mut cache = TileCache::<usize, u8>::default();
        for key in 0..MAX_ENTRIES {
            assert!(cache
                .get_or_build(key, 1, || panic!("first sighting must not build"))
                .unwrap()
                .is_none());
        }
        for key in MAX_ENTRIES..MAX_ENTRIES + 100 {
            let error = cache
                .get_or_build(key, 1, || panic!("overflow must not build"))
                .unwrap_err();
            assert!(error.to_string().contains("metadata exceeds"));
        }
        assert_eq!(cache.slots.len(), MAX_ENTRIES);
        assert_eq!(*cache.get_or_build(0, 1, || 73).unwrap().unwrap(), 73);
        assert_eq!(
            *cache
                .get_or_build(0, 1, || panic!("ready tile must be reused"))
                .unwrap()
                .unwrap(),
            73
        );
        assert_eq!(cache.bytes, 1);
    }

    #[test]
    fn oversized_costs_cannot_wrap_or_call_the_builder() {
        let mut cache = TileCache::<usize, u8>::default();
        cache.get_or_build(0, 1, || 0).unwrap();
        cache.get_or_build(0, 1, || 0).unwrap();
        cache.get_or_build(1, usize::MAX, || 0).unwrap();
        assert!(cache
            .get_or_build(1, usize::MAX, || panic!("oversized tile must stay direct"))
            .unwrap()
            .is_none());
        assert_eq!(cache.bytes, 1);
        assert!(cache
            .get_or_build(1, 1, || panic!("direct slot must remain direct"))
            .unwrap()
            .is_none());
    }
}
