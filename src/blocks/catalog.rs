//! The built-in catalog: block and theme templates embedded from the repository's `blocks/` folder,
//! and the generated character rigs from `rig`.

use super::rig::Rig;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    /// A scene fragment or element group to place inside a composition.
    Block,
    /// A `<style>` with custom properties that recolors every block.
    Theme,
}

/// Where an entry's HTML comes from.
#[derive(Clone, Copy, Debug)]
pub(super) enum Source {
    /// A `{{name|default}}` template.
    Template(&'static str),
    /// A generated rig.
    Rig(Rig),
}

pub(super) struct Entry {
    pub(super) name: &'static str,
    pub(super) kind: Kind,
    pub(super) source: Source,
}

pub(super) const ENTRIES: &[Entry] = &[
    Entry {
        name: "hero-title",
        kind: Kind::Block,
        source: Source::Template(include_str!("../../blocks/hero-title.html")),
    },
    Entry {
        name: "feature-card",
        kind: Kind::Block,
        source: Source::Template(include_str!("../../blocks/feature-card.html")),
    },
    Entry {
        name: "stat-card",
        kind: Kind::Block,
        source: Source::Template(include_str!("../../blocks/stat-card.html")),
    },
    Entry {
        name: "counter-card",
        kind: Kind::Block,
        source: Source::Template(include_str!("../../blocks/counter-card.html")),
    },
    Entry {
        name: "cta",
        kind: Kind::Block,
        source: Source::Template(include_str!("../../blocks/cta.html")),
    },
    Entry {
        name: "glow",
        kind: Kind::Block,
        source: Source::Template(include_str!("../../blocks/glow.html")),
    },
    Entry {
        name: "human-2d",
        kind: Kind::Block,
        source: Source::Rig(Rig::Human2d),
    },
    Entry {
        name: "human-3d",
        kind: Kind::Block,
        source: Source::Rig(Rig::Human3d),
    },
    Entry {
        name: "theme-midnight",
        kind: Kind::Theme,
        source: Source::Template(include_str!("../../blocks/theme-midnight.html")),
    },
    Entry {
        name: "theme-sunrise",
        kind: Kind::Theme,
        source: Source::Template(include_str!("../../blocks/theme-sunrise.html")),
    },
    Entry {
        name: "theme-paper",
        kind: Kind::Theme,
        source: Source::Template(include_str!("../../blocks/theme-paper.html")),
    },
];
