//! The built-in catalog: block and theme templates embedded from the repository's `blocks/` folder.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    /// A scene fragment or element group to place inside a composition.
    Block,
    /// A `<style>` with custom properties that recolors every block.
    Theme,
}

pub(super) struct Entry {
    pub(super) name: &'static str,
    pub(super) kind: Kind,
    pub(super) template: &'static str,
}

pub(super) const ENTRIES: &[Entry] = &[
    Entry {
        name: "hero-title",
        kind: Kind::Block,
        template: include_str!("../../blocks/hero-title.html"),
    },
    Entry {
        name: "feature-card",
        kind: Kind::Block,
        template: include_str!("../../blocks/feature-card.html"),
    },
    Entry {
        name: "stat-card",
        kind: Kind::Block,
        template: include_str!("../../blocks/stat-card.html"),
    },
    Entry {
        name: "cta",
        kind: Kind::Block,
        template: include_str!("../../blocks/cta.html"),
    },
    Entry {
        name: "glow",
        kind: Kind::Block,
        template: include_str!("../../blocks/glow.html"),
    },
    Entry {
        name: "theme-midnight",
        kind: Kind::Theme,
        template: include_str!("../../blocks/theme-midnight.html"),
    },
    Entry {
        name: "theme-sunrise",
        kind: Kind::Theme,
        template: include_str!("../../blocks/theme-sunrise.html"),
    },
    Entry {
        name: "theme-paper",
        kind: Kind::Theme,
        template: include_str!("../../blocks/theme-paper.html"),
    },
];
