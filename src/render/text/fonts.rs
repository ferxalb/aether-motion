//! System font lookup and caching by CSS `font-family` list.

impl<'a> Renderer<'a> {
    pub(in crate::render) fn font(&mut self, family: &str) -> Result<&Font> {
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
                self.font_db.query(&Query {
                    families: &list,
                    ..Default::default()
                })
            });
            let fallback = [Family::SansSerif];
            let id = id
                .or_else(|| {
                    self.font_db.query(&Query {
                        families: &fallback,
                        ..Default::default()
                    })
                })
                .ok_or_else(|| anyhow!("no system font is available"))?;
            let font = self
                .font_db
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
