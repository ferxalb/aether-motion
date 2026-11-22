//! MCP protocol layer: JSON-RPC dispatch for the stateless 2026-07-28 revision (`server/discover`,
//! per-request `_meta`) and for legacy sessions that open with `initialize`. Tool behavior lives in
//! `tools.rs`; this module only validates envelopes, negotiates versions, and shapes results.
use super::progress::McpProgress;
use super::tools::{call, is_known_tool, tools};
use serde_json::{json, Value};

pub(super) const PROTOCOL_VERSION: &str = "2026-07-28";
const PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES_META: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO_META: &str = "io.modelcontextprotocol/serverInfo";
const SUPPORTED_LEGACY_VERSIONS: [&str; 4] =
    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Server guidance shown to the agent: the tools as one pipeline, in the order to use them.
pub(super) const INSTRUCTIONS: &str = "AETHER Motion makes deterministic video from HTML compositions. Workflow: 1) aether_guide once, to learn the supported contract; 2) aether_block to pick ready-made blocks, aether_write to save a composition (it validates too); 3) aether_check until ok is true, then aether_snapshot to look at key frames; 4) aether_render to export the MP4. Paths are relative to the server's working directory.";

pub(super) fn rpc_error(code: i32, message: impl Into<String>, data: Option<Value>) -> Value {
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

/// Runs one tool and shapes its outcome as a tool result: errors become `isError` text content, and a
/// panic inside a handler is caught and reported the same way, so one bad composition cannot take
/// the whole stdio server down.
fn run_tool(name: &str, arguments: &Value, params: &Value) -> Value {
    let reporter = McpProgress::from_params(params);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        call(name, arguments, reporter.as_ref().map(|r| r as _))
    }));
    let message = match outcome {
        Ok(Ok(result)) => return result,
        Ok(Err(error)) => format!("{error:#}"),
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            format!("internal error in {name}: {detail}; please report it with the composition")
        }
    };
    json!({"content": [{"type": "text", "text": message}], "isError": true})
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
    let mut result = run_tool(name, &arguments, params);
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
            "instructions": INSTRUCTIONS,
            "ttlMs": 0,
            "cacheScope": "private"
        }),
        "tools/list" => json!({
            "tools": tools(),
            "ttlMs": 0,
            "cacheScope": "private"
        }),
        "tools/call" => return tool_call(params),
        "ping" => json!({}),
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
            Ok(run_tool(name, &arguments, params))
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
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS
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

pub(super) fn process_request(req: &Value, legacy_initialized: &mut bool) -> Option<Value> {
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
    } else if method == "ping" {
        Ok(json!({}))
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
