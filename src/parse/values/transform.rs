//! `transform` parsing: translate, uniform scale, and rotate.
use crate::model::Length;
use crate::parse::values::length::parse_length;
use anyhow::{anyhow, bail, Context, Result};

/// `transform` as (translate x, translate y, uniform scale, rotation in degrees), composed left to right.
pub(in crate::parse) fn parse_transform(raw: &str) -> Result<(f32, f32, f32, f32)> {
    let raw = raw.trim();
    if raw == "none" {
        return Ok((0.0, 0.0, 1.0, 0.0));
    }
    let px = |arg: &str| -> Result<f32> {
        match parse_length(arg, "transform")? {
            Length::Px(n) => Ok(n),
            Length::Percent(_) => bail!("transform: translate only supports px, not %"),
        }
    };
    let factor = |arg: &str| -> Result<f32> {
        let n: f32 = arg
            .parse()
            .with_context(|| format!("transform: invalid scale factor: {arg}"))?;
        if !n.is_finite() || n < 0.0 {
            bail!("transform: scale must be a non-negative number");
        }
        Ok(n)
    };
    let angle = |arg: &str| -> Result<f32> {
        let (number, per_unit) = if let Some(n) = arg.strip_suffix("deg") {
            (n, 1.0)
        } else if let Some(n) = arg.strip_suffix("turn") {
            (n, 360.0)
        } else if let Some(n) = arg.strip_suffix("rad") {
            (n, 180.0 / std::f32::consts::PI)
        } else if arg == "0" {
            ("0", 1.0)
        } else {
            bail!("transform: angles need deg, rad or turn: {arg}");
        };
        let n: f32 = number
            .trim()
            .parse()
            .with_context(|| format!("transform: invalid angle: {arg}"))?;
        if !n.is_finite() {
            bail!("transform: angle is not finite");
        }
        Ok(n * per_unit)
    };
    let (mut tx, mut ty, mut scale, mut rotate) = (0.0_f32, 0.0_f32, 1.0_f32, 0.0_f32);
    let mut rest = raw;
    while !rest.trim().is_empty() {
        let text = rest.trim_start();
        let (open, close) = text
            .find('(')
            .zip(text.find(')'))
            .filter(|(open, close)| open < close)
            .ok_or_else(|| anyhow!("invalid transform: {raw}"))?;
        let name = text[..open].trim().to_ascii_lowercase();
        let args: Vec<_> = text[open + 1..close].split(',').map(str::trim).collect();
        // Composing `C ∘ F`: a translation written after a scale or rotation is scaled and rotated too.
        let (sin, cos) = rotate.to_radians().sin_cos();
        let mut shift = |x: f32, y: f32| {
            tx += scale * (x * cos - y * sin);
            ty += scale * (x * sin + y * cos);
        };
        match (name.as_str(), args.as_slice()) {
            ("translate", [x]) | ("translatex", [x]) => shift(px(x)?, 0.0),
            ("translatey", [y]) => shift(0.0, px(y)?),
            ("translate", [x, y]) => shift(px(x)?, px(y)?),
            ("scale", [k]) => scale *= factor(k)?,
            ("scale", [a, b]) if factor(a)? == factor(b)? => scale *= factor(a)?,
            ("scale", [_, _]) => bail!("transform: non-uniform scale is unsupported"),
            ("rotate", [a]) => rotate += angle(a)?,
            _ => bail!(
                "unsupported transform: {name}(...); use translate, translateX, translateY, scale or rotate"
            ),
        }
        rest = &text[close + 1..];
    }
    Ok((tx, ty, scale, rotate))
}
