use crate::model::{
    Background, Color, Composition, Content, Easing, Element, Keyframe, Length, Motion, Pose,
    Shadow, Style, Track, TrackPoint,
};
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::traits::*;
use kuchiki::NodeRef;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

#[derive(Debug)]
struct Rule {
    selector: String,
    declarations: Vec<(String, String)>,
    specificity: usize,
    order: usize,
}

#[derive(Debug)]
struct Pending {
    element: Element,
    start_expr: Option<String>,
    duration: Option<f64>,
    keyframes: Vec<Keyframe>,
    animation: Option<AnimationSpec>,
}

/// Declared in `@keyframes`, before values are resolved against an element.
#[derive(Debug)]
struct RawFrame {
    offset: f64,
    declarations: Vec<(String, String)>,
}

#[derive(Default, Debug)]
struct Stylesheet {
    rules: Vec<Rule>,
    keyframes: HashMap<String, Vec<RawFrame>>,
}

/// Custom properties (`--name`) visible to an element.
type Vars = HashMap<String, String>;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Iterations {
    Count(u32),
    Infinite,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Direction {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

#[derive(Clone, Debug)]
struct AnimationDecl {
    name: Option<String>,
    duration: Option<f64>,
    delay: f64,
    easing: Easing,
    iterations: Iterations,
    direction: Direction,
    backwards: bool,
    forwards: bool,
}

impl Default for AnimationDecl {
    fn default() -> Self {
        Self {
            name: None,
            duration: None,
            delay: 0.0,
            easing: CSS_EASE,
            iterations: Iterations::Count(1),
            direction: Direction::Normal,
            backwards: false,
            forwards: false,
        }
    }
}

/// One keyframe with values already resolved for a specific element.
#[derive(Clone, Debug)]
struct CssFrame {
    offset: f64,
    /// `transform` as (translate x, translate y, scale).
    transform: Option<(f32, f32, f32)>,
    opacity: Option<f32>,
    easing: Option<Easing>,
}

#[derive(Clone, Debug)]
struct AnimationSpec {
    name: String,
    frames: Vec<CssFrame>,
    decl: AnimationDecl,
    duration: f64,
}

struct Resolved {
    style: Style,
    vars: Vars,
    animation: Option<AnimationSpec>,
}

const CSS_EASE: Easing = Easing::Bezier([0.25, 0.1, 0.25, 1.0]);
const MAX_TRACK_POINTS: usize = 50_000;

fn attr(node: &NodeRef, key: &str) -> Option<String> {
    node.as_element()
        .and_then(|e| e.attributes.borrow().get(key).map(str::to_owned))
}

fn number(value: &str, name: &str) -> Result<f64> {
    let n: f64 = value
        .parse()
        .with_context(|| format!("{name}: invalid number: {value}"))?;
    if !n.is_finite() || n < 0.0 {
        bail!("{name}: must be a finite non-negative number: {value}");
    }
    Ok(n)
}

fn strip_comments(css: &str) -> Result<String> {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut closed = false;
            while let Some(x) = chars.next() {
                if x == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    closed = true;
                    break;
                }
            }
            if !closed {
                bail!("unclosed CSS comment");
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

fn declarations(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for declaration in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let (key, value) = declaration
            .split_once(':')
            .ok_or_else(|| anyhow!("invalid CSS declaration: {declaration}"))?;
        let key = key.trim();
        let key = if key.starts_with("--") {
            key.to_string()
        } else {
            key.to_ascii_lowercase()
        };
        let value = value.trim().to_string();
        if value.is_empty() {
            bail!("empty CSS value for {key}");
        }
        out.push((key, value));
    }
    Ok(out)
}

fn valid_simple_selector(token: &str) -> bool {
    token == ":root"
        || (!token.is_empty()
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '*' | '-' | '_' | '#' | '.')))
}

fn specificity(selector: &str) -> usize {
    selector.bytes().filter(|c| *c == b'#').count() * 100
        + selector.bytes().filter(|c| *c == b'.').count() * 10
        + selector.matches(":root").count() * 10
        + selector
            .split_whitespace()
            .filter(|s| !s.starts_with(['.', '#', '*', ':']))
            .count()
}

fn matching_brace(text: &str, open: usize) -> Result<usize> {
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(open + i);
                }
            }
            _ => {}
        }
    }
    bail!("CSS block without '}}'")
}

fn parse_keyframe_blocks(body: &str) -> Result<Vec<RawFrame>> {
    let mut rest = body;
    let mut frames = Vec::new();
    while !rest.trim().is_empty() {
        let open = rest
            .find('{')
            .ok_or_else(|| anyhow!("@keyframes: block without '{{'"))?;
        let close = matching_brace(rest, open)?;
        let declarations = declarations(&rest[open + 1..close])?;
        for selector in rest[..open].split(',').map(str::trim) {
            let offset = match selector {
                "from" => 0.0,
                "to" => 1.0,
                _ => {
                    let pct: f64 = selector
                        .strip_suffix('%')
                        .and_then(|n| n.trim().parse().ok())
                        .filter(|n| (0.0..=100.0).contains(n))
                        .ok_or_else(|| {
                            anyhow!("@keyframes: invalid selector (use from, to or 0%-100%): {selector}")
                        })?;
                    pct / 100.0
                }
            };
            frames.push(RawFrame {
                offset,
                declarations: declarations.clone(),
            });
        }
        rest = &rest[close + 1..];
    }
    if frames.is_empty() {
        bail!("@keyframes without frames");
    }
    Ok(frames)
}

