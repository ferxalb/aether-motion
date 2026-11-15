use crate::model::{
    Background, Color, Composition, Content, Easing, Element, Keyframe, Length, Style,
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
}

fn attr(node: &NodeRef, key: &str) -> Option<String> {
    node.as_element()
        .and_then(|e| e.attributes.borrow().get(key).map(str::to_owned))
}

fn number(value: &str, name: &str) -> Result<f64> {
    let n: f64 = value
        .parse()
        .with_context(|| format!("{name}: número inválido: {value}"))?;
    if !n.is_finite() || n < 0.0 {
        bail!("{name}: debe ser un número finito no negativo: {value}");
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
                bail!("comentario CSS sin cerrar");
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
            .ok_or_else(|| anyhow!("declaración CSS inválida: {declaration}"))?;
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if value.is_empty() {
            bail!("valor CSS vacío para {key}");
        }
        out.push((key, value));
    }
    Ok(out)
}

fn valid_simple_selector(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '*' | '-' | '_' | '#' | '.'))
}

fn specificity(selector: &str) -> usize {
    selector.bytes().filter(|c| *c == b'#').count() * 100
        + selector.bytes().filter(|c| *c == b'.').count() * 10
        + selector
            .split_whitespace()
            .filter(|s| !s.starts_with(['.', '#', '*']))
            .count()
}

fn parse_css(css: &str, offset: usize) -> Result<Vec<Rule>> {
    let css = strip_comments(css)?;
    let mut rest = css.as_str();
    let mut rules = Vec::new();
    while !rest.trim().is_empty() {
        let open = rest
            .find('{')
            .ok_or_else(|| anyhow!("bloque CSS sin '{{'"))?;
        let close = rest[open + 1..]
            .find('}')
            .ok_or_else(|| anyhow!("bloque CSS sin '}}'"))?
            + open
            + 1;
        let selector_group = rest[..open].trim();
        if selector_group.starts_with('@') {
            bail!("reglas CSS @ no soportadas: {selector_group}");
        }
        let decls = declarations(&rest[open + 1..close])?;
        for selector in selector_group.split(',').map(str::trim) {
            if selector.is_empty() || !selector.split_whitespace().all(valid_simple_selector) {
                bail!("selector CSS no soportado: {selector}");
            }
            rules.push(Rule {
                selector: selector.to_string(),
                declarations: decls.clone(),
                specificity: specificity(selector),
                order: offset + rules.len(),
            });
        }
        rest = &rest[close + 1..];
    }
    Ok(rules)
}

fn matches_simple(node: &NodeRef, token: &str) -> bool {
    let Some(element) = node.as_element() else {
        return false;
    };
    let tag = element.name.local.as_ref();
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
                .with_context(|| format!("{property}: porcentaje inválido"))?,
        ));
    }
    let s = raw.strip_suffix("px").unwrap_or(raw).trim();
    let n: f32 = s
        .parse()
        .with_context(|| format!("{property}: solo px y % son soportados: {raw}"))?;
    if !n.is_finite() {
        bail!("{property}: valor no finito");
    }
    Ok(Length::Px(n))
}

pub fn parse_color(raw: &str) -> Result<Color> {
    let value = raw.trim().to_ascii_lowercase();
    let hex = value.strip_prefix('#');
    if let Some(hex) = hex {
        let bytes = match hex.len() {
            3 => hex.chars().map(|c| format!("{c}{c}")).collect::<String>(),
            6 => hex.to_string(),
            _ => bail!("color hex no soportado: {raw}"),
        };
        let n =
            u32::from_str_radix(&bytes, 16).with_context(|| format!("color inválido: {raw}"))?;
        return Ok(Color {
            r: (n >> 16) as u8,
            g: (n >> 8) as u8,
            b: n as u8,
            a: 255,
        });
    }
    let color = match value.as_str() {
        "black" => Color::BLACK,
        "white" => Color::WHITE,
        "red" => Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        },
        "green" => Color {
            r: 0,
            g: 128,
            b: 0,
            a: 255,
        },
        "blue" => Color {
            r: 0,
            g: 0,
            b: 255,
            a: 255,
        },
        "transparent" => Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        },
        _ => bail!("color CSS no soportado: {raw}"),
    };
    Ok(color)
}

