//! Agent mode: structured `check` and MCP server over stdio.
use crate::encode::encode_native;
use crate::model::{Composition, Content};
use crate::parse::parse_file;
use crate::render::{check_image, Renderer};
use anyhow::{anyhow, Result};
use image::{ColorType, ImageFormat};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, Cursor, Write};
use std::path::Path;

pub const GUIDE: &str = include_str!("../AGENTS.md");

fn fix_hint(message: &str) -> &'static str {
    if message.contains("data-hf-keyframes") {
        "use a JSON array with at in local seconds, strictly increasing and within the clip; check the indicated field"
    } else if message.contains("CSS variable") || message.contains("var()") {
        "define --name on :root or an ancestor, or add a fallback: var(--name, value)"
    } else if message.contains("animation") || message.contains("@keyframes") {
        "define @keyframes name { from {..} 50% {..} to {..} } using only transform (translate/scale), opacity and animation-timing-function, then use animation: name 1s ease-out [delay] [count] [direction] [fill-mode]"
    } else if message.contains("text-shadow") {
        "use one shadow: text-shadow: offset-x offset-y [blur] color (no spread, inset, or lists)"
    } else if message.contains("line-height")
        || message.contains("white-space")
        || message.contains("max-width")
    {
        "use line-height as a positive number or px, white-space: normal or nowrap, and a non-negative max-width in px or %"
    } else if message.contains("unsupported CSS") || message.contains("unsupported transform") {
        "use a supported CSS value; see the contract in AGENTS.md"
    } else if message.contains("could not read image") || message.contains("local images") {
        "make sure the PNG, JPEG, or WebP file exists and that src is relative to the HTML file"
    } else if message.contains("JavaScript") || message.contains("unsupported HTML element") {
        "remove the element or replace it with a div, text, or local img"
    } else if message.contains("duplicate id") {
        "use a unique id for each element"
    } else if message.contains("data-start references") {
        "point data-start to an existing id with data-duration"
    } else if message.contains("unknown duration") {
        "add data-duration to the root or to each clip"
    } else if message.contains("data-composition-id") {
        "add data-composition-id, data-width, and data-height to the root"
    } else if message.contains("not supported") || message.contains("only px") {
        "use a supported value; see the contract in AGENTS.md"
    } else {
        "see the contract in AGENTS.md"
    }
}

