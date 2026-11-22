//! MCP tool catalog and handlers: tool definitions (with titles and behavior annotations) and the
//! `aether_*` implementations that author, verify, and render compositions. Protocol framing
//! lives in `protocol.rs`; this module only maps tool names and JSON arguments to engine calls.
use crate::encode::{encode_native_with_progress, EncodeOptions, ProgressObserver};
use crate::model::Composition;
use crate::parse::{parse_file_with, VariableValues};
use crate::render::Renderer;
use anyhow::{anyhow, Result};
use image::{ColorType, ImageFormat};
use serde_json::{json, Value};
use std::io::Cursor;
use std::path::Path;

const GUIDE: &str = include_str!("../../../AGENTS.md");

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut output = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let value = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        output.push(B64[(value >> 18) as usize & 63] as char);
        output.push(B64[(value >> 12) as usize & 63] as char);
        output.push(if chunk.len() > 1 {
            B64[(value >> 6) as usize & 63] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            B64[value as usize & 63] as char
        } else {
            '='
        });
    }
    output
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
    let scale = number_arg(args, "scale", 1.0)? as f32;
    Renderer::new(composition, scale)
}

/// An optional numeric argument: `default` when absent or null, an error for any other non-number
/// so a mistyped value never renders silently with the default.
fn number_arg(args: &Value, name: &str, default: f64) -> Result<f64> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_f64()
            .ok_or_else(|| anyhow!("{name} must be a number (got {value})")),
    }
}

/// The optional `variables` argument: values for the composition's declared variables.
fn variables_arg(args: &Value) -> Result<VariableValues> {
    match args.get("variables") {
        None | Some(Value::Null) => Ok(VariableValues::new()),
        Some(Value::Object(values)) => Ok(values.clone()),
        Some(_) => Err(anyhow!(
            "variables must be an object keyed by variable id, such as {{\"title\": \"Q4\"}}"
        )),
    }
}

/// The optional `fps` argument of `aether_snapshot`: validated like the CLI's `--fps`.
fn fps_arg(args: &Value) -> Result<Option<u32>> {
    match args.get("fps") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_u64() {
            Some(fps) => crate::render::validate_fps(fps, "fps").map(Some),
            None => Err(anyhow!(
                "fps must be a whole number from 1 to {} (got {value}); use the --fps of the export, such as 30 or 60",
                crate::render::MAX_FPS
            )),
        },
    }
}

/// `aether_snapshot`: the still at `t` seconds. With `fps`, motion blur uses that frame rate and a
/// time on a frame boundary is the CLI's `render --frame N --fps F` (see `Renderer::render_still`).
fn snapshot(path: &Path, time: f64, args: &Value) -> Result<Value> {
    let fps = fps_arg(args)?;
    let composition = parse_file_with(path, &variables_arg(args)?)?;
    let mut renderer = renderer_for(&composition, args)?;
    let (width, height) = renderer.dimensions();
    let pixels = renderer.render_still(time, fps)?;
    let mut png = Cursor::new(Vec::new());
    image::write_buffer_with_format(
        &mut png,
        pixels,
        width,
        height,
        ColorType::Rgba8,
        ImageFormat::Png,
    )?;
    Ok(
        json!({"content": [{"type": "image", "data": base64(&png.into_inner()), "mimeType": "image/png"}]}),
    )
}

/// Writes `html` to `path` for `aether_write`. Without `overwrite` the file is created
/// exclusively, so an existing file (or a symlink planted at the path) is never replaced, even if
/// it appears between a check and the write. With `overwrite` the text goes to a sibling temporary
/// file that is renamed into place, so a failed write never leaves a truncated composition.
fn write_composition(path: &Path, html: &str, overwrite: bool) -> Result<()> {
    use std::io::Write;
    if !overwrite {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => {
                    anyhow!("{} already exists; use overwrite=true", path.display())
                }
                _ => anyhow!("could not create {}: {error}", path.display()),
            })?;
        return file
            .write_all(html.as_bytes())
            .map_err(|error| anyhow!("could not write {}: {error}", path.display()));
    }
    let partial = path.with_extension(format!("aether-{}.partial.html", std::process::id()));
    // Exclusive create, so a symlink planted at the predictable temporary name is never followed.
    let _ = std::fs::remove_file(&partial);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .and_then(|mut file| file.write_all(html.as_bytes()))
        .and_then(|()| std::fs::rename(&partial, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(anyhow!("could not write {}: {error}", path.display()));
    }
    Ok(())
}

fn text(value: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": value.into()}]})
}