fn parse_background(raw: &str) -> Result<Background> {
    if let Some(body) = raw
        .strip_prefix("linear-gradient(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<_> = body.split(',').map(str::trim).collect();
        let (horizontal, from, to) = match parts.as_slice() {
            ["to bottom", from, to] | [from, to] => (false, from, to),
            ["to top", from, to] => (false, to, from),
            ["to right", from, to] => (true, from, to),
            ["to left", from, to] => (true, to, from),
            _ => bail!(
                "linear-gradient: usa una dirección horizontal o vertical y dos colores opacos"
            ),
        };
        let (from, to) = (parse_color(from)?, parse_color(to)?);
        if from.a != 255 || to.a != 255 {
            bail!("linear-gradient: se necesitan dos colores opacos");
        }
        return Ok(Background::Linear {
            from,
            to,
            horizontal,
        });
    }
    Ok(Background::Solid(parse_color(raw)?))
}

fn style_for(node: &NodeRef, rules: &[Rule]) -> Result<Style> {
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
    let mut style = Style::default();
    for (key, value) in properties {
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
                _ => bail!("border-radius: solo un radio no negativo en px"),
            },
            "color" => style.color = Some(parse_color(&value)?),
            "font-size" => {
                let Length::Px(n) = parse_length(&value, &key)? else {
                    bail!("font-size: solo px");
                };
                style.font_size = Some(n);
            }
            "font-family" => style.font_family = Some(value),
            "opacity" => {
                let n: f32 = value.parse().context("opacity inválida")?;
                if !n.is_finite() || !(0.0..=1.0).contains(&n) {
                    bail!("opacity fuera de 0..1");
                }
                style.opacity = Some(n);
            }
            "text-align" => {
                if !matches!(value.as_str(), "left" | "center" | "right") {
                    bail!("text-align no soportado: {value}");
                }
                style.text_align = Some(value);
            }
            "display" => {
                if value != "flex" && value != "block" {
                    bail!("display no soportado: {value}");
                }
                style.display = Some(value);
            }
            "align-items" => {
                if value != "center" {
                    bail!("align-items no soportado: {value}");
                }
                style.align_items = Some(value);
            }
            "justify-content" => {
                if value != "center" {
                    bail!("justify-content no soportado: {value}");
                }
                style.justify_content = Some(value);
            }
            "object-fit" => {
                if !matches!(value.as_str(), "fill" | "contain" | "cover") {
                    bail!("object-fit no soportado: {value}");
                }
                style.object_fit = Some(value);
            }
            "z-index" => style.z_index = Some(value.parse().context("z-index inválido")?),
            "position" => {
                if !matches!(value.as_str(), "relative" | "absolute" | "static") {
                    bail!("position no soportado: {value}");
                }
                style.position = Some(value);
            }
            "margin" | "padding" if value == "0" || value == "0px" => {}
            "box-sizing" if value == "border-box" => {}
            "overflow" if value == "hidden" => {}
            "visibility"
                if value == "hidden"
                    && attr(node, "class")
                        .is_some_and(|c| c.split_whitespace().any(|x| x == "clip")) => {}
            _ => bail!(
                "propiedad CSS no soportada en <{}>: {key}: {value}",
                node.as_element().unwrap().name.local
            ),
        }
    }
    Ok(style)
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
        serde_json::from_str(raw).context("se esperaba un array JSON de objetos")?;
    if frames.is_empty() {
        bail!("se necesita al menos un fotograma clave");
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
                    bail!("campo desconocido: {key}");
                }
            }
            let get_number = |key: &str, fallback: f64| -> Result<f64> {
                match frame.get(key) {
                    Some(value) => value
                        .as_f64()
                        .ok_or_else(|| anyhow!("{key} requiere un número")),
                    None => Ok(fallback),
                }
            };
            let at = get_number("at", -1.0)?;
            if at < 0.0 || at <= previous.at {
                bail!("at es obligatorio y los tiempos deben ser no negativos y ascendentes");
            }
            let x = get_number("x", previous.x as f64)? as f32;
            let y = get_number("y", previous.y as f64)? as f32;
            if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
                bail!("x e y deben ser desplazamientos de hasta 1.000.000 px");
            }
            let opacity = get_number("opacity", previous.opacity as f64)?;
            if !(0.0..=1.0).contains(&opacity) {
                bail!("opacity debe estar entre 0 y 1");
            }
            let easing = match frame.get("easing") {
                None => Easing::Linear,
                Some(value) => match value.as_str() {
                    Some("linear") => Easing::Linear,
                    Some("ease-in") => Easing::EaseIn,
                    Some("ease-out") => Easing::EaseOut,
                    Some("ease-in-out") => Easing::EaseInOut,
                    _ => bail!("easing debe ser linear, ease-in, ease-out o ease-in-out"),
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
        .with_context(|| format!("fotograma clave {}", index + 1))?;
        keyframes.push(keyframe);
        previous = keyframe;
    }
    Ok(keyframes)
}

