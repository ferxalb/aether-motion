//! MCP stdio server for the current stateless protocol and legacy clients. A `tools/call` whose
//! `_meta` carries a `progressToken` receives `notifications/progress` while `aether_render` encodes.
use crate::encode::{encode_native_with_progress, EncodeOptions, ProgressObserver, ProgressUpdate};
use crate::model::Composition;
use crate::parse::{parse_file_with, VariableValues};
use crate::render::Renderer;
use anyhow::{anyhow, Result};
use image::{ColorType, ImageFormat};
use serde_json::{json, Value};
use std::io::{BufRead, Cursor, Write};
use std::path::Path;

const PROTOCOL_VERSION: &str = "2026-07-28";
const PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES_META: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO_META: &str = "io.modelcontextprotocol/serverInfo";
const SUPPORTED_LEGACY_VERSIONS: [&str; 4] =
    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const GUIDE: &str = include_str!("../../AGENTS.md");

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
    let scale = args.get("scale").and_then(Value::as_f64).unwrap_or(1.0) as f32;
    Renderer::new(composition, scale)
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

fn snapshot(path: &Path, time: f64, args: &Value) -> Result<Value> {
    let composition = parse_file_with(path, &variables_arg(args)?)?;
    let mut renderer = renderer_for(&composition, args)?;
    let (width, height) = renderer.dimensions();
    let pixels = renderer.render_at(time)?;
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

fn text(value: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": value.into()}]})
}

fn tools() -> Value {
    let path = json!({"type": "string", "description": "Path to the .html composition"});
    let variables = json!({"type": "object", "description": "Values for variables declared in data-composition-variables, keyed by id (for example {\"title\": \"Q4\", \"accent\": \"#ff3366\"}); omitted ids keep their defaults. Unknown ids and values of the wrong type are errors."});
    json!([
        {"name": "aether_guide", "description": "Format contract and work loop. Read it before writing a composition.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "aether_write", "description": "Write an HTML composition and validate it; the response is the same report as aether_check, including layout.findings. Do not overwrite unless overwrite=true.", "inputSchema": {"type": "object", "properties": {"path": path, "html": {"type": "string"}, "overwrite": {"type": "boolean"}, "variables": variables, "safeZone": {"type": "string"}, "captionZone": {"type": "string"}, "samples": {"type": "integer", "minimum": 1, "maximum": 200}, "at": {"type": "array", "items": {"type": "number"}}}, "required": ["path", "html"]}},
        {"name": "aether_block", "description": "List ready-made blocks and themes, or render one as HTML to paste into a composition. Without name it returns the catalog with parameters and defaults.", "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "params": {"type": "object", "additionalProperties": {"type": "string"}}}}},
        {"name": "aether_check", "description": "Validate the composition; return JSON with an error and fix on failure. On success it also probes text and returns layout.findings: outside_safe_zone warnings (TikTok, Reels, Shorts on 9:16 frames; 10% margins on other frames) and, with captionZone, caption_zone_collision. Opt out with data-layout-allow-safe-zone or data-layout-allow-caption-zone. Media reports: videos[] (speed, mediaStart, mediaEnd: source seconds at the clip end) and audio.clips[] (start, end, speed, mediaSeconds, effects: volumeRamp, eq with responseDb, duck with regions, duckedSeconds, and deepestDb) plus audio.peak and clippedSamples, so data-hf-speed, data-hf-speed-ramp, data-hf-volume-ramp, data-hf-eq, and data-hf-duck can be verified without rendering.", "inputSchema": {"type": "object", "properties": {"path": path, "variables": variables, "safeZone": {"type": "string", "description": "tiktok, reels, shorts, standard (comma list), or off. Default: all platform zones on 9:16, standard otherwise."}, "captionZone": {"type": "string", "description": "Fractional band, for example x0=0;y0=0.7;x1=1;y1=1 (optional ;severity=error)"}, "samples": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Midpoint sample count (default 9)"}, "at": {"type": "array", "items": {"type": "number"}, "description": "Exact sample times in seconds, instead of samples"}}, "required": ["path"]}},
        {"name": "aether_snapshot", "description": "Render the exact second t (0 <= t < duration) and return a PNG. For data-hf-speed-ramp clips, snapshot before, inside, and after each ramp segment.", "inputSchema": {"type": "object", "properties": {"path": path, "t": {"type": "number"}, "variables": variables, "scale": {"type": "number"}, "size": {"type": "string", "enum": ["1080p", "4k"]}}, "required": ["path"]}},
        {"name": "aether_render", "description": "Render to MP4 (native H.264). Does not overwrite existing outputs.", "inputSchema": {"type": "object", "properties": {"path": path, "output": {"type": "string"}, "variables": variables, "fps": {"type": "integer", "minimum": 1, "maximum": 60}, "threads": {"type": "integer", "minimum": 1, "maximum": 64}, "scale": {"type": "number"}, "size": {"type": "string", "enum": ["1080p", "4k"]}, "audioBitrate": {"type": "integer", "minimum": 64, "maximum": 320, "description": "AAC bitrate in kbit/s for compositions with <audio> (default 192)"}, "quality": {"type": "string", "enum": ["draft", "standard", "high", "max"], "description": "H.264 quality preset: fixed quantizer 30, 26, 22, or 18 (default standard); lower quantizers give larger files at about the same CPU cost"}}, "required": ["path", "output"]}}
    ])
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

fn call(name: &str, args: &Value, progress: Option<&dyn ProgressObserver>) -> Result<Value> {
    let string_arg = |key: &str| args.get(key).and_then(Value::as_str);
    let path = Path::new(string_arg("path").unwrap_or(""));
    match name {
        "aether_guide" => Ok(text(GUIDE)),
        "aether_write" => {
            if path.extension().and_then(|extension| extension.to_str()) != Some("html") {
                return Err(anyhow!("the path must end in .html"));
            }
            if path.exists() && args.get("overwrite").and_then(Value::as_bool) != Some(true) {
                return Err(anyhow!(
                    "{} already exists; use overwrite=true",
                    path.display()
                ));
            }
            std::fs::write(
                path,
                string_arg("html").ok_or_else(|| anyhow!("missing html"))?,
            )?;
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
        "aether_snapshot" => snapshot(
            path,
            args.get("t").and_then(Value::as_f64).unwrap_or(0.0),
            args,
        ),
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
            let audio_bitrate = integer("audioBitrate", crate::audio::DEFAULT_BITRATE / 1000)?;
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
                        audio_bitrate: audio_bitrate * 1000,
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

/// Sends `notifications/progress` for a tool call whose request carried `_meta.progressToken`
/// (a string or an integer), written to stdout between the request and its response.
struct McpProgress {
    token: Value,
}

impl McpProgress {
    fn from_params(params: &Value) -> Option<Self> {
        let token = params.get("_meta")?.get("progressToken")?;
        (token.is_string() || token.is_i64() || token.is_u64()).then(|| Self {
            token: token.clone(),
        })
    }

    fn notification(&self, update: &ProgressUpdate) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {
                "progressToken": self.token,
                "progress": update.done,
                "total": update.total,
                "message": update.summary().trim_start(),
            }
        })
    }
}

impl ProgressObserver for McpProgress {
    fn update(&self, update: ProgressUpdate) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{}", self.notification(&update));
        let _ = stdout.flush();
    }
}

fn is_known_tool(name: &str) -> bool {
    matches!(
        name,
        "aether_guide"
            | "aether_write"
            | "aether_block"
            | "aether_check"
            | "aether_snapshot"
            | "aether_render"
    )
}

fn rpc_error(code: i32, message: impl Into<String>, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message.into()});
    if let Some(data) = data {
        error["data"] = data;
    }
    error
}

fn invalid_params(message: impl Into<String>) -> Value {
    rpc_error(-32602, message, None)
}

fn server_info() -> Value {
    json!({"name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION")})
}

fn complete_result(mut result: Value) -> Value {
    result["resultType"] = json!("complete");
    result["_meta"] = json!({});
    result["_meta"][SERVER_INFO_META] = server_info();
    result
}

fn tool_call(params: &Value) -> std::result::Result<Value, Value> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err(invalid_params("tools/call requires a string name"));
    };
    if !is_known_tool(name) {
        return Err(invalid_params(format!("Unknown tool: {name}")));
    }
    let arguments = match params.get("arguments") {
        None => json!({}),
        Some(Value::Object(_)) => params["arguments"].clone(),
        Some(_) => return Err(invalid_params("tools/call arguments must be an object")),
    };
    let reporter = McpProgress::from_params(params);
    let mut result = match call(name, &arguments, reporter.as_ref().map(|r| r as _)) {
        Ok(result) => result,
        Err(error) => {
            json!({"content": [{"type": "text", "text": format!("{error:#}")}], "isError": true})
        }
    };
    if result.get("isError").is_none() {
        result["isError"] = json!(false);
    }
    Ok(complete_result(result))
}

fn modern_dispatch(method: &str, params: &Value) -> std::result::Result<Value, Value> {
    let result = match method {
        "server/discover" => json!({
            "supportedVersions": [PROTOCOL_VERSION],
            "capabilities": {"tools": {}},
            "instructions": "Use aether_guide to read the composition contract and workflow before creating a composition.",
            "ttlMs": 0,
            "cacheScope": "private"
        }),
        "tools/list" => json!({
            "tools": tools(),
            "ttlMs": 0,
            "cacheScope": "private"
        }),
        "tools/call" => return tool_call(params),
        _ => {
            return Err(rpc_error(
                -32601,
                format!("method not found: {method}"),
                None,
            ))
        }
    };
    Ok(complete_result(result))
}

fn legacy_dispatch(method: &str, params: &Value) -> std::result::Result<Value, Value> {
    match method {
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Err(invalid_params("tools/call requires a string name"));
            };
            if !is_known_tool(name) {
                return Err(invalid_params(format!("Unknown tool: {name}")));
            }
            let arguments = match params.get("arguments") {
                None => json!({}),
                Some(Value::Object(_)) => params["arguments"].clone(),
                Some(_) => return Err(invalid_params("tools/call arguments must be an object")),
            };
            let reporter = McpProgress::from_params(params);
            match call(name, &arguments, reporter.as_ref().map(|r| r as _)) {
                Ok(result) => Ok(result),
                Err(error) => Ok(
                    json!({"content": [{"type": "text", "text": format!("{error:#}")}], "isError": true}),
                ),
            }
        }
        "ping" => Ok(json!({})),
        _ => Err(rpc_error(
            -32601,
            format!("method not found: {method}"),
            None,
        )),
    }
}

