//! Font index and per-family cache, independent of any renderer state.
//!
//! Roboto Regular and Bold (Apache 2.0, `assets/fonts/`) are embedded in the binary and back the
//! generic families (`sans-serif`, `serif`, `monospace`) and the final fallback, so text that does
//! not name an installed font renders identically on every machine. Named families still resolve
//! against the system fonts.

use anyhow::{anyhow, Result};
use fontdb::{Database, Family, Query, Weight};
use fontdue::{Font, FontSettings};
use std::collections::HashMap;
use std::sync::Arc;

/// Family name of the embedded fonts, used for every generic family.
const EMBEDDED_FAMILY: &str = "Roboto";

/// Embedded font faces (Roboto Regular and Bold).
const EMBEDDED_FONTS: [&[u8]; 2] = [
    include_bytes!("../../../assets/fonts/Roboto-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Roboto-Bold.ttf"),
];

/// A parsed font plus the face bytes it came from (for glyph outlines, see `stroke.rs`).
struct Loaded {
    font: Font,
    data: Arc<Vec<u8>>,
    index: u32,
}

/// Font index plus the parsed fonts requested so far, keyed by the CSS family list and weight.
pub(in crate::render) struct FontStore {
    fonts: HashMap<(String, u16), Loaded>,
    db: Database,
}

impl FontStore {
    /// Loads the embedded fonts and scans system fonts only when `needed`; otherwise the index stays empty.
    pub(in crate::render) fn new(needed: bool) -> Self {
        let mut db = Database::new();
        if needed {
            db.load_system_fonts();
            for data in EMBEDDED_FONTS {
                db.load_font_data(data.to_vec());
            }
            db.set_sans_serif_family(EMBEDDED_FAMILY);
            db.set_serif_family(EMBEDDED_FAMILY);
            db.set_monospace_family(EMBEDDED_FAMILY);
        }
        Self::with_index(db)
    }

    /// Reuses an already scanned font index (see [`FontStore::index`]).
    pub(in crate::render) fn with_index(db: Database) -> Self {
        Self {
            fonts: HashMap::new(),
            db,
        }
    }

    /// Clone of the font index, so another renderer does not scan the disk again.
    pub(in crate::render) fn index(&self) -> Database {
        self.db.clone()
    }

    /// Resolves the first available family of a CSS list, falling back to `sans-serif`.
    pub(in crate::render) fn get(&mut self, family: &str, weight: u16) -> Result<&Font> {
        Ok(&self.load(family, weight)?.font)
    }

    /// A font already resolved by [`Self::get`].
    pub(in crate::render) fn loaded(&self, family: &str, weight: u16) -> Option<&Font> {
        self.fonts
            .get(&(family.to_string(), weight))
            .map(|loaded| &loaded.font)
    }

    /// Face bytes and collection index of a font, for outline access.
    pub(in crate::render) fn face_data(
        &mut self,
        family: &str,
        weight: u16,
    ) -> Result<(Arc<Vec<u8>>, u32)> {
        let loaded = self.load(family, weight)?;
        Ok((Arc::clone(&loaded.data), loaded.index))
    }

    fn load(&mut self, family: &str, weight: u16) -> Result<&Loaded> {
        let key = (family.to_string(), weight);
        if !self.fonts.contains_key(&key) {
            let id = family.split(',').find_map(|item| {
                let item = item.trim().trim_matches(['\'', '"']);
                let requested = match item.to_ascii_lowercase().as_str() {
                    "sans-serif" => Family::SansSerif,
                    "serif" => Family::Serif,
                    "monospace" => Family::Monospace,
                    _ => Family::Name(item),
                };
                let list = [requested];
                self.db.query(&Query {
                    families: &list,
                    weight: Weight(weight),
                    ..Default::default()
                })
            });
            let fallback = [Family::SansSerif];
            let id = id
                .or_else(|| {
                    self.db.query(&Query {
                        families: &fallback,
                        weight: Weight(weight),
                        ..Default::default()
                    })
                })
                .ok_or_else(|| anyhow!("no system font is available"))?;
            let (data, index) = self
                .db
                .with_face_data(id, |data, index| (Arc::new(data.to_vec()), index))
                .ok_or_else(|| anyhow!("could not read font"))?;
            let font = Font::from_bytes(
                data.as_slice(),
                FontSettings {
                    collection_index: index,
                    ..Default::default()
                },
            )
            .map_err(|error| anyhow!("invalid font: {error}"))?;
            self.fonts.insert(key.clone(), Loaded { font, data, index });
        }
        Ok(&self.fonts[&key])
    }
}