fn walk(
    node: &NodeRef,
    parent: Option<usize>,
    base: &Path,
    rules: &[Rule],
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
        bail!("<{tag}> requiere navegador/JavaScript y no pertenece al backend nativo");
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
        bail!("elemento HTML no soportado: <{tag}>");
    }
    if attr(node, "data-composition-src").is_some() {
        bail!("data-composition-src aún no soportado");
    }
    let style = style_for(node, rules).with_context(|| format!("estilos de <{tag}>"))?;
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
                attr(node, "id").as_deref().unwrap_or("sin id")
            )
        })?
        .unwrap_or_default();
    let track = attr(node, "data-track-index")
        .map(|s| s.parse::<i32>().context("data-track-index inválido"))
        .transpose()?
        .unwrap_or(0);
    let content = if tag == "img" {
        let src = attr(node, "src").ok_or_else(|| anyhow!("<img> necesita src"))?;
        if src.starts_with("data:") || src.contains("://") {
            bail!("solo imágenes locales son soportadas: {src}");
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
            keyframes,
        },
        start_expr,
        duration,
    });
    if let Some(parent_idx) = parent {
        out[parent_idx].element.children.push(idx);
    }
    for child in node.children() {
        walk(&child, Some(idx), base, rules, out)?;
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
            "ciclo en data-start cerca de {}",
            pending[idx]
                .element
                .id
                .as_deref()
                .unwrap_or("elemento sin id")
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
                .ok_or_else(|| anyhow!("data-start referencia id desconocido: {id}"))?;
            let duration = pending[referenced]
                .duration
                .ok_or_else(|| anyhow!("data-start referencia {id} sin data-duration"))?;
            resolve_start(referenced, pending, ids, memo, visiting)? + duration + offset
        }
    } else if let Some(parent) = pending[idx].element.parent {
        resolve_start(parent, pending, ids, memo, visiting)?
    } else {
        0.0
    };
    if !start.is_finite() || start < 0.0 {
        bail!("data-start resuelto fuera de rango");
    }
    memo[idx] = Some(start);
    visiting[idx] = false;
    Ok(start)
}

pub fn parse_file(path: &Path) -> Result<Composition> {
    let html =
        fs::read_to_string(path).with_context(|| format!("no se pudo leer {}", path.display()))?;
    let document = kuchiki::parse_html().one(html);
    if let Some(unsupported) = document
        .select("script, video, audio, canvas, svg, iframe")
        .map_err(|_| anyhow!("error seleccionando elementos HTML"))?
        .next()
    {
        bail!(
            "<{}> requiere navegador/JavaScript y no pertenece al backend nativo",
            unsupported.name.local
        );
    }
    let metadata = document
        .select_first("[data-composition-id][data-width][data-height]")
        .map_err(|_| anyhow!("falta data-composition-id con data-width y data-height"))?;
    let width: u32 = attr(metadata.as_node(), "data-width")
        .unwrap()
        .parse()
        .context("data-width inválido")?;
    let height: u32 = attr(metadata.as_node(), "data-height")
        .unwrap()
        .parse()
        .context("data-height inválido")?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        bail!("dimensiones fuera de 1..8192");
    }
    let mut rules = Vec::new();
    for node in document
        .select("style")
        .map_err(|_| anyhow!("error seleccionando CSS"))?
    {
        let parsed = parse_css(&node.text_contents(), rules.len())?;
        rules.extend(parsed);
    }
    let root = if metadata
        .as_node()
        .as_element()
        .is_some_and(|e| e.name.local.as_ref() == "meta")
    {
        document
            .select_first("body")
            .map_err(|_| anyhow!("falta <body>"))?
            .as_node()
            .clone()
    } else {
        metadata.as_node().clone()
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut pending = Vec::new();
    walk(&root, None, base, &rules, &mut pending)?;
    if pending.is_empty() {
        bail!("composición vacía");
    }
    let mut ids = HashMap::new();
    for (idx, item) in pending.iter().enumerate() {
        if let Some(id) = &item.element.id {
            if ids.insert(id.clone(), idx).is_some() {
                bail!("id duplicado: {id}");
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
        bail!("duración desconocida: añade data-duration al root o a los clips");
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
                "clip {} excede duración de composición",
                p.element.id.as_deref().unwrap_or("sin id")
            );
        }
        if p.element.fade_in + p.element.fade_out > p.element.end - p.element.start + 1e-9 {
            bail!("fade-in + fade-out excede la duración de un clip");
        }
        if p.element
            .keyframes
            .last()
            .is_some_and(|frame| frame.at > p.element.end - p.element.start + 1e-9)
        {
            bail!(
                "data-hf-keyframes excede la duración de {}",
                p.element.id.as_deref().unwrap_or("elemento sin id")
            );
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
