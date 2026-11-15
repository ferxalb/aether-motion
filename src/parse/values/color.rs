//! Color parsing: hex, `rgb()`, `hsl()` functions, and named colors.
use crate::model::Color;
use anyhow::{anyhow, bail, Context, Result};

pub(in crate::parse) const NAMED_COLORS: &[(&str, u32)] = &[
    ("black", 0x000000),
    ("white", 0xffffff),
    ("red", 0xff0000),
    ("green", 0x008000),
    ("blue", 0x0000ff),
    ("lime", 0x00ff00),
    ("yellow", 0xffff00),
    ("cyan", 0x00ffff),
    ("aqua", 0x00ffff),
    ("magenta", 0xff00ff),
    ("fuchsia", 0xff00ff),
    ("gray", 0x808080),
    ("grey", 0x808080),
    ("silver", 0xc0c0c0),
    ("maroon", 0x800000),
    ("olive", 0x808000),
    ("purple", 0x800080),
    ("teal", 0x008080),
    ("navy", 0x000080),
    ("orange", 0xffa500),
    ("pink", 0xffc0cb),
    ("gold", 0xffd700),
    ("coral", 0xff7f50),
    ("tomato", 0xff6347),
    ("indigo", 0x4b0082),
    ("violet", 0xee82ee),
    ("crimson", 0xdc143c),
    ("skyblue", 0x87ceeb),
];

pub(in crate::parse) fn color_number(raw: &str, scale: f32, what: &str) -> Result<f32> {
    let n = match raw.strip_suffix('%') {
        Some(p) => p.parse::<f32>().map(|p| p * scale / 100.0),
        None => raw.parse::<f32>(),
    }
    .with_context(|| format!("{what}: invalid number: {raw}"))?;
    if !n.is_finite() {
        bail!("{what}: value is not finite");
    }
    Ok(n)
}

pub(in crate::parse) fn color_alpha(raw: Option<&&str>) -> Result<u8> {
    let alpha = match raw {
        Some(raw) => color_number(raw, 1.0, "alpha")?,
        None => 1.0,
    };
    Ok((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub(in crate::parse) fn parse_color_function(name: &str, args: &str, raw: &str) -> Result<Color> {
    let normalized = args.replace([',', '/'], " ");
    let parts: Vec<_> = normalized.split_whitespace().collect();
    if !(3..=4).contains(&parts.len()) {
        bail!("unsupported CSS color: {raw}");
    }
    match name {
        "rgb" | "rgba" => {
            let channel = |i: usize| -> Result<u8> {
                Ok(color_number(parts[i], 255.0, "rgb")?
                    .clamp(0.0, 255.0)
                    .round() as u8)
            };
            Ok(Color {
                r: channel(0)?,
                g: channel(1)?,
                b: channel(2)?,
                a: color_alpha(parts.get(3))?,
            })
        }
        "hsl" | "hsla" => {
            let hue: f32 = parts[0]
                .trim_end_matches("deg")
                .parse()
                .with_context(|| format!("hsl: invalid hue: {raw}"))?;
            let percent = |i: usize| -> Result<f32> {
                if !parts[i].ends_with('%') {
                    bail!("hsl: saturation and lightness need %: {raw}");
                }
                Ok((color_number(parts[i], 1.0, "hsl")?).clamp(0.0, 1.0))
            };
            let (s, l) = (percent(1)?, percent(2)?);
            let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
            let h = hue.rem_euclid(360.0) / 60.0;
            let x = c * (1.0 - (h % 2.0 - 1.0).abs());
            let (r, g, b) = match h as u32 {
                0 => (c, x, 0.0),
                1 => (x, c, 0.0),
                2 => (0.0, c, x),
                3 => (0.0, x, c),
                4 => (x, 0.0, c),
                _ => (c, 0.0, x),
            };
            let m = l - c / 2.0;
            let byte = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
            Ok(Color {
                r: byte(r),
                g: byte(g),
                b: byte(b),
                a: color_alpha(parts.get(3))?,
            })
        }
        _ => bail!("unsupported CSS color: {raw}"),
    }
}

pub fn parse_color(raw: &str) -> Result<Color> {
    let value = raw.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        let digits = match hex.len() {
            3 | 4 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
            6 | 8 => hex.to_string(),
            _ => bail!("unsupported hex color: {raw}"),
        };
        let n =
            u32::from_str_radix(&digits, 16).with_context(|| format!("invalid color: {raw}"))?;
        return Ok(if digits.len() == 8 {
            Color {
                r: (n >> 24) as u8,
                g: (n >> 16) as u8,
                b: (n >> 8) as u8,
                a: n as u8,
            }
        } else {
            Color {
                r: (n >> 16) as u8,
                g: (n >> 8) as u8,
                b: n as u8,
                a: 255,
            }
        });
    }
    if let Some((name, args)) = value.split_once('(') {
        let args = args
            .strip_suffix(')')
            .ok_or_else(|| anyhow!("CSS color without ')': {raw}"))?;
        return parse_color_function(name.trim(), args, raw);
    }
    if value == "transparent" {
        return Ok(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        });
    }
    NAMED_COLORS
        .iter()
        .find(|(name, _)| *name == value)
        .map(|&(_, n)| Color {
            r: (n >> 16) as u8,
            g: (n >> 8) as u8,
            b: n as u8,
            a: 255,
        })
        .ok_or_else(|| anyhow!("unsupported CSS color: {raw}"))
}