/// Validates the structure and image headers, then renders a test frame.
pub fn check(path: &Path) -> Value {
    let run = || -> Result<Value> {
        let c = parse_file(path)?;
        let mut seen = HashSet::new();
        for element in &c.elements {
            if let Content::Image(path) = &element.content {
                if seen.insert(path) {
                    check_image(path)?;
                }
            }
        }
        let scale = 0.25_f32.max(1.0 / c.width.min(c.height) as f32);
        let mut r = Renderer::new(&c, scale)?;
        r.render_frame(0, 30)?;
        let clips: Vec<Value> = c
            .elements
            .iter()
            .filter(|e| e.parent.is_some() && (e.end - e.start) < c.duration)
            .map(|e| json!({"id": e.id, "tag": e.tag, "start": e.start, "end": e.end, "track": e.track}))
            .collect();
        let animations: Vec<Value> = c
            .elements
            .iter()
            .filter(|e| !e.motion.is_static())
            .map(|e| {
                json!({"id": e.id, "tag": e.tag, "start": e.start, "end": e.end,
                "keyframeTimes": e.motion.times()})
            })
            .collect();
        Ok(
            json!({"animations": animations, "ok": true, "width": c.width, "height": c.height, "duration": c.duration, "elements": c.elements.len(), "clips": clips}),
        )
    };
    match run() {
        Ok(v) => v,
        Err(e) => {
            let m = format!("{e:#}");
            json!({"ok": false, "error": m, "fix": fix_hint(&m)})
        }
    }
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn base64(d: &[u8]) -> String {
    let mut o = String::with_capacity(d.len() * 4 / 3 + 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        o.push(B64[(n >> 18) as usize & 63] as char);
        o.push(B64[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        o.push(if c.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    o
}

fn renderer_for<'a>(composition: &'a Composition, args: &Value) -> Result<Renderer<'a>> {
    if let Some(size) = args.get("size") {
        if args.get("scale").is_some() {
            return Err(anyhow!("choose either size or scale"));
        }
        let size = size
            .as_str()
            .ok_or_else(|| anyhow!("size must be 1080p or 4k"))?;
        return Renderer::with_output_size(composition, size.parse()?);
    }
    let scale = args.get("scale").and_then(Value::as_f64).unwrap_or(1.0) as f32;
    Renderer::new(composition, scale)
}

fn snapshot(path: &Path, t: f64, args: &Value) -> Result<Value> {
    let c = parse_file(path)?;
    let mut r = renderer_for(&c, args)?;
    let (w, h) = r.dimensions();
    let px = r.render_at(t)?;
    let mut png = Cursor::new(Vec::new());
    image::write_buffer_with_format(&mut png, px, w, h, ColorType::Rgba8, ImageFormat::Png)?;
    Ok(
        json!({"content": [{"type": "image", "data": base64(&png.into_inner()), "mimeType": "image/png"}]}),
    )
}

fn text(s: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": s.into()}]})
}

fn tools() -> Value {
    let path = json!({"type": "string", "description": "Path to the .html composition"});
    json!([
        {"name": "aether_guide", "description": "Format contract and work loop. Read it before writing a composition.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "aether_write", "description": "Write an HTML composition and validate it. Do not overwrite unless overwrite=true.", "inputSchema": {"type": "object", "properties": {"path": path, "html": {"type": "string"}, "overwrite": {"type": "boolean"}}, "required": ["path", "html"]}},
        {"name": "aether_check", "description": "Validate the composition; return JSON with an error and fix on failure.", "inputSchema": {"type": "object", "properties": {"path": path}, "required": ["path"]}},
        {"name": "aether_snapshot", "description": "Render the exact second t (0 <= t < duration) and return a PNG.", "inputSchema": {"type": "object", "properties": {"path": path, "t": {"type": "number"}, "scale": {"type": "number"}, "size": {"type": "string", "enum": ["1080p", "4k"]}}, "required": ["path"]}},
        {"name": "aether_render", "description": "Render to MP4 (native H.264). Does not overwrite existing outputs.", "inputSchema": {"type": "object", "properties": {"path": path, "output": {"type": "string"}, "fps": {"type": "integer", "minimum": 1, "maximum": 60}, "threads": {"type": "integer", "minimum": 1, "maximum": 64}, "scale": {"type": "number"}, "size": {"type": "string", "enum": ["1080p", "4k"]}}, "required": ["path", "output"]}}
    ])
}

fn call(name: &str, a: &Value) -> Result<Value> {
    let s = |k: &str| a.get(k).and_then(Value::as_str);
    let path = Path::new(s("path").unwrap_or(""));
    match name {
        "aether_guide" => Ok(text(GUIDE)),
        "aether_write" => {
            if path.extension().and_then(|e| e.to_str()) != Some("html") {
                return Err(anyhow!("the path must end in .html"));
            }
            if path.exists() && a.get("overwrite").and_then(Value::as_bool) != Some(true) {
                return Err(anyhow!(
                    "{} already exists; use overwrite=true",
                    path.display()
                ));
            }
            std::fs::write(path, s("html").ok_or_else(|| anyhow!("missing html"))?)?;
            Ok(text(check(path).to_string()))
        }
        "aether_check" => Ok(text(check(path).to_string())),
        "aether_snapshot" => snapshot(path, a.get("t").and_then(Value::as_f64).unwrap_or(0.0), a),
        "aether_render" => {
            let out = Path::new(s("output").ok_or_else(|| anyhow!("missing output"))?);
            let integer = |name: &str, default| -> Result<u32> {
                match a.get(name) {
                    None => Ok(default),
                    Some(value) => value
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| anyhow!("{name} must be a positive integer")),
                }
            };
            let fps = integer("fps", 30)?;
            let threads = integer("threads", crate::encode::default_threads())?;
            let c = parse_file(path)?;
            let mut r = renderer_for(&c, a)?;
            let t0 = std::time::Instant::now();
            match out.extension().and_then(|e| e.to_str()) {
                Some("mp4") => encode_native(&mut r, out, fps, threads)?,
                _ => {
                    return Err(anyhow!(
                        "output must end in .mp4; use aether_snapshot for a frame"
                    ))
                }
            }
            Ok(text(format!(
                "{} frames in {:.2}s -> {}",
                r.frame_count(fps),
                t0.elapsed().as_secs_f64(),
                out.display()
            )))
        }
        _ => Err(anyhow!("unknown tool: {name}")),
    }
}

pub fn serve() -> Result<()> {
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(req) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        let id = req.get("id").cloned();
        let method = req["method"].as_str().unwrap_or("");
        let result = match method {
            "initialize" => Some(Ok(
                json!({"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "aether", "version": env!("CARGO_PKG_VERSION")}}),
            )),
            "tools/list" => Some(Ok(json!({"tools": tools()}))),
            "tools/call" => {
                let p = &req["params"];
                Some(Ok(
                    match call(p["name"].as_str().unwrap_or(""), &p["arguments"]) {
                        Ok(v) => v,
                        Err(e) => {
                            json!({"isError": true, "content": [{"type": "text", "text": format!("{e:#}")}]})
                        }
                    },
                ))
            }
            "ping" => Some(Ok(json!({}))),
            _ if id.is_some() => Some(Err(format!("method not found: {method}"))),
            _ => None,
        };
        if let (Some(id), Some(r)) = (id, result) {
            let msg = match r {
                Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
                Err(m) => {
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": m}})
                }
            };
            writeln!(out, "{msg}")?;
            out.flush()?;
        }
    }
    Ok(())
}
