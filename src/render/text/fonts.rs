//! System font index and per-family cache, independent of any renderer state.

use anyhow::{anyhow, Result};
use fontdb::{Database, Family, Query};
use fontdue::{Font, FontSettings};
use std::collections::HashMap;

/// System font index plus the parsed fonts requested so far, keyed by the CSS family list.
pub(in crate::render) struct FontStore {
    fonts: HashMap<String, Font>,
    db: Database,
}

impl FontStore {
    /// Scans system fonts only when `needed`; otherwise the index stays empty.
    pub(in crate::render) fn new(needed: bool) -> Self {
        let mut db = Database::new();
        if needed {
            db.load_system_fonts();
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
    pub(in crate::render) fn get(&mut self, family: &str) -> Result<&Font> {
        if !self.fonts.contains_key(family) {
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
                    ..Default::default()
                })
            });
            let fallback = [Family::SansSerif];
            let id = id
                .or_else(|| {
                    self.db.query(&Query {
                        families: &fallback,
                        ..Default::default()
                    })
                })
                .ok_or_else(|| anyhow!("no system font is available"))?;
            let font = self
                .db
                .with_face_data(id, |data, index| {
                    Font::from_bytes(
                        // Font must own these bytes after fontdb's callback returns.
                        #[allow(clippy::unnecessary_to_owned)]
                        data.to_vec(),
                        FontSettings {
                            collection_index: index,
                            ..Default::default()
                        },
                    )
                })
                .ok_or_else(|| anyhow!("could not read font"))?
                .map_err(|error| anyhow!("invalid font: {error}"))?;
            self.fonts.insert(family.to_string(), font);
        }
        Ok(&self.fonts[family])
    }
}