fn parse_css(css: &str, sheet: &mut Stylesheet) -> Result<()> {
    let css = strip_comments(css)?;
    let mut rest = css.as_str();
    while !rest.trim().is_empty() {
        let open = rest
            .find('{')
            .ok_or_else(|| anyhow!("CSS block is missing '{{'"))?;
        let close = matching_brace(rest, open)?;
        let prelude = rest[..open].trim();
        let body = &rest[open + 1..close];
        if let Some(name) = prelude
            .strip_prefix("@keyframes")
            .filter(|n| n.starts_with(char::is_whitespace))
        {
            let name = name.trim();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            {
                bail!("@keyframes: invalid name: {name}");
            }
            sheet
                .keyframes
                .insert(name.to_string(), parse_keyframe_blocks(body)?);
        } else if prelude.starts_with('@') {
            bail!("unsupported CSS at-rule (only @keyframes): {prelude}");
        } else {
            let decls = declarations(body)?;
            for selector in prelude.split(',').map(str::trim) {
                if selector.is_empty() || !selector.split_whitespace().all(valid_simple_selector) {
                    bail!("unsupported CSS selector: {selector}");
                }
                sheet.rules.push(Rule {
                    selector: selector.to_string(),
                    declarations: decls.clone(),
                    specificity: specificity(selector),
                    order: sheet.rules.len(),
                });
            }
        }
        rest = &rest[close + 1..];
    }
    Ok(())
}

fn matches_simple(node: &NodeRef, token: &str) -> bool {
    let Some(element) = node.as_element() else {
        return false;
    };
    let tag = element.name.local.as_ref();
    if token == ":root" {
        return tag == "html";
    }
    let attributes = element.attributes.borrow();
    let mut name = String::new();
    let mut mode = 't';
    let mut checks = Vec::new();
    for ch in token.chars().chain(std::iter::once('.')) {
        if ch == '#' || ch == '.' {
            if !name.is_empty() {
                checks.push((mode, name.clone()));
                name.clear();
            }
            mode = ch;
        } else {
            name.push(ch);
        }
    }
    checks.into_iter().all(|(mode, value)| match mode {
        't' => value == "*" || tag == value,
        '#' => attributes.get("id") == Some(value.as_str()),
        '.' => attributes
            .get("class")
            .is_some_and(|classes| classes.split_whitespace().any(|c| c == value)),
        _ => false,
    })
}

fn matches_selector(node: &NodeRef, selector: &str) -> bool {
    let parts: Vec<_> = selector.split_whitespace().collect();
    if !matches_simple(node, parts[parts.len() - 1]) {
        return false;
    }
    let mut ancestor = node.parent();
    for token in parts[..parts.len() - 1].iter().rev() {
        let Some(found) = ancestor.as_ref().and_then(|a| {
            std::iter::once(a.clone())
                .chain(a.ancestors())
                .find(|candidate| matches_simple(candidate, token))
        }) else {
            return false;
        };
        ancestor = found.parent();
    }
    true
}

fn parse_length(raw: &str, property: &str) -> Result<Length> {
    let raw = raw.trim();
    if let Some(s) = raw.strip_suffix('%') {
        return Ok(Length::Percent(
            s.trim()
                .parse()
                .with_context(|| format!("{property}: invalid percentage"))?,
        ));
    }
    let s = raw.strip_suffix("px").unwrap_or(raw).trim();
    let n: f32 = s
        .parse()
        .with_context(|| format!("{property}: only px and % are supported: {raw}"))?;
    if !n.is_finite() {
        bail!("{property}: non-finite value");
    }
    Ok(Length::Px(n))
}

