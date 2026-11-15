//! Computes an element's `Style` from the cascade by dispatching each declaration to a property group.
//!
//! Groups: `layout` (box position, size, flex, spacing), `text` (font, color, alignment),
//! `visual` (background, border, shadow, opacity, transform). Each group returns `Ok(false)` for
//! properties it does not own, and anything nobody owns is an explicit error.

mod layout;
mod text;
mod visual;

use crate::model::Style;
use crate::parse::animation::decl::{apply_animation_longhand, parse_animation_shorthand};
use crate::parse::animation::resolve::resolve_animation;
use crate::parse::animation::{AnimationDecl, AnimationSpec};
use crate::parse::css::cascade::cascade;
use crate::parse::css::vars::{inherit_vars, substitute};
use crate::parse::css::{Stylesheet, Vars};
use crate::parse::html::attr;
use anyhow::{bail, Context, Result};
use kuchiki::NodeRef;

pub(in crate::parse) struct Resolved {
    pub(in crate::parse) style: Style,
    pub(in crate::parse) vars: Vars,
    pub(in crate::parse) animation: Option<AnimationSpec>,
}

pub(in crate::parse) fn style_for(
    node: &NodeRef,
    sheet: &Stylesheet,
    parent_vars: &Vars,
) -> Result<Resolved> {
    let properties = cascade(node, &sheet.rules)?;
    let vars = inherit_vars(&properties, parent_vars);
    let mut animation = AnimationDecl::default();
    let mut style = Style::default();
    for (key, value) in properties {
        if key.starts_with("--") {
            continue;
        }
        let value = substitute(&value, &vars, 0).with_context(|| format!("property {key}"))?;
        if layout::apply(&key, &value, &mut style)?
            || text::apply(&key, &value, &mut style)?
            || visual::apply(&key, &value, &mut style)?
        {
            continue;
        }
        match key.as_str() {
            "animation" => animation = parse_animation_shorthand(&value)?,
            k if k.starts_with("animation-") => {
                apply_animation_longhand(&mut animation, k, &value)?
            }
            "box-sizing" if value == "border-box" => {}
            "visibility"
                if value == "hidden"
                    && attr(node, "class")
                        .is_some_and(|c| c.split_whitespace().any(|x| x == "clip")) => {}
            _ => bail!(
                "unsupported CSS property on <{}>: {key}: {value}",
                node.as_element().unwrap().name.local
            ),
        }
    }
    let animation =
        resolve_animation(animation, &sheet.keyframes, &vars).with_context(|| "CSS animation")?;
    Ok(Resolved {
        style,
        vars,
        animation,
    })
}
