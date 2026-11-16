//! Agent mode: composition checks and the MCP server facade.
use crate::model::{Composition, Content};
use crate::parse::parse_file;
use crate::render::{check_image, svg, Renderer};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub mod mcp;
pub use mcp::serve;

fn fix_hint(message: &str) -> &'static str {
    if message.contains("data-hf-reveal") {
        "use data-hf-reveal=from-left|from-right|from-top|from-bottom on a non-root element, with optional data-hf-reveal-at, -duration, and -easing; not with rotate"
    } else if message.contains("data-hf-counter") {
        "use data-hf-counter=<end value> on an empty element, with optional data-hf-counter-from, -duration (0.05 to 10), -delay, -decimals (0 to 6), -prefix, -suffix, -separator, and -easing"
    } else if message.contains("backdrop-filter") {
        "use backdrop-filter: blur(<px>) or none (at most 200px); no other filter functions are supported"
    } else if message.contains("data-hf-vignette") || message.contains("data-hf-grain") {
        "set data-hf-vignette and data-hf-grain on the composition root to a number between 0 and 1"
    } else if message.contains("data-hf-transition") || message.contains("transition ") {
        "on the incoming clip use data-hf-transition=crossfade|push-left|cover-left|wipe-left|zoom-in|zoom-out and data-hf-transition-from=<outgoing id>; start it inside the outgoing clip (data-start=\"a-0.6\"), give the outgoing clip a data-duration, and draw it above on a higher data-track-index"
    } else if message.contains("data-hf-split") || message.contains("data-hf-stagger") {
        "use data-hf-split=words|chars on an element with one line of plain text (no <br> or child elements); see AGENTS.md for data-hf-stagger, data-hf-split-from, and the other options"
    } else if message.contains("data-hf-camera") || message.contains("camera keyframe") {
        "use a JSON array of {at, zoom, x, y, easing} on a non-root container; at in local seconds, strictly increasing; zoom between 0.05 and 50"
    } else if message.contains("data-hf-keyframes") {
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
    } else if message.contains("audio")
        || message.contains("data-volume")
        || message.contains("data-hf-trim-start")
        || message.contains("data-hf-loop")
    {
        "use <audio src=\"file.wav\"> with a local WAV, FLAC, Ogg Vorbis, or MP3 file (mono or stereo), optional data-start, data-duration, data-volume (0 to 8), data-hf-fade-in, data-hf-fade-out, data-hf-trim-start, and data-hf-loop (needs data-duration); keep the clip inside the composition"
    } else if message.contains("<script>") {
        "remove the <script> (GSAP, Date.now, and randomness are not executed so output stays deterministic) and use data-hf-keyframes, data-hf-camera, data-hf-split, data-hf-counter, data-hf-transition, or CSS @keyframes"
    } else if message.contains("invalid SVG")
        || message.contains("<svg>")
        || message.contains("inside <svg>")
    {
        "use a static SVG (shapes, paths, gradients, strokes, clip paths) with a viewBox, as inline <svg> or <img src=\"file.svg\"> with an explicit width and height; no <script>, animation elements, <foreignObject>, text, or external references"
    } else if message.contains("JavaScript") || message.contains("unsupported HTML element") {
        "remove the element or replace it with a div, text, local img (PNG, JPEG, WebP, SVG), or inline <svg>"
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

/// Decodes and mixes the composition's audio (without encoding) and reports each clip, the peak
/// level, and any samples that would clip. Empty when the composition has no `<audio>`.
fn check_audio(c: &Composition) -> Result<Value> {
    if c.audio.is_empty() {
        return Ok(json!({"clips": [], "peak": 0.0, "clippedSamples": 0}));
    }
    let frames = crate::audio::frames_for_video((c.duration * 30.0).ceil() as usize, 30);
    let mix = crate::audio::mix::mix(c, frames)?;
    let clips: Vec<Value> = mix
        .clips
        .iter()
        .map(|clip| {
            let rate = f64::from(crate::audio::SAMPLE_RATE);
            json!({
                "id": clip.id,
                "src": clip.src.display().to_string(),
                "sourceRate": clip.source_rate,
                "sourceChannels": clip.source_channels,
                "sourceSeconds": clip.source_seconds,
                "start": clip.start_frame as f64 / rate,
                "end": (clip.start_frame + clip.frames) as f64 / rate,
            })
        })
        .collect();
    Ok(json!({"clips": clips, "peak": mix.peak, "clippedSamples": mix.clipped}))
}

/// Validates the structure, image headers, and audio files, then renders a test frame.
pub fn check(path: &Path) -> Value {
    let run = || -> Result<Value> {
        let c = parse_file(path)?;
        let mut seen = HashSet::new();
        for element in &c.elements {
            match &element.content {
                Content::Image(path) if seen.insert(path) => {
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
                    {
                        svg::check_svg_file(path)?;
                    } else {
                        check_image(path)?;
                    }
                }
                Content::InlineSvg(markup) => {
                    svg::parse_svg(markup.as_bytes()).context("inline <svg>")?;
                }
                _ => {}
            }
        }
        let audio = check_audio(&c)?;
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
            json!({"animations": animations, "ok": true, "width": c.width, "height": c.height, "duration": c.duration, "elements": c.elements.len(), "clips": clips, "audio": audio}),
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
