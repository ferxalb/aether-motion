//! Animation timing values: durations, easing functions, iteration counts, direction, and fill mode.
use crate::model::Easing;
use crate::parse::animation::{Direction, Iterations, CSS_EASE};
use anyhow::{anyhow, bail, Context, Result};

pub(in crate::parse) fn parse_time(raw: &str) -> Result<f64> {
    let (number, factor) = match raw.strip_suffix("ms") {
        Some(n) => (n, 0.001),
        None => (
            raw.strip_suffix('s')
                .ok_or_else(|| anyhow!("invalid CSS time (use s or ms): {raw}"))?,
            1.0,
        ),
    };
    let n: f64 = number
        .trim()
        .parse()
        .with_context(|| format!("invalid CSS time: {raw}"))?;
    if !n.is_finite() {
        bail!("CSS time is not finite: {raw}");
    }
    Ok(n * factor)
}

pub(in crate::parse) fn parse_timing(raw: &str) -> Result<Easing> {
    let raw = raw.trim();
    Ok(match raw {
        "linear" => Easing::Linear,
        "ease" => CSS_EASE,
        "ease-in" => Easing::Bezier([0.42, 0.0, 1.0, 1.0]),
        "ease-out" => Easing::Bezier([0.0, 0.0, 0.58, 1.0]),
        "ease-in-out" => Easing::Bezier([0.42, 0.0, 0.58, 1.0]),
        "step-start" => Easing::Steps {
            count: 1,
            jump_start: true,
        },
        "step-end" => Easing::Steps {
            count: 1,
            jump_start: false,
        },
        _ => {
            let args = |prefix: &str| {
                raw.strip_prefix(prefix)
                    .and_then(|s| s.strip_suffix(')'))
                    .map(|s| s.split(',').map(str::trim).collect::<Vec<_>>())
            };
            if let Some(args) = args("cubic-bezier(") {
                let n: Vec<f32> = args
                    .iter()
                    .map(|a| a.parse::<f32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| anyhow!("cubic-bezier: expected 4 numbers: {raw}"))?;
                let [x1, y1, x2, y2] = n[..] else {
                    bail!("cubic-bezier: expected 4 numbers: {raw}");
                };
                if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
                    bail!("cubic-bezier: x1 and x2 must be between 0 and 1: {raw}");
                }
                if !y1.is_finite() || !y2.is_finite() {
                    bail!("cubic-bezier: values are not finite: {raw}");
                }
                Easing::Bezier([x1, y1, x2, y2])
            } else if let Some(args) = args("steps(") {
                let count: u32 = args
                    .first()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| (1..=1000).contains(n))
                    .ok_or_else(|| anyhow!("steps: expected an integer from 1 to 1000: {raw}"))?;
                let jump_start = match args.get(1).copied() {
                    None | Some("jump-end") | Some("end") => false,
                    Some("jump-start") | Some("start") => true,
                    Some(other) => bail!("steps: unsupported jump: {other}"),
                };
                if args.len() > 2 {
                    bail!("steps: too many arguments: {raw}");
                }
                Easing::Steps { count, jump_start }
            } else {
                bail!("unsupported animation-timing-function: {raw}")
            }
        }
    })
}

pub(in crate::parse) fn parse_iterations(raw: &str) -> Result<Iterations> {
    if raw == "infinite" {
        return Ok(Iterations::Infinite);
    }
    raw.parse::<u32>()
        .ok()
        .filter(|n| (1..=10_000).contains(n))
        .map(Iterations::Count)
        .ok_or_else(|| {
            anyhow!("animation-iteration-count: only positive integers or infinite: {raw}")
        })
}

pub(in crate::parse) fn parse_direction(raw: &str) -> Option<Direction> {
    match raw {
        "normal" => Some(Direction::Normal),
        "reverse" => Some(Direction::Reverse),
        "alternate" => Some(Direction::Alternate),
        "alternate-reverse" => Some(Direction::AlternateReverse),
        _ => None,
    }
}

pub(in crate::parse) fn parse_fill(raw: &str) -> Option<(bool, bool)> {
    match raw {
        "none" => Some((false, false)),
        "forwards" => Some((false, true)),
        "backwards" => Some((true, false)),
        "both" => Some((true, true)),
        _ => None,
    }
}