fn initialize(req: &Value, legacy_initialized: &mut bool) -> std::result::Result<Value, Value> {
    let params = match req.get("params") {
        Some(Value::Object(params)) => params,
        _ => return Err(invalid_params("initialize requires object params")),
    };
    let Some(requested_version) = params.get("protocolVersion").and_then(Value::as_str) else {
        return Err(invalid_params(
            "initialize requires a protocolVersion string",
        ));
    };
    if !params.get("capabilities").is_some_and(Value::is_object)
        || !params.get("clientInfo").is_some_and(Value::is_object)
    {
        return Err(invalid_params(
            "initialize requires object capabilities and clientInfo",
        ));
    }
    *legacy_initialized = true;
    let version = if SUPPORTED_LEGACY_VERSIONS.contains(&requested_version) {
        requested_version
    } else {
        SUPPORTED_LEGACY_VERSIONS[0]
    };
    Ok(json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": server_info()
    }))
}

fn modern_version_error(requested: &str) -> Value {
    rpc_error(
        -32022,
        "Unsupported protocol version",
        Some(json!({"supported": [PROTOCOL_VERSION], "requested": requested})),
    )
}

fn request_params(req: &Value) -> std::result::Result<Value, Value> {
    match req.get("params") {
        None => Ok(json!({})),
        Some(Value::Object(_)) => Ok(req["params"].clone()),
        Some(_) => Err(invalid_params("request params must be an object")),
    }
}

