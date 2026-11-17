//! Font index and per-family cache, independent of any renderer state.
//!
//! Roboto Regular and Bold (Apache 2.0, `assets/fonts/`) are embedded in the binary and back the
//! generic families (`sans-serif`, `serif`, `monospace`) and the final fallback, so text that does
//! not name an installed font renders identically on every machine. Named families still resolve
//! against the system fonts.
//!
//! A request names a family list, a weight, and whether the text is italic. When no italic face of
//! the family is installed (the embedded Roboto has none) the upright face is used and the request
//! is marked for a synthetic oblique (see `oblique.rs`), as browsers do. Each loaded face also carries
//! its pair [`Kerning`].

use crate::render::text::kerning::Kerning;
use anyhow::{anyhow, Result};
use fontdb::{Database, Family, Query, Style, Weight};
use fontdue::{Font, FontSettings};
use std::collections::HashMap;
use std::sync::Arc;

/// The face a run of text asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::render) struct FontRequest<'a> {
    /// A CSS family list.
    pub(in crate::render) family: &'a str,
    pub(in crate::render) weight: u16,
    pub(in crate::render) italic: bool,
}

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
    /// Italic was requested but the face is upright: slant it synthetically.
    synthetic_oblique: bool,
    kerning: Kerning,
}

/// Font index plus the parsed fonts requested so far, keyed by the CSS family list, weight, and
/// italic flag.
pub(in crate::render) struct FontStore {
    fonts: HashMap<(String, u16, bool), Loaded>,
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
    pub(in crate::render) fn get(&mut self, request: FontRequest) -> Result<&Font> {
        Ok(&self.load(request)?.font)
    }

    /// A font already resolved by [`Self::get`].
    pub(in crate::render) fn loaded(&self, request: FontRequest) -> Option<&Font> {
        self.fonts.get(&key(request)).map(|loaded| &loaded.font)
    }

    /// Whether a resolved request is drawn with a synthetic oblique (italic asked, upright face).
    pub(in crate::render) fn synthetic_oblique(&mut self, request: FontRequest) -> Result<bool> {
        Ok(self.load(request)?.synthetic_oblique)
    }

    /// Face bytes and collection index of a font, for outline access.
    pub(in crate::render) fn face_data(
        &mut self,
        request: FontRequest,
    ) -> Result<(Arc<Vec<u8>>, u32)> {
        let loaded = self.load(request)?;
        Ok((Arc::clone(&loaded.data), loaded.index))
    }

    /// Pair kerning between `left` and `right` in em (see `kerning.rs`).
    pub(in crate::render) fn kerning(
        &mut self,
        request: FontRequest,
        left: char,
        right: char,
    ) -> Result<f32> {
        let loaded = self.load_mut(request)?;
        if loaded.kerning.is_empty() {
            return Ok(0.0);
        }
        let face = ttf_parser::Face::parse(&loaded.data, loaded.index)
            .map_err(|error| anyhow!("invalid font: {error}"))?;
        Ok(loaded.kerning.pair(&face, left, right))
    }

    fn load(&mut self, request: FontRequest) -> Result<&Loaded> {
        Ok(self.load_mut(request)?)
    }

    fn load_mut(&mut self, request: FontRequest) -> Result<&mut Loaded> {
        let FontRequest {
            family,
            weight,
            italic,
        } = request;
        let key = key(request);
        if !self.fonts.contains_key(&key) {
            let style = if italic { Style::Italic } else { Style::Normal };
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
                    style,
                    ..Default::default()
                })
            });
            let fallback = [Family::SansSerif];
            let id = id
                .or_else(|| {
                    self.db.query(&Query {
                        families: &fallback,
                        weight: Weight(weight),
                        style,
                        ..Default::default()
                    })
                })
                .ok_or_else(|| anyhow!("no system font is available"))?;
            // fontdb falls back to the nearest style; an upright face for italic text is slanted.
            let upright = self
                .db
                .face(id)
                .is_some_and(|face| face.style == Style::Normal);
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
            let kerning = Kerning::new(
                &ttf_parser::Face::parse(&data, index)
                    .map_err(|error| anyhow!("invalid font: {error}"))?,
            );
            self.fonts.insert(
                key.clone(),
                Loaded {
                    font,
                    data,
                    index,
                    synthetic_oblique: italic && upright,
                    kerning,
                },
            );
        }
        Ok(self.fonts.get_mut(&key).expect("inserted above"))
    }
}

fn key(request: FontRequest) -> (String, u16, bool) {
    (request.family.to_string(), request.weight, request.italic)
}