fn definitions() -> Value {
    let path = json!({"type": "string", "description": "Path to the .html composition"});
    let variables = json!({"type": "object", "description": "Values for variables declared in data-composition-variables, keyed by id (for example {\"title\": \"Q4\", \"accent\": \"#ff3366\"}); omitted ids keep their defaults. Unknown ids and values of the wrong type are errors."});
    json!([
        {"name": "aether_guide", "description": "Format contract and work loop. Read it before writing a composition.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "aether_write", "description": "Write an HTML composition and validate it; the response is the same report as aether_check, including layout.findings. Do not overwrite unless overwrite=true.", "inputSchema": {"type": "object", "properties": {"path": path, "html": {"type": "string"}, "overwrite": {"type": "boolean"}, "variables": variables, "safeZone": {"type": "string"}, "captionZone": {"type": "string"}, "samples": {"type": "integer", "minimum": 1, "maximum": 200}, "at": {"type": "array", "items": {"type": "number"}}}, "required": ["path", "html"]}},
        {"name": "aether_block", "description": "List ready-made blocks and themes, or render one as HTML to paste into a composition. Without name it returns the catalog with parameters and defaults.", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "params": {"type": "object", "additionalProperties": {"type": "string"}}}}},
        {"name": "aether_check", "description": "Validate the composition; return JSON with an error and fix on failure. On success it also probes text and returns layout.findings: outside_safe_zone warnings (TikTok, Reels, Shorts on 9:16 frames; 10% margins on other frames) and, with captionZone, caption_zone_collision. Opt out with data-layout-allow-safe-zone or data-layout-allow-caption-zone. Media reports: videos[] (speed, mediaStart, mediaEnd: source seconds at the clip end) and audio.clips[] (start, end, speed, mediaSeconds, effects: volumeRamp, eq with responseDb, duck with regions, duckedSeconds, and deepestDb) plus audio.peak and clippedSamples, so data-hf-speed, data-hf-speed-ramp, data-hf-volume-ramp, data-hf-eq, and data-hf-duck can be verified without rendering.", "inputSchema": {"type": "object", "properties": {"path": path, "variables": variables, "safeZone": {"type": "string", "description": "tiktok, reels, shorts, standard (comma list), or off. Default: all platform zones on 9:16, standard otherwise."}, "captionZone": {"type": "string", "description": "Fractional band, for example x0=0;y0=0.7;x1=1;y1=1 (optional ;severity=error)"}, "samples": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Midpoint sample count (default 9)"}, "at": {"type": "array", "items": {"type": "number"}, "description": "Exact sample times in seconds, instead of samples"}}, "required": ["path"]}},
        {"name": "aether_snapshot", "description": "Render the exact second t (0 <= t < duration) and return a PNG. For data-hf-speed-ramp clips, snapshot before, inside, and after each ramp segment. Pass fps to match an export: motion blur (data-motion-blur, data-hf-motion-blur) then uses that frame rate, and t = N / fps gives the same pixels as `render --frame N --fps F` with the same size or scale.", "inputSchema": {"type": "object", "properties": {"path": path, "t": {"type": "number", "description": "Composition time in seconds (default 0)"}, "fps": {"type": "integer", "minimum": 1, "maximum": 240, "description": "Frame rate for motion-blur shutter windows, validated like the CLI --fps (1 to 240); default 30"}, "variables": variables, "scale": {"type": "number"}, "size": {"type": "string", "enum": ["1080p", "4k"]}}, "required": ["path"]}},
        {"name": "aether_render", "description": "Render to MP4 (native H.264). Does not overwrite existing outputs.", "inputSchema": {"type": "object", "properties": {"path": path, "output": {"type": "string"}, "variables": variables, "fps": {"type": "integer", "minimum": 1, "maximum": 60}, "threads": {"type": "integer", "minimum": 1, "maximum": 64}, "scale": {"type": "number"}, "size": {"type": "string", "enum": ["1080p", "4k"]}, "audioBitrate": {"type": "integer", "minimum": 64, "maximum": 320, "description": "AAC bitrate in kbit/s for compositions with <audio> (default 192)"}, "quality": {"type": "string", "enum": ["draft", "standard", "high", "max"], "description": "H.264 quality preset: fixed quantizer 30, 26, 22, or 18 (default standard); lower quantizers give larger files at about the same CPU cost"}}, "required": ["path", "output"]}}
    ])
}

/// Display title, workflow stage, and behavior hints per tool, in the order an agent uses them.
/// Hints follow MCP tool annotations: none of the tools reaches the network (`openWorldHint`).
struct ToolMeta {
    name: &'static str,
    title: &'static str,
    /// Workflow stage prefixed to the description so the catalog reads as one pipeline.
    stage: &'static str,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
}

const TOOL_META: [ToolMeta; 6] = [
    ToolMeta {
        name: "aether_guide",
        title: "Read the composition guide",
        stage: "[1 Learn]",
        read_only: true,
        destructive: false,
        idempotent: true,
    },
    ToolMeta {
        name: "aether_block",
        title: "Browse and render blocks",
        stage: "[2 Author]",
        read_only: true,
        destructive: false,
        idempotent: true,
    },
    ToolMeta {
        name: "aether_write",
        title: "Write and validate a composition",
        stage: "[2 Author]",
        read_only: false,
        destructive: true,
        idempotent: true,
    },
    ToolMeta {
        name: "aether_check",
        title: "Validate a composition",
        stage: "[3 Verify]",
        read_only: true,
        destructive: false,
        idempotent: true,
    },
    ToolMeta {
        name: "aether_snapshot",
        title: "Render a still frame",
        stage: "[3 Verify]",
        read_only: true,
        destructive: false,
        idempotent: true,
    },
    ToolMeta {
        name: "aether_render",
        title: "Render the MP4",
        stage: "[4 Deliver]",
        read_only: false,
        destructive: false,
        idempotent: false,
    },
];

pub(super) fn is_known_tool(name: &str) -> bool {
    TOOL_META.iter().any(|meta| meta.name == name)
}

/// Names of the tools, in workflow order.
pub(super) fn tool_names() -> impl Iterator<Item = &'static str> {
    TOOL_META.iter().map(|meta| meta.name)
}

/// The `tools/list` payload: each definition plus its title and annotations.
pub(super) fn tools() -> Value {
    let mut list = definitions();
    for tool in list.as_array_mut().into_iter().flatten() {
        let name = tool["name"].as_str().unwrap_or_default().to_string();
        let Some(meta) = TOOL_META.iter().find(|meta| meta.name == name) else {
            continue;
        };
        let description = format!(
            "{} {}",
            meta.stage,
            tool["description"].as_str().unwrap_or("")
        );
        tool["description"] = json!(description);
        tool["title"] = json!(meta.title);
        tool["annotations"] = json!({
            "title": meta.title,
            "readOnlyHint": meta.read_only,
            "destructiveHint": meta.destructive,
            "idempotentHint": meta.idempotent,
            "openWorldHint": false
        });
    }
    list
}

/// Structural check plus the optional zone probe (`safeZone`, `captionZone`, `samples`, `at`),
/// with the optional `variables` values.
fn checked(path: &Path, args: &Value) -> Result<Value> {
    let variables = variables_arg(args)?;
    let mut report = crate::agent::check_with(path, &variables);
    let at = match args.get("at") {
        None => Vec::new(),
        Some(list) => list
            .as_array()
            .and_then(|items| items.iter().map(Value::as_f64).collect::<Option<Vec<_>>>())
            .ok_or_else(|| anyhow!("at must be an array of seconds"))?,
    };
    let samples = match args.get("samples") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| anyhow!("samples must be a positive integer"))? as usize,
        ),
    };
    let request = crate::agent::zones::ZoneRequest {
        variables: Some(&variables),
        safe_zone: args.get("safeZone").and_then(Value::as_str),
        caption_zone: args.get("captionZone").and_then(Value::as_str),
        samples,
        at,
    };
    crate::agent::zones::attach(&mut report, path, &request)?;
    Ok(report)
}

