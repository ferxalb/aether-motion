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

/// Named cubic-bezier presets (expo, cubic, quart, back) shared by the `data-hf-*` attributes.
/// The `back` curves overshoot, which gives a spring-like settle while staying a pure function of time.
fn preset_easing(name: &str) -> Option<Easing> {
    let curve = match name {
        "ease-in-cubic" => [0.32, 0.0, 0.67, 0.0],
        "ease-out-cubic" => [0.33, 1.0, 0.68, 1.0],
        "ease-in-out-cubic" => [0.65, 0.0, 0.35, 1.0],
        "ease-out-quart" => [0.25, 1.0, 0.5, 1.0],
        "ease-in-out-quart" => [0.76, 0.0, 0.24, 1.0],
        "ease-in-expo" => [0.7, 0.0, 0.84, 0.0],
        "ease-out-expo" => [0.16, 1.0, 0.3, 1.0],
        "ease-in-out-expo" => [0.87, 0.0, 0.13, 1.0],
        "ease-in-back" => [0.36, 0.0, 0.66, -0.56],
        "ease-out-back" => [0.34, 1.56, 0.64, 1.0],
        "ease-in-out-back" => [0.68, -0.6, 0.32, 1.6],
        _ => return None,
    };
    Some(Easing::Bezier(curve))
}

/// Easing for `data-hf-keyframes` and `data-hf-camera`.
///
/// `linear`, `ease-in`, `ease-out`, and `ease-in-out` keep their quadratic meaning; named presets,
/// `spring(overshoot, oscillations)`, and any CSS timing function (`cubic-bezier(...)`,
/// `steps(...)`, `ease`) are also accepted.
pub(in crate::parse) fn parse_keyframe_easing(raw: &str) -> Result<Easing> {
    let raw = raw.trim();
    if let Some(spring) = parse_spring(raw)? {
        return Ok(spring);
    }
    Ok(match raw {
        "linear" => Easing::Linear,
        "ease-in" => Easing::EaseIn,
        "ease-out" => Easing::EaseOut,
        "ease-in-out" => Easing::EaseInOut,
        _ => match preset_easing(raw) {
            Some(easing) => easing,
            None => parse_timing(raw).map_err(|e| {
                anyhow!("unsupported easing '{raw}' ({e}); use linear, ease-in/out, a named preset such as ease-out-expo or ease-out-back, spring(overshoot, oscillations), or cubic-bezier(x1, y1, x2, y2)")
            })?,
        },
    })
}

/// `spring`, `spring(overshoot)`, or `spring(overshoot, oscillations)` (defaults 0.12 and 2): a
/// damped spring whose highest value is exactly `1 + overshoot` (0 to 0.6) and that comes to rest at
/// the keyframe; `None` for other names.
fn parse_spring(raw: &str) -> Result<Option<Easing>> {
    use crate::model::Spring;
    let args = if raw == "spring" {
        Vec::new()
    } else if let Some(inner) = raw
        .strip_prefix("spring(")
        .and_then(|s| s.strip_suffix(')'))
    {
        inner.split(',').map(str::trim).collect()
    } else {
        return Ok(None);
    };
    let usage = || {
        anyhow!(
            "spring takes an overshoot from 0 to {} and a whole number of oscillations from 1 to 6, for example spring(0.15, 2): {raw}",
            Spring::MAX_OVERSHOOT
        )
    };
    if args.len() > 2 {
        return Err(usage());
    }
    let overshoot = match args.first() {
        Some(a) => a.parse::<f32>().map_err(|_| usage())?,
        None => 0.12,
    };
    let oscillations = match args.get(1) {
        Some(a) => a.parse::<u32>().map_err(|_| usage())?,
        None => 2,
    };
    Spring::new(overshoot, oscillations)
        .map(|s| Some(Easing::Spring(s)))
        .ok_or_else(usage)
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
