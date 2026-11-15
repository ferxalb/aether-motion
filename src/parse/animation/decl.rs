//! `animation` shorthand and longhand declaration parsing.
use crate::parse::animation::timing::{
    parse_direction, parse_fill, parse_iterations, parse_time, parse_timing,
};
use crate::parse::animation::AnimationDecl;
use crate::parse::values::tokens::split_top;
use anyhow::{anyhow, bail, Result};

pub(in crate::parse) fn single_animation(value: &str) -> Result<()> {
    if split_top(value, ',').len() > 1 {
        bail!("animation: several animations per element are unsupported: {value}");
    }
    Ok(())
}

pub(in crate::parse) fn parse_animation_shorthand(value: &str) -> Result<AnimationDecl> {
    single_animation(value)?;
    let mut decl = AnimationDecl::default();
    let mut times = 0;
    for token in split_top(value, ' ') {
        let starts_numeric =
            token.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'));
        if starts_numeric && (token.ends_with('s')) {
            let time = parse_time(token)?;
            match times {
                0 => decl.duration = Some(time),
                1 => decl.delay = time,
                _ => bail!("animation: too many times: {value}"),
            }
            times += 1;
        } else if starts_numeric || token == "infinite" {
            decl.iterations = parse_iterations(token)?;
        } else if let Some(direction) = parse_direction(token) {
            decl.direction = direction;
        } else if let Some((backwards, forwards)) = parse_fill(token).filter(|_| token != "none") {
            (decl.backwards, decl.forwards) = (backwards, forwards);
        } else if token == "running" {
        } else if token == "paused" {
            bail!("animation-play-state: paused is unsupported");
        } else if token == "none" {
            decl.name = None;
        } else if matches!(
            token,
            "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" | "step-start" | "step-end"
        ) || token.starts_with("cubic-bezier(")
            || token.starts_with("steps(")
        {
            decl.easing = parse_timing(token)?;
        } else if token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            if decl.name.is_some() {
                bail!("animation: more than one name: {value}");
            }
            decl.name = Some(token.to_string());
        } else {
            bail!("animation: unsupported value: {token}");
        }
    }
    Ok(decl)
}

pub(in crate::parse) fn apply_animation_longhand(
    decl: &mut AnimationDecl,
    key: &str,
    value: &str,
) -> Result<()> {
    single_animation(value)?;
    match key {
        "animation-name" => {
            decl.name = (value != "none").then(|| value.to_string());
        }
        "animation-duration" => decl.duration = Some(parse_time(value)?),
        "animation-delay" => decl.delay = parse_time(value)?,
        "animation-timing-function" => decl.easing = parse_timing(value)?,
        "animation-iteration-count" => decl.iterations = parse_iterations(value)?,
        "animation-direction" => {
            decl.direction = parse_direction(value)
                .ok_or_else(|| anyhow!("unsupported animation-direction: {value}"))?;
        }
        "animation-fill-mode" => {
            (decl.backwards, decl.forwards) = parse_fill(value)
                .ok_or_else(|| anyhow!("unsupported animation-fill-mode: {value}"))?;
        }
        "animation-play-state" if value == "running" => {}
        _ => bail!("unsupported {key}: {value}"),
    }
    Ok(())
}
