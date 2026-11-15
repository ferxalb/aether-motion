//! Modo agéntico: `check` estructurado y servidor MCP por stdio.
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
        "usa un array JSON con at en segundos locales, estrictamente ascendente y dentro del clip; revisa el campo indicado"
    } else if message.contains("no se pudo leer imagen") || message.contains("imágenes locales") {
        "comprueba que el PNG, JPEG o WebP exista y que src sea relativo al HTML"
    } else if message.contains("JavaScript") || message.contains("elemento HTML no soportado") {
        "elimina el elemento o reemplázalo por div, texto o img local"
    } else if message.contains("id duplicado") {
        "usa un id único por elemento"
    } else if message.contains("data-start referencia") {
        "apunta data-start a un id existente que tenga data-duration"
    } else if message.contains("duración desconocida") {
        "añade data-duration al root o a cada clip"
    } else if message.contains("data-composition-id") {
        "añade data-composition-id, data-width y data-height al root"
    } else if message.contains("no soportado") || message.contains("solo px") {
        "usa un valor soportado; ver contrato en AGENTS.md"
    } else {
        "revisa el contrato en AGENTS.md"
    }
}

/// Valida la estructura y las cabeceras de imágenes, y dibuja un fotograma de prueba.
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
            .filter(|e| !e.keyframes.is_empty())
            .map(|e| {
                json!({"id": e.id, "tag": e.tag, "start": e.start, "end": e.end,
                "keyframeTimes": e.keyframes.iter().map(|key| key.at).collect::<Vec<_>>()})
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
            return Err(anyhow!("elige size o scale"));
        }
        let size = size
            .as_str()
            .ok_or_else(|| anyhow!("size debe ser 1080p o 4k"))?;
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
    let path = json!({"type": "string", "description": "Ruta a la composición .html"});
    json!([
        {"name": "aether_guide", "description": "Contrato del formato y bucle de trabajo. Léelo antes de escribir una composición.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "aether_write", "description": "Escribe una composición HTML y la valida. No sobrescribe salvo overwrite=true.", "inputSchema": {"type": "object", "properties": {"path": path, "html": {"type": "string"}, "overwrite": {"type": "boolean"}}, "required": ["path", "html"]}},
        {"name": "aether_check", "description": "Valida la composición; devuelve JSON con error y fix si falla.", "inputSchema": {"type": "object", "properties": {"path": path}, "required": ["path"]}},
        {"name": "aether_snapshot", "description": "Dibuja en el segundo exacto t (0 <= t < duración) y devuelve un PNG.", "inputSchema": {"type": "object", "properties": {"path": path, "t": {"type": "number"}, "scale": {"type": "number"}, "size": {"type": "string", "enum": ["1080p", "4k"]}}, "required": ["path"]}},
        {"name": "aether_render", "description": "Renderiza a MP4 (H.264 nativo). No sobrescribe salidas existentes.", "inputSchema": {"type": "object", "properties": {"path": path, "output": {"type": "string"}, "fps": {"type": "integer", "minimum": 1, "maximum": 60}, "threads": {"type": "integer", "minimum": 1, "maximum": 64}, "scale": {"type": "number"}, "size": {"type": "string", "enum": ["1080p", "4k"]}}, "required": ["path", "output"]}}
    ])
}

fn call(name: &str, a: &Value) -> Result<Value> {
    let s = |k: &str| a.get(k).and_then(Value::as_str);
    let path = Path::new(s("path").unwrap_or(""));
    match name {
        "aether_guide" => Ok(text(GUIDE)),
        "aether_write" => {
            if path.extension().and_then(|e| e.to_str()) != Some("html") {
                return Err(anyhow!("la ruta debe terminar en .html"));
            }
            if path.exists() && a.get("overwrite").and_then(Value::as_bool) != Some(true) {
                return Err(anyhow!("{} ya existe; usa overwrite=true", path.display()));
            }
            std::fs::write(path, s("html").ok_or_else(|| anyhow!("falta html"))?)?;
            Ok(text(check(path).to_string()))
        }
        "aether_check" => Ok(text(check(path).to_string())),
        "aether_snapshot" => snapshot(path, a.get("t").and_then(Value::as_f64).unwrap_or(0.0), a),
        "aether_render" => {
            let out = Path::new(s("output").ok_or_else(|| anyhow!("falta output"))?);
            let integer = |name: &str, default| -> Result<u32> {
                match a.get(name) {
                    None => Ok(default),
                    Some(value) => value
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| anyhow!("{name} requiere un entero positivo")),
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
                        "output debe terminar en .mp4; para un fotograma usa aether_snapshot"
                    ))
                }
            }
            Ok(text(format!(
                "{} frames en {:.2}s -> {}",
                r.frame_count(fps),
                t0.elapsed().as_secs_f64(),
                out.display()
            )))
        }
        _ => Err(anyhow!("herramienta desconocida: {name}")),
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
            _ if id.is_some() => Some(Err(format!("método no encontrado: {method}"))),
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