fn process_request(req: &Value, legacy_initialized: &mut bool) -> Option<Value> {
    let id = req.get("id")?;
    if !(id.is_string() || id.is_number()) {
        return None;
    }
    let id = id.clone();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = match request_params(req) {
        Ok(params) => params,
        Err(error) => return Some(json!({"jsonrpc": "2.0", "id": id, "error": error})),
    };
    let meta = match params.get("_meta") {
        None => None,
        Some(Value::Object(meta)) => Some(meta),
        Some(_) => {
            let error = invalid_params("request _meta must be an object");
            return Some(json!({"jsonrpc": "2.0", "id": id, "error": error}));
        }
    };
    let is_modern = meta.is_some_and(|meta| {
        meta.contains_key(PROTOCOL_VERSION_META) || meta.contains_key(CLIENT_CAPABILITIES_META)
    });

    let result = if method == "initialize" && !is_modern {
        initialize(req, legacy_initialized)
    } else if is_modern {
        let meta = meta.expect("modern requests have metadata");
        let Some(version) = meta.get(PROTOCOL_VERSION_META).and_then(Value::as_str) else {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": invalid_params("request metadata requires a protocol version")
            }));
        };
        if !meta
            .get(CLIENT_CAPABILITIES_META)
            .is_some_and(Value::is_object)
        {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": invalid_params("request metadata requires object client capabilities")
            }));
        }
        if version != PROTOCOL_VERSION {
            Err(modern_version_error(version))
        } else {
            modern_dispatch(method, &params)
        }
    } else if *legacy_initialized {
        legacy_dispatch(method, &params)
    } else {
        Err(invalid_params(
            "send server/discover with per-request metadata or initialize a legacy session first",
        ))
    };

    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    })
}

/// Serves newline-delimited MCP JSON-RPC messages over stdin and stdout.
pub fn serve() -> Result<()> {
    let mut output = std::io::stdout();
    let mut legacy_initialized = false;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(response) = process_request(&request, &mut legacy_initialized) {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
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
    fn progress_tokens_become_progress_notifications() {
        assert!(McpProgress::from_params(&json!({"name": "aether_render"})).is_none());
        assert!(McpProgress::from_params(&json!({"_meta": {"progressToken": 1.5}})).is_none());
        let reporter =
            McpProgress::from_params(&json!({"_meta": {"progressToken": "r1"}})).unwrap();
        let note = reporter.notification(&ProgressUpdate {
            done: 30,
            total: 60,
            elapsed: std::time::Duration::from_secs(1),
        });
        assert_eq!(note["method"], "notifications/progress");
        assert_eq!(note["params"]["progressToken"], "r1");
        assert_eq!(note["params"]["progress"], 30);
        assert_eq!(note["params"]["total"], 60);
        assert_eq!(
            note["params"]["message"],
            "50% 30/60 frames, 30.0 fps, ETA 1.0 s"
        );
    }
}