pub(super) fn call(
    name: &str,
    args: &Value,
    progress: Option<&dyn ProgressObserver>,
) -> Result<Value> {
    let string_arg = |key: &str| args.get(key).and_then(Value::as_str);
    let path = Path::new(string_arg("path").unwrap_or(""));
    match name {
        "aether_guide" => Ok(text(GUIDE)),
        "aether_write" => {
            if path.extension().and_then(|extension| extension.to_str()) != Some("html") {
                return Err(anyhow!("the path must end in .html"));
            }
            let html = string_arg("html").ok_or_else(|| anyhow!("missing html"))?;
            let overwrite = args.get("overwrite").and_then(Value::as_bool) == Some(true);
            write_composition(path, html, overwrite)?;
            Ok(text(checked(path, args)?.to_string()))
        }
        "aether_block" => match string_arg("name") {
            None => Ok(text(crate::blocks::list_json()?.to_string())),
            Some(name) => {
                let mut values = std::collections::BTreeMap::new();
                if let Some(params) = args.get("params") {
                    let params = params
                        .as_object()
                        .ok_or_else(|| anyhow!("params must be an object of strings"))?;
                    for (key, value) in params {
                        let value = value
                            .as_str()
                            .map(str::to_string)
                            .or_else(|| value.as_f64().map(|number| number.to_string()))
                            .ok_or_else(|| anyhow!("params.{key} must be a string or number"))?;
                        values.insert(key.clone(), value);
                    }
                }
                Ok(text(crate::blocks::render(name, &values)?))
            }
        },
        "aether_check" => Ok(text(checked(path, args)?.to_string())),
        "aether_snapshot" => snapshot(path, number_arg(args, "t", 0.0)?, args),
        "aether_render" => {
            let output = Path::new(string_arg("output").ok_or_else(|| anyhow!("missing output"))?);
            let integer = |name: &str, default| -> Result<u32> {
                match args.get(name) {
                    None => Ok(default),
                    Some(value) => value
                        .as_u64()
                        .and_then(|number| u32::try_from(number).ok())
                        .ok_or_else(|| anyhow!("{name} must be a positive integer")),
                }
            };
            let fps = integer("fps", 30)?;
            let threads = integer("threads", crate::encode::default_threads())?;
            let audio_bitrate = integer("audioBitrate", crate::audio::DEFAULT_BITRATE / 1000)?
                .checked_mul(1000)
                .ok_or_else(|| anyhow!("audioBitrate must be between 64 and 320"))?;
            let quality = string_arg("quality")
                .map(str::parse)
                .transpose()?
                .unwrap_or_default();
            let variables = variables_arg(args)?;
            crate::encode::lower_priority();
            let composition = parse_file_with(path, &variables)?;
            let mut renderer = renderer_for(&composition, args)?;
            let started = std::time::Instant::now();
            match output.extension().and_then(|extension| extension.to_str()) {
                Some("mp4") => encode_native_with_progress(
                    &mut renderer,
                    output,
                    &EncodeOptions {
                        audio_bitrate,
                        quality,
                        ..EncodeOptions::new(fps, threads)
                    },
                    progress,
                )?,
                _ => {
                    return Err(anyhow!(
                        "output must end in .mp4; use aether_snapshot for a frame"
                    ))
                }
            }
            Ok(text(format!(
                "{} frames in {:.2}s -> {}",
                renderer.frame_count(fps),
                started.elapsed().as_secs_f64(),
                output.display()
            )))
        }
        _ => Err(anyhow!("unknown tool: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_argument_reaches_check_and_rejects_non_objects() {
        let dir = std::env::temp_dir().join(format!("aether-mcp-vars-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v.html");
        std::fs::write(
            &path,
            r#"<div data-composition-id="v" data-width="64" data-height="64" data-duration="1" data-composition-variables='[{"id":"title","type":"string","label":"Title","default":"A"}]'><div data-var-text="title" style="font-size:10px">x</div></div>"#,
        )
        .unwrap();
        let args = json!({"path": path, "variables": {"title": "B"}, "safeZone": "off"});
        let report: Value =
            match call("aether_check", &args, None).unwrap()["content"][0]["text"].as_str() {
                Some(text) => serde_json::from_str(text).unwrap(),
                None => panic!("text content"),
            };
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["variables"]["declared"][0]["value"], "B");
        assert!(variables_arg(&json!({"variables": "title=B"})).is_err());
        let snapshot = call(
            "aether_snapshot",
            &json!({"path": path, "variables": {"title": 1}}),
            None,
        );
        assert!(format!("{:#}", snapshot.unwrap_err()).contains("must be a string"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn numeric_arguments_reject_non_numbers() {
        assert_eq!(number_arg(&json!({}), "t", 0.5).unwrap(), 0.5);
        assert_eq!(number_arg(&json!({"t": null}), "t", 0.5).unwrap(), 0.5);
        assert_eq!(number_arg(&json!({"t": 2}), "t", 0.0).unwrap(), 2.0);
        let error = number_arg(&json!({"t": "2"}), "t", 0.0).unwrap_err();
        assert!(error.to_string().contains("t must be a number"));
    }

    #[test]
    fn write_refuses_existing_files_unless_overwrite() {
        let dir = std::env::temp_dir().join(format!("aether-mcp-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.html");
        write_composition(&path, "a", false).unwrap();
        let error = write_composition(&path, "b", false).unwrap_err();
        assert!(error.to_string().contains("overwrite=true"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a");
        write_composition(&path, "c", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "c");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "no partial file left"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