const NAMED_COLORS: &[(&str, u32)] = &[
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

/// Splits at `separator` outside parentheses.
fn split_top(text: &str, separator: char) -> Vec<&str> {
    let (mut parts, mut depth, mut start) = (Vec::new(), 0usize, 0usize);
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if depth == 0 && (c == separator || (separator == ' ' && c.is_whitespace())) => {
                parts.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

fn color_number(raw: &str, scale: f32, what: &str) -> Result<f32> {
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

fn color_alpha(raw: Option<&&str>) -> Result<u8> {
    let alpha = match raw {
        Some(raw) => color_number(raw, 1.0, "alpha")?,
        None => 1.0,
    };
    Ok((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn parse_color_function(name: &str, args: &str, raw: &str) -> Result<Color> {
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

fn parse_background(raw: &str) -> Result<Background> {
    if let Some(body) = raw
        .strip_prefix("linear-gradient(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts = split_top(body, ',');
        let (horizontal, from, to) = match parts.as_slice() {
            ["to bottom", from, to] | [from, to] => (false, from, to),
            ["to top", from, to] => (false, to, from),
            ["to right", from, to] => (true, from, to),
            ["to left", from, to] => (true, to, from),
            _ => bail!(
                "linear-gradient: use a horizontal or vertical direction and two opaque colors"
            ),
        };
        let (from, to) = (parse_color(from)?, parse_color(to)?);
        if from.a != 255 || to.a != 255 {
            bail!("linear-gradient requires two opaque colors");
        }
        return Ok(Background::Linear {
            from,
            to,
            horizontal,
        });
    }
    Ok(Background::Solid(parse_color(raw)?))
}

/// Replaces every `var(--name, fallback)` with its value.
fn substitute(value: &str, vars: &Vars, depth: usize) -> Result<String> {
    if depth > 16 {
        bail!("var() is circular or nested too deeply: {value}");
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(pos) = rest.find("var(") {
        out.push_str(&rest[..pos]);
        let args = pos + 4;
        let mut level = 1usize;
        let mut end = None;
        for (i, c) in rest[args..].char_indices() {
            match c {
                '(' => level += 1,
                ')' => {
                    level -= 1;
                    if level == 0 {
                        end = Some(args + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.ok_or_else(|| anyhow!("var() without ')': {value}"))?;
        let inner = &rest[args..end];
        let (name, fallback) = match inner.split_once(',') {
            Some((name, fallback)) => (name.trim(), Some(fallback.trim())),
            None => (inner.trim(), None),
        };
        if !name.starts_with("--") {
            bail!("var(): expected --name, got {name}");
        }
        let resolved = match (vars.get(name), fallback) {
            (Some(found), _) => substitute(found, vars, depth + 1)?,
            (None, Some(fallback)) => substitute(fallback, vars, depth + 1)?,
            (None, None) => bail!("CSS variable not defined: {name}"),
        };
        out.push_str(&resolved);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `transform` as (translate x, translate y, uniform scale), composed left to right.
fn parse_transform(raw: &str) -> Result<(f32, f32, f32)> {
    let raw = raw.trim();
    if raw == "none" {
        return Ok((0.0, 0.0, 1.0));
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
    let (mut tx, mut ty, mut scale) = (0.0_f32, 0.0_f32, 1.0_f32);
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
        // Composing `C ∘ F`: a translation written after a scale is scaled as well.
        match (name.as_str(), args.as_slice()) {
            ("translate", [x]) | ("translatex", [x]) => tx += scale * px(x)?,
            ("translatey", [y]) => ty += scale * px(y)?,
            ("translate", [x, y]) => {
                tx += scale * px(x)?;
                ty += scale * px(y)?;
            }
            ("scale", [k]) => scale *= factor(k)?,
            ("scale", [a, b]) if factor(a)? == factor(b)? => scale *= factor(a)?,
            ("scale", [_, _]) => bail!("transform: non-uniform scale is unsupported"),
            _ => bail!(
                "unsupported transform: {name}(...); use translate, translateX, translateY or scale"
            ),
        }
        rest = &text[close + 1..];
    }
    Ok((tx, ty, scale))
}

/// Non-negative px length (`0` allowed) for spacing properties.
fn spacing_px(raw: &str, property: &str) -> Result<f32> {
    match parse_length(raw, property)? {
        Length::Px(n) if n >= 0.0 => Ok(n),
        _ => bail!("{property}: only non-negative px values are supported: {raw}"),
    }
}

/// CSS 1-4 value box shorthand into top, right, bottom, left.
fn parse_box(raw: &str, property: &str) -> Result<[f32; 4]> {
    let v: Vec<f32> = split_top(raw, ' ')
        .into_iter()
        .map(|part| spacing_px(part, property))
        .collect::<Result<_>>()?;
    Ok(match v[..] {
        [a] => [a, a, a, a],
        [v1, h] => [v1, h, v1, h],
        [t, h, b] => [t, h, b, h],
        [t, r, b, l] => [t, r, b, l],
        _ => bail!("{property}: expected 1 to 4 values: {raw}"),
    })
}

fn parse_shadow(raw: &str) -> Result<Option<Shadow>> {
    if raw == "none" {
        return Ok(None);
    }
    if split_top(raw, ',').len() > 1 {
        bail!("box-shadow: several shadows are unsupported: {raw}");
    }
    let mut lengths = Vec::new();
    let mut color = None;
    for token in split_top(raw, ' ') {
        if token == "inset" {
            bail!("box-shadow: inset is unsupported");
        }
        let numeric = token.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'));
        if numeric && !token.contains('(') {
            let n = match parse_length(token, "box-shadow")? {
                Length::Px(n) => n,
                Length::Percent(_) => bail!("box-shadow: only px lengths are supported"),
            };
            lengths.push(n);
        } else if color.replace(parse_color(token)?).is_some() {
            bail!("box-shadow: more than one color: {raw}");
        }
    }
    let [x, y, blur, spread] = match lengths[..] {
        [x, y] => [x, y, 0.0, 0.0],
        [x, y, blur] => [x, y, blur, 0.0],
        [x, y, blur, spread] => [x, y, blur, spread],
        _ => bail!("box-shadow: expected offset-x offset-y [blur [spread]] color: {raw}"),
    };
    if blur < 0.0 {
        bail!("box-shadow: blur must not be negative");
    }
    Ok(Some(Shadow {
        x,
        y,
        blur,
        spread,
        color: color.unwrap_or(Color::BLACK),
    }))
}

fn parse_time(raw: &str) -> Result<f64> {
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

fn parse_timing(raw: &str) -> Result<Easing> {
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

fn parse_iterations(raw: &str) -> Result<Iterations> {
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

fn parse_direction(raw: &str) -> Option<Direction> {
    match raw {
        "normal" => Some(Direction::Normal),
        "reverse" => Some(Direction::Reverse),
        "alternate" => Some(Direction::Alternate),
        "alternate-reverse" => Some(Direction::AlternateReverse),
        _ => None,
    }
}

fn parse_fill(raw: &str) -> Option<(bool, bool)> {
    match raw {
        "none" => Some((false, false)),
        "forwards" => Some((false, true)),
        "backwards" => Some((true, false)),
        "both" => Some((true, true)),
        _ => None,
    }
}

fn single_animation(value: &str) -> Result<()> {
    if split_top(value, ',').len() > 1 {
        bail!("animation: several animations per element are unsupported: {value}");
    }
    Ok(())
}

fn parse_animation_shorthand(value: &str) -> Result<AnimationDecl> {
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

fn apply_animation_longhand(decl: &mut AnimationDecl, key: &str, value: &str) -> Result<()> {
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

/// Resolves `@keyframes` for one element: values (with `var()`), merge of equal offsets.
fn resolve_animation(
    decl: AnimationDecl,
    keyframes: &HashMap<String, Vec<RawFrame>>,
    vars: &Vars,
) -> Result<Option<AnimationSpec>> {
    let Some(name) = decl.name.clone() else {
        return Ok(None);
    };
    let raw = keyframes
        .get(&name)
        .ok_or_else(|| anyhow!("animation-name: @keyframes {name} is not defined"))?;
    let duration = decl
        .duration
        .ok_or_else(|| anyhow!("animation {name}: animation-duration is missing"))?;
    if duration < 0.001 {
        bail!("animation {name}: animation-duration must be at least 1ms");
    }
    let mut ordered: Vec<&RawFrame> = raw.iter().collect();
    ordered.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let mut frames: Vec<CssFrame> = Vec::new();
    for frame in ordered {
        if frames
            .last()
            .is_none_or(|last| (last.offset - frame.offset).abs() > 1e-9)
        {
            frames.push(CssFrame {
                offset: frame.offset,
                transform: None,
                opacity: None,
                easing: None,
            });
        }
        let current = frames.last_mut().unwrap();
        for (key, raw_value) in &frame.declarations {
            let value = substitute(raw_value, vars, 0)?;
            match key.as_str() {
                "transform" => current.transform = Some(parse_transform(&value)?),
                "opacity" => {
                    let n: f32 = value.parse().context("invalid opacity")?;
                    if !n.is_finite() || !(0.0..=1.0).contains(&n) {
                        bail!("opacity is outside 0..1");
                    }
                    current.opacity = Some(n);
                }
                "animation-timing-function" => current.easing = Some(parse_timing(&value)?),
                _ => bail!(
                    "@keyframes {name}: unsupported property: {key} (only transform, opacity and animation-timing-function)"
                ),
            }
        }
    }
    Ok(Some(AnimationSpec {
        name,
        frames,
        decl,
        duration,
    }))
}

/// Expands iterations, direction and fill into per-channel tracks for a clip of `clip_len` seconds.
fn build_motion(spec: &AnimationSpec, base: Pose, clip_len: f64) -> Result<Motion> {
    let decl = &spec.decl;
    let count = match decl.iterations {
        Iterations::Count(n) => n as usize,
        Iterations::Infinite => ((clip_len - decl.delay) / spec.duration).ceil().max(1.0) as usize,
    };
    let channel = |get: &dyn Fn(&CssFrame) -> Option<f32>, rest: f32| -> Result<Track> {
        let mut sparse: Vec<(f64, f32, Easing)> = spec
            .frames
            .iter()
            .filter_map(|f| get(f).map(|v| (f.offset, v, f.easing.unwrap_or(decl.easing))))
            .collect();
        if sparse.is_empty() {
            return Ok(Track::default());
        }
        if sparse[0].0 > 0.0 {
            sparse.insert(0, (0.0, rest, decl.easing));
        }
        if sparse.last().is_some_and(|last| last.0 < 1.0) {
            sparse.push((1.0, rest, decl.easing));
        }
        // The easing declared at a keyframe shapes the interval that starts there.
        let forward: Vec<(f64, f32, Easing)> = sparse
            .iter()
            .enumerate()
            .map(|(k, &(offset, value, _))| {
                let arriving = if k == 0 {
                    Easing::Linear
                } else {
                    sparse[k - 1].2
                };
                (offset, value, arriving)
            })
            .collect();
        let n = forward.len();
        if count.saturating_mul(n) > MAX_TRACK_POINTS {
            bail!(
                "animation {}: generates too many points; raise animation-duration or lower the iteration count",
                spec.name
            );
        }
        let mut points: Vec<TrackPoint> = Vec::with_capacity(count * n);
        for i in 0..count {
            let reverse = match decl.direction {
                Direction::Normal => false,
                Direction::Reverse => true,
                Direction::Alternate => i % 2 == 1,
                Direction::AlternateReverse => i % 2 == 0,
            };
            for j in 0..n {
                let (offset, value, easing) = if reverse {
                    let (offset, value, _) = forward[n - 1 - j];
                    let easing = if j == 0 {
                        Easing::Linear
                    } else {
                        forward[n - j].2.reversed()
                    };
                    (1.0 - offset, value, easing)
                } else {
                    forward[j]
                };
                let at = decl.delay + (i as f64 + offset) * spec.duration;
                // Two points may share `at` at an iteration boundary: the later one wins from there on.
                points.push(TrackPoint { at, value, easing });
            }
        }
        if points.windows(2).any(|w| w[1].at < w[0].at) {
            bail!("animation {}: keyframes are too close together", spec.name);
        }
        Ok(Track(points))
    };
    Ok(Motion {
        base,
        x: channel(&|f| f.transform.map(|t| t.0), base.x)?,
        y: channel(&|f| f.transform.map(|t| t.1), base.y)?,
        scale: channel(&|f| f.transform.map(|t| t.2), base.scale)?,
        opacity: channel(&|f| f.opacity, base.opacity)?,
        hold_before: decl.backwards,
        hold_after: decl.forwards,
    })
}

/// `data-hf-keyframes` offsets add to the resting pose; opacity multiplies it.
fn motion_from_keyframes(base: Pose, frames: &[Keyframe]) -> Motion {
    let track = |value: &dyn Fn(&Keyframe) -> f32| {
        Track(
            frames
                .iter()
                .map(|k| TrackPoint {
                    at: k.at,
                    value: value(k),
                    easing: k.easing,
                })
                .collect(),
        )
    };
    Motion {
        base,
        x: track(&|k| base.x + k.x),
        y: track(&|k| base.y + k.y),
        scale: Track::default(),
        opacity: track(&|k| base.opacity * k.opacity),
        hold_before: true,
        hold_after: true,
    }
}

fn cascade(node: &NodeRef, rules: &[Rule]) -> Result<Vec<(String, String)>> {
    let mut matches: Vec<_> = rules
        .iter()
        .filter(|r| matches_selector(node, &r.selector))
        .collect();
    matches.sort_by_key(|r| (r.specificity, r.order));
    let mut properties = Vec::<(String, String)>::new();
    for rule in matches {
        properties.extend(rule.declarations.iter().cloned());
    }
    if let Some(inline) = attr(node, "style") {
        properties.extend(declarations(&inline)?);
    }
    Ok(properties)
}

fn inherit_vars(properties: &[(String, String)], parent: &Vars) -> Vars {
    let mut vars = parent.clone();
    for (key, value) in properties {
        if key.starts_with("--") {
            vars.insert(key.clone(), value.clone());
        }
    }
    vars
}

fn style_for(node: &NodeRef, sheet: &Stylesheet, parent_vars: &Vars) -> Result<Resolved> {
    let properties = cascade(node, &sheet.rules)?;
    let vars = inherit_vars(&properties, parent_vars);
    let mut animation = AnimationDecl::default();
    let mut style = Style::default();
    for (key, value) in properties {
        if key.starts_with("--") {
            continue;
        }
        let value = substitute(&value, &vars, 0).with_context(|| format!("property {key}"))?;
        match key.as_str() {
            "left" => style.left = Some(parse_length(&value, &key)?),
            "top" => style.top = Some(parse_length(&value, &key)?),
            "width" => style.width = Some(parse_length(&value, &key)?),
            "height" => style.height = Some(parse_length(&value, &key)?),
            "background" => style.background = Some(parse_background(&value)?),
            "background-color" => {
                let color = parse_color(&value)?;
                // The supported gradients are opaque and cover the background color.
                if !matches!(style.background, Some(Background::Linear { .. })) {
                    style.background = Some(Background::Solid(color));
                }
            }
            "border-radius" => match parse_length(&value, &key)? {
                Length::Px(n) if n >= 0.0 => style.border_radius = n,
                _ => bail!("border-radius: only one non-negative radius in px is supported"),
            },
            "color" => style.color = Some(parse_color(&value)?),
            "font-size" => {
                let Length::Px(n) = parse_length(&value, &key)? else {
                    bail!("font-size: only px is supported");
                };
                style.font_size = Some(n);
            }
            "font-family" => style.font_family = Some(value),
            "opacity" => {
                let n: f32 = value.parse().context("invalid opacity")?;
                if !n.is_finite() || !(0.0..=1.0).contains(&n) {
                    bail!("opacity is outside 0..1");
                }
                style.opacity = Some(n);
            }
            "text-align" => {
                if !matches!(value.as_str(), "left" | "center" | "right") {
                    bail!("unsupported text-align: {value}");
                }
                style.text_align = Some(value);
            }
            "display" => {
                if value != "flex" && value != "block" {
                    bail!("unsupported display: {value}");
                }
                style.display = Some(value);
            }
            "align-items" => {
                if value != "center" {
                    bail!("unsupported align-items: {value}");
                }
                style.align_items = Some(value);
            }
            "justify-content" => {
                if value != "center" {
                    bail!("unsupported justify-content: {value}");
                }
                style.justify_content = Some(value);
            }
            "object-fit" => {
                if !matches!(value.as_str(), "fill" | "contain" | "cover") {
                    bail!("unsupported object-fit: {value}");
                }
                style.object_fit = Some(value);
            }
            "z-index" => style.z_index = Some(value.parse().context("invalid z-index")?),
            "position" => {
                if !matches!(value.as_str(), "relative" | "absolute" | "static") {
                    bail!("unsupported position: {value}");
                }
                style.position = Some(value);
            }
            "transform" => {
                let (tx, ty, scale) = parse_transform(&value)?;
                style.translate = (tx, ty);
                style.scale = Some(scale);
            }
            "transform-origin"
                if matches!(value.as_str(), "center" | "center center" | "50% 50%") => {}
            "animation" => animation = parse_animation_shorthand(&value)?,
            k if k.starts_with("animation-") => {
                apply_animation_longhand(&mut animation, k, &value)?
            }
            "margin" if value == "0" || value == "0px" => {}
            "box-sizing" if value == "border-box" => {}
            "overflow" if value == "hidden" => {}
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

fn direct_text(node: &NodeRef) -> String {
    node.children()
        .filter_map(|n| n.as_text().map(|t| t.borrow().to_string()))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_keyframes(raw: &str) -> Result<Vec<Keyframe>> {
    let frames: Vec<BTreeMap<String, serde_json::Value>> =
        serde_json::from_str(raw).context("expected a JSON array of objects")?;
    if frames.is_empty() {
        bail!("at least one keyframe is required");
    }
    let mut previous = Keyframe {
        at: -1.0,
        x: 0.0,
        y: 0.0,
        opacity: 1.0,
        easing: Easing::Linear,
    };
    let mut keyframes = Vec::with_capacity(frames.len());
    for (index, frame) in frames.into_iter().enumerate() {
        let keyframe = (|| -> Result<Keyframe> {
            for key in frame.keys() {
                if !matches!(key.as_str(), "at" | "x" | "y" | "opacity" | "easing") {
                    bail!("unknown field: {key}");
                }
            }
            let get_number = |key: &str, fallback: f64| -> Result<f64> {
                match frame.get(key) {
                    Some(value) => value
                        .as_f64()
                        .ok_or_else(|| anyhow!("{key} must be a number")),
                    None => Ok(fallback),
                }
            };
            let at = get_number("at", -1.0)?;
            if at < 0.0 || at <= previous.at {
                bail!("at is required, and times must be non-negative and increasing");
            }
            let x = get_number("x", previous.x as f64)? as f32;
            let y = get_number("y", previous.y as f64)? as f32;
            if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
                bail!("x and y must be offsets of at most 1,000,000 px");
            }
            let opacity = get_number("opacity", previous.opacity as f64)?;
            if !(0.0..=1.0).contains(&opacity) {
                bail!("opacity must be between 0 and 1");
            }
            let easing = match frame.get("easing") {
                None => Easing::Linear,
                Some(value) => match value.as_str() {
                    Some("linear") => Easing::Linear,
                    Some("ease-in") => Easing::EaseIn,
                    Some("ease-out") => Easing::EaseOut,
                    Some("ease-in-out") => Easing::EaseInOut,
                    _ => bail!("easing must be linear, ease-in, ease-out, or ease-in-out"),
                },
            };
            Ok(Keyframe {
                at,
                x,
                y,
                opacity: opacity as f32,
                easing,
            })
        })()
        .with_context(|| format!("keyframe {}", index + 1))?;
        keyframes.push(keyframe);
        previous = keyframe;
    }
    Ok(keyframes)
}

fn walk(
    node: &NodeRef,
    parent: Option<usize>,
    base: &Path,
    sheet: &Stylesheet,
    vars: &Vars,
    out: &mut Vec<Pending>,
) -> Result<()> {
    let Some(el) = node.as_element() else {
        return Ok(());
    };
    let tag = el.name.local.to_string();
    if matches!(tag.as_str(), "head" | "meta" | "style" | "title" | "link") {
        return Ok(());
    }
    if matches!(
        tag.as_str(),
        "script" | "video" | "audio" | "canvas" | "svg" | "iframe"
    ) {
        bail!("<{tag}> requires a browser/JavaScript and is not supported by the native backend");
    }
    if !matches!(
        tag.as_str(),
        "html"
            | "body"
            | "main"
            | "section"
            | "article"
            | "div"
            | "h1"
            | "h2"
            | "h3"
            | "p"
            | "span"
            | "small"
            | "strong"
            | "img"
    ) {
        bail!("unsupported HTML element: <{tag}>");
    }
    if attr(node, "data-composition-src").is_some() {
        bail!("data-composition-src is not yet supported");
    }
    let Resolved {
        style,
        vars,
        animation,
    } = style_for(node, sheet, vars).with_context(|| format!("styles for <{tag}>"))?;
    let start_expr = attr(node, "data-start");
    let duration = attr(node, "data-duration")
        .map(|s| number(&s, "data-duration"))
        .transpose()?;
    let fade_in = attr(node, "data-hf-fade-in")
        .map(|s| number(&s, "data-hf-fade-in"))
        .transpose()?
        .unwrap_or(0.0);
    let fade_out = attr(node, "data-hf-fade-out")
        .map(|s| number(&s, "data-hf-fade-out"))
        .transpose()?
        .unwrap_or(0.0);
    let keyframes = attr(node, "data-hf-keyframes")
        .map(|raw| parse_keyframes(&raw))
        .transpose()
        .with_context(|| {
            format!(
                "data-hf-keyframes en <{tag}> ({})",
                attr(node, "id").as_deref().unwrap_or("no id")
            )
        })?
        .unwrap_or_default();
    if animation.is_some() && !keyframes.is_empty() {
        bail!(
            "<{tag}> ({}) combines CSS animation and data-hf-keyframes; use only one",
            attr(node, "id").as_deref().unwrap_or("no id")
        );
    }
    let track = attr(node, "data-track-index")
        .map(|s| s.parse::<i32>().context("invalid data-track-index"))
        .transpose()?
        .unwrap_or(0);
    let content = if tag == "img" {
        let src = attr(node, "src").ok_or_else(|| anyhow!("<img> requires src"))?;
        if src.starts_with("data:") || src.contains("://") {
            bail!("only local images are supported: {src}");
        }
        Content::Image(base.join(src))
    } else {
        let text = direct_text(node);
        if text.is_empty() {
            Content::Container
        } else {
            Content::Text(text)
        }
    };
    let idx = out.len();
    let rest = Pose {
        x: style.translate.0,
        y: style.translate.1,
        scale: style.scale.unwrap_or(1.0),
        opacity: style.opacity.unwrap_or(1.0),
    };
    out.push(Pending {
        element: Element {
            tag,
            id: attr(node, "id"),
            parent,
            children: Vec::new(),
            content,
            style,
            start: 0.0,
            end: f64::INFINITY,
            fade_in,
            fade_out,
            track,
            motion: Motion {
                base: rest,
                ..Motion::default()
            },
        },
        start_expr,
        duration,
        keyframes,
        animation,
    });
    if let Some(parent_idx) = parent {
        out[parent_idx].element.children.push(idx);
    }
    for child in node.children() {
        walk(&child, Some(idx), base, sheet, &vars, out)?;
    }
    Ok(())
}

fn resolve_start(
    idx: usize,
    pending: &[Pending],
    ids: &HashMap<String, usize>,
    memo: &mut [Option<f64>],
    visiting: &mut [bool],
) -> Result<f64> {
    if let Some(n) = memo[idx] {
        return Ok(n);
    }
    if visiting[idx] {
        bail!(
            "cycle in data-start near {}",
            pending[idx]
                .element
                .id
                .as_deref()
                .unwrap_or("element with no id")
        );
    }
    visiting[idx] = true;
    let raw = pending[idx].start_expr.as_deref();
    let start = if let Some(raw) = raw {
        if let Ok(n) = number(raw, "data-start") {
            n
        } else {
            let normalized = raw.replace(' ', "");
            let split = normalized
                .char_indices()
                .skip(1)
                .find(|(_, c)| *c == '+' || *c == '-');
            let (id, offset) = if let Some((pos, sign)) = split {
                let offset = number(&normalized[pos + 1..], "offset data-start")?;
                (
                    &normalized[..pos],
                    if sign == '-' { -offset } else { offset },
                )
            } else {
                (normalized.as_str(), 0.0)
            };
            let referenced = *ids
                .get(id)
                .ok_or_else(|| anyhow!("data-start references unknown id: {id}"))?;
            let duration = pending[referenced]
                .duration
                .ok_or_else(|| anyhow!("data-start references {id} without data-duration"))?;
            resolve_start(referenced, pending, ids, memo, visiting)? + duration + offset
        }
    } else if let Some(parent) = pending[idx].element.parent {
        resolve_start(parent, pending, ids, memo, visiting)?
    } else {
        0.0
    };
    if !start.is_finite() || start < 0.0 {
        bail!("resolved data-start is out of range");
    }
    memo[idx] = Some(start);
    visiting[idx] = false;
    Ok(start)
}

pub fn parse_file(path: &Path) -> Result<Composition> {
    let html =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let document = kuchiki::parse_html().one(html);
    if let Some(unsupported) = document
        .select("script, video, audio, canvas, svg, iframe")
        .map_err(|_| anyhow!("error selecting HTML elements"))?
        .next()
    {
        bail!(
            "<{}> requires a browser/JavaScript and is not supported by the native backend",
            unsupported.name.local
        );
    }
    let metadata = document
        .select_first("[data-composition-id][data-width][data-height]")
        .map_err(|_| anyhow!("missing data-composition-id, data-width, or data-height"))?;
    let width: u32 = attr(metadata.as_node(), "data-width")
        .unwrap()
        .parse()
        .context("invalid data-width")?;
    let height: u32 = attr(metadata.as_node(), "data-height")
        .unwrap()
        .parse()
        .context("invalid data-height")?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        bail!("dimensions are outside 1..8192");
    }
    let mut sheet = Stylesheet::default();
    for node in document
        .select("style")
        .map_err(|_| anyhow!("error selecting CSS"))?
    {
        parse_css(&node.text_contents(), &mut sheet)?;
    }
    // Custom properties declared on `:root` (the <html> element) inherit into the composition.
    let mut root_vars = Vars::new();
    if let Ok(html) = document.select_first("html") {
        let properties = cascade(html.as_node(), &sheet.rules)?;
        root_vars = inherit_vars(&properties, &root_vars);
    }
    let root = if metadata
        .as_node()
        .as_element()
        .is_some_and(|e| e.name.local.as_ref() == "meta")
    {
        document
            .select_first("body")
            .map_err(|_| anyhow!("missing <body>"))?
            .as_node()
            .clone()
    } else {
        metadata.as_node().clone()
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut pending = Vec::new();
    walk(&root, None, base, &sheet, &root_vars, &mut pending)?;
    if pending.is_empty() {
        bail!("empty composition");
    }
    let mut ids = HashMap::new();
    for (idx, item) in pending.iter().enumerate() {
        if let Some(id) = &item.element.id {
            if ids.insert(id.clone(), idx).is_some() {
                bail!("duplicate id: {id}");
            }
        }
    }
    let mut memo = vec![None; pending.len()];
    let mut visiting = vec![false; pending.len()];
    for idx in 0..pending.len() {
        resolve_start(idx, &pending, &ids, &mut memo, &mut visiting)?;
    }
    let explicit_duration = attr(metadata.as_node(), "data-duration")
        .map(|s| number(&s, "data-duration"))
        .transpose()?;
    let inferred_duration = pending
        .iter()
        .enumerate()
        .filter_map(|(idx, p)| {
            p.duration
                .or_else(|| {
                    if p.start_expr.is_some() && matches!(p.element.content, Content::Image(_)) {
                        Some(3.0)
                    } else {
                        None
                    }
                })
                .map(|d| memo[idx].unwrap() + d)
        })
        .fold(0.0_f64, f64::max);
    let duration = explicit_duration.unwrap_or(inferred_duration);
    if duration <= 0.0 {
        bail!("unknown duration: add data-duration to the root or clips");
    }
    let mut elements = Vec::with_capacity(pending.len());
    for (idx, mut p) in pending.into_iter().enumerate() {
        p.element.start = memo[idx].unwrap();
        p.element.end = if let Some(d) = p.duration {
            p.element.start + d
        } else if p.start_expr.is_some() && matches!(p.element.content, Content::Image(_)) {
            p.element.start + 3.0
        } else {
            duration
        };
        if p.element.end > duration + 1e-9 {
            bail!(
                "clip {} exceeds composition duration",
                p.element.id.as_deref().unwrap_or("no id")
            );
        }
        if p.element.fade_in + p.element.fade_out > p.element.end - p.element.start + 1e-9 {
            bail!("fade-in + fade-out exceeds the clip duration");
        }
        let clip_len = p.element.end - p.element.start;
        let label = p.element.id.as_deref().unwrap_or("element with no id");
        let rest = p.element.motion.base;
        if let Some(spec) = &p.animation {
            p.element.motion = build_motion(spec, rest, clip_len)
                .with_context(|| format!("CSS animation on {label}"))?;
        } else if !p.keyframes.is_empty() {
            if p.keyframes
                .last()
                .is_some_and(|frame| frame.at > clip_len + 1e-9)
            {
                bail!("data-hf-keyframes exceeds the duration of {label}");
            }
            p.element.motion = motion_from_keyframes(rest, &p.keyframes);
        }
        elements.push(p.element);
    }
    let background = match elements[0].style.background {
        Some(Background::Solid(color)) => color,
        _ => Color::BLACK,
    };
    Ok(Composition {
        width,
        height,
        duration,
        background,
        elements,
        ids,
    })
}
