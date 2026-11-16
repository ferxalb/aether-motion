//! CSS data model (rules, `@keyframes` blocks, stylesheet) and its parsing submodules.

pub(in crate::parse) mod cascade;
pub(in crate::parse) mod selectors;
pub(in crate::parse) mod stylesheet;
pub(in crate::parse) mod vars;

use std::collections::HashMap;

#[derive(Debug)]
pub(in crate::parse) struct Rule {
    pub(in crate::parse) selector: String,
    pub(in crate::parse) declarations: Vec<(String, String)>,
    pub(in crate::parse) specificity: usize,
    pub(in crate::parse) order: usize,
}

/// Declared in `@keyframes`, before values are resolved against an element.
#[derive(Debug)]
pub(in crate::parse) struct RawFrame {
    pub(in crate::parse) offset: f64,
    pub(in crate::parse) declarations: Vec<(String, String)>,
}

#[derive(Default, Debug)]
pub(in crate::parse) struct Stylesheet {
    pub(in crate::parse) rules: Vec<Rule>,
    pub(in crate::parse) keyframes: HashMap<String, Vec<RawFrame>>,
    /// `--id` custom properties from composition variables, set on the composition root over any
    /// declaration of the same name there (descendants may still redefine them).
    pub(in crate::parse) pinned: Vars,
}

/// Custom properties (`--name`) visible to an element.
pub(in crate::parse) type Vars = HashMap<String, String>;
