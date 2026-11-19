//! Layout entry points: which children follow a flex parent, and how items are sized and placed.
//! `split_wrap` measures the line breaks of `data-hf-split-wrap` text between the two parses.

pub(super) mod flex;
pub(super) mod sizing;
pub(super) mod split_wrap;

use crate::model::Composition;

/// Direct children that follow the parent's flex layout instead of positioning themselves.
pub(in crate::render) fn in_flow(composition: &Composition, idx: usize) -> bool {
    let element = &composition.elements[idx];
    element
        .parent
        .is_some_and(|parent| composition.elements[parent].style.display.as_deref() == Some("flex"))
        && element.style.position.as_deref() != Some("absolute")
}
