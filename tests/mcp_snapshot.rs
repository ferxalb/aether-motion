//! MCP snapshot parity: `aether_snapshot` with `fps` gives the pixels of `aether-motion render
//! --frame N --fps F` for a moving element with its own motion blur, omitting `fps` keeps the 30 fps
//! shutter, and invalid `fps` values are refused with an actionable message. Both sides run the
//! real binary (MCP over stdio, the CLI writing a PNG), and the PNGs are decoded and compared.
mod common;

use common::scratch;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_aether-motion");

/// A static background and a box that moves 300 px per second with `data-hf-motion-blur`.
fn composition(dir: &Path) -> PathBuf {
    let html = dir.join("blur.html");
    std::fs::write(
        &html,
        "<div data-composition-id='m' data-width='160' data-height='90' data-duration='2' style='background:#101010'>\
         <div class='clip' data-start='0' data-duration='2' data-track-index='0' style='left:0;top:60px;width:160px;height:4px;background:#777'></div>\
         <div id='box' class='clip' data-start='0' data-duration='2' data-track-index='1' style='left:0;top:20px;width:20px;height:20px;background:#fff'\
          data-hf-motion-blur='on' data-hf-motion-blur-shutter='360'\
          data-hf-keyframes='[{\"at\":0.2,\"x\":0},{\"at\":0.65,\"x\":135,\"easing\":\"linear\"}]'></div></div>",
    )
    .unwrap();
    html
}

/// Standard base64 (with padding), as `aether_snapshot` encodes its PNG.
fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => panic!("not base64: {c}"),
    };
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0_u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= u32::from(value(c)) << (18 - 6 * i);
        }
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

/// RGBA pixels of a PNG.
fn decode_png(bytes: &[u8]) -> Vec<u8> {
    image::load_from_memory(bytes)
        .unwrap()
        .to_rgba8()
        .into_raw()
}

/// Calls `aether_snapshot` once per argument set through one `aether-motion mcp` process (legacy
/// session) and returns each result object.
fn snapshots(calls: &[Value]) -> Vec<Value> {
    let mut child = Command::new(BIN)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut send = |request: Value| {
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let response: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        response
    };
    send(
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}
        }}),
    );
    let results = calls
        .iter()
        .enumerate()
        .map(|(i, arguments)| {
            let response = send(
                json!({"jsonrpc": "2.0", "id": i + 1, "method": "tools/call",
                "params": {"name": "aether_snapshot", "arguments": arguments}}),
            );
            response["result"].clone()
        })
        .collect();
    drop(stdin);
    child.wait().unwrap();
    results
}

fn image_of(result: &Value) -> Vec<u8> {
    assert_ne!(result["isError"], true, "{result}");
    let content = &result["content"][0];
    assert_eq!(content["mimeType"], "image/png");
    decode_png(&base64_decode(content["data"].as_str().unwrap()))
}

fn error_of(result: &Value) -> String {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap().to_string()
}

/// `aether-motion render <html> --output <png> --frame N --fps F`, decoded.
fn cli_frame(html: &Path, dir: &Path, frame: usize, fps: u32) -> Vec<u8> {
    let out = dir.join(format!("cli-{frame}-{fps}.png"));
    let status = Command::new(BIN)
        .args([
            "render",
            html.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ])
        .args(["--frame", &frame.to_string(), "--fps", &fps.to_string()])
        .args(["--priority", "normal"])
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    decode_png(&std::fs::read(out).unwrap())
}

#[test]
fn snapshots_match_cli_frames_at_the_requested_fps() {
    let dir = scratch("mcp-snapshot");
    let html = composition(&dir);
    let path = html.to_str().unwrap();
    // Frame 27 at 60 fps (t = 0.45) is mid-travel; frame 37 is not a round time in seconds.
    let results = snapshots(&[
        json!({"path": path, "t": 27.0 / 60.0, "fps": 60}),
        json!({"path": path, "t": 37.0 / 60.0, "fps": 60}),
        json!({"path": path, "t": 0.45}),
        json!({"path": path, "t": 0.45, "fps": 30}),
        json!({"path": path, "t": 0.0, "fps": 60}),
    ]);
    let at_60 = image_of(&results[0]);
    assert_eq!(at_60, cli_frame(&html, &dir, 27, 60), "frame 27 at 60 fps");
    assert_eq!(
        image_of(&results[1]),
        cli_frame(&html, &dir, 37, 60),
        "frame 37 at 60 fps"
    );
    // Without fps the snapshot keeps the 30 fps shutter: frame 27/2 = 13.5 is not a frame at 30, so
    // compare t = 0.45 with an explicit fps of 30, which is the CLI's 30 fps shutter window.
    let default = image_of(&results[2]);
    assert_eq!(default, image_of(&results[3]));
    // The 30 fps window is twice as long, so the blur differs from the 60 fps one.
    assert_ne!(default, at_60);
    // The static frame before the motion is the same at any rate.
    assert_eq!(image_of(&results[4]), cli_frame(&html, &dir, 0, 60));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn omitting_fps_matches_cli_frames_at_30_fps() {
    let dir = scratch("mcp-snapshot-30");
    let html = composition(&dir);
    let path = html.to_str().unwrap();
    let results = snapshots(&[
        json!({"path": path, "t": 0.4}),
        json!({"path": path, "t": 0.5}),
    ]);
    assert_eq!(image_of(&results[0]), cli_frame(&html, &dir, 12, 30));
    assert_eq!(image_of(&results[1]), cli_frame(&html, &dir, 15, 30));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_fps_values_are_refused_with_the_cli_range() {
    let dir = scratch("mcp-snapshot-bad");
    let html = composition(&dir);
    let path = html.to_str().unwrap();
    let results = snapshots(&[
        json!({"path": path, "t": 0.5, "fps": 0}),
        json!({"path": path, "t": 0.5, "fps": 241}),
        json!({"path": path, "t": 0.5, "fps": 29.97}),
        json!({"path": path, "t": 0.5, "fps": "60"}),
        json!({"path": path, "t": 0.5, "fps": -30}),
    ]);
    for result in &results {
        let error = error_of(result);
        assert!(
            error.contains("fps must be a whole number from 1 to 240")
                && error.contains("such as 30 or 60"),
            "{error}"
        );
    }
    // The CLI refuses the same values with the same range.
    let output = Command::new(BIN)
        .args([
            "render",
            path,
            "--output",
            dir.join("x.png").to_str().unwrap(),
            "--fps",
            "241",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--fps must be a whole number from 1 to 240"),
        "{stderr}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
