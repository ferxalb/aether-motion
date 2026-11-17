//! Agent mode: composition checks and the MCP server facade. `zones` holds the safe-zone and caption-zone text checks.
use crate::model::{Composition, Content};
use crate::parse::{parse_file_with, VariableValues};
use crate::render::{check_image, svg, Renderer};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub mod mcp;
pub mod zones;
pub use mcp::serve;

fn fix_hint(message: &str) -> &'static str {
    if message.contains("data-composition-variables")
        || message.contains("data-var-")
        || message.contains("data-variable-values")
        || message.contains("variable values")
        || message.contains("variables file")
        || message.starts_with("variable ")
        || message.contains(": variable ")
    {
        "declare data-composition-variables='[{\"id\":\"title\",\"type\":\"string\",\"label\":\"Title\",\"default\":\"Hello\"}]' on <html> or the composition root (types string, number, color, boolean, enum, image, font); bind with var(--id) in CSS, data-var-text, data-var-src (img, video, audio, source), or data-var-if (id, !id, id=value, id!=value); pass values as a JSON object of declared ids with matching types"
    } else if message.contains("data-hf-reveal") {
        "use data-hf-reveal=from-left|from-right|from-top|from-bottom on a non-root element, with optional data-hf-reveal-at, -duration, and -easing; not with rotate"
    } else if message.contains("data-hf-counter") {
        "use data-hf-counter=<end value> on an empty element, with optional data-hf-counter-from, -duration (0.05 to 10), -delay, -decimals (0 to 6), -prefix, -suffix, -separator, and -easing"
    } else if message.contains("clip-path")
        || message.contains("clipPath")
        || message.contains("mask-image")
        || message.contains("mask shorthand")
    {
        "use clip-path: inset(t r b l [round Npx]) | circle(r [at x y]) | ellipse(rx ry [at x y]) | polygon(x y, ...) with px or %, animate it with clipPath in data-hf-keyframes using the same function, point count, and units, and use mask-image with one linear-gradient() or radial-gradient(); not on the root and not with rotate or mix-blend-mode on the same element"
    } else if message.contains("backdrop-filter") {
        "use backdrop-filter: blur(<px>) or none (at most 200px); no other filter functions are supported"
    } else if message.contains("animated filter") || message.contains("not animatable") {
        "animate filters with blur, brightness, contrast, saturate, grayscale, sepia, hueRotate, or invert in data-hf-keyframes, or with filter in CSS @keyframes, on a non-root element; list each animated function at most once in the static filter; opacity() and drop-shadow() are not animatable"
    } else if message.contains("mix-blend-mode") {
        "use mix-blend-mode: normal|multiply|screen|overlay|darken|lighten|color-dodge|color-burn|hard-light|soft-light|difference|exclusion|plus-lighter on a non-root element that is not inside overflow: hidden, rotation, a reveal, a wipe, or another filter"
    } else if message.contains("filter") || message.contains("drop-shadow") {
        "write filter as [blur(<px>)] [brightness|contrast|saturate|grayscale|sepia|invert|opacity(<n or %>)] [hue-rotate(<angle>)] [drop-shadow(x y [blur] color)], in that order; on the root only color functions (except opacity) are allowed"
    } else if message.contains("data-motion-blur") {
        "set data-motion-blur=\"on\" on the composition root, with optional data-motion-blur-shutter (degrees, above 0 to 720, default 180) and data-motion-blur-samples (2 to 64; automatic when omitted)"
    } else if message.contains("data-hf-vignette") || message.contains("data-hf-grain") {
        "set data-hf-vignette and data-hf-grain on the composition root to a number between 0 and 1"
    } else if message.contains("data-hf-transition") || message.contains("transition ") {
        "on the incoming clip use data-hf-transition=crossfade|push-<dir>|cover-<dir>|wipe-<dir>|whip-<dir>|zoom-in|zoom-out|dip-to-black|dip-to-white|flash|blur|glitch|rgb-split (dir: left, right, up, down) and data-hf-transition-from=<outgoing id>; start it inside the outgoing clip (data-start=\"a-0.6\"), give the outgoing clip a data-duration, and draw it above on a higher data-track-index"
    } else if message.contains("data-hf-split") || message.contains("data-hf-stagger") {
        "use data-hf-split=words|chars on an element with one line of plain text (no <br> or child elements); see AGENTS.md for data-hf-stagger, data-hf-split-from, and the other options"
    } else if message.contains("data-hf-camera") || message.contains("camera keyframe") {
        "use a JSON array of {at, zoom, x, y, easing} on a non-root container; at in local seconds, strictly increasing; zoom between 0.05 and 50"
    } else if message.contains("data-hf-keyframes") {
        "use a JSON array of {at, x, y, opacity, easing} plus optional filter amounts blur (0 to 200 px), brightness, contrast, saturate (0 to 100), grayscale, sepia, invert (0 to 1), and hueRotate (degrees), and the effects glitch (0 to 1) and rgbSplit (-500 to 500 px); at in local seconds, strictly increasing and within the clip; check the indicated field"
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
    } else if message.contains("need sound") {
        "audio effects need sound: remove muted from the <video>, use a source with an AAC audio track, or remove data-hf-volume-ramp, data-hf-eq, and data-hf-duck from it"
    } else if message.contains("data-hf-duck") {
        "on the music clip set data-hf-duck=\"<voice id>[,<id>]\" naming <audio> or unmuted <video> clips with ids; optional data-hf-duck-amount (dB, 0 to 60, default 12), -threshold (dBFS, -90 to 0, default -40), -attack (look-ahead seconds, default 0.15), -hold (default 0.15), and -release (default 0.5), times 0 to 5; no self references or cycles"
    } else if message.contains("data-hf-eq") {
        "use data-hf-eq=voice|music-bed|telephone|warm|bright or a JSON band list such as '[{\"type\":\"highpass\",\"freq\":80},{\"type\":\"peak\",\"freq\":3000,\"gain\":3,\"q\":1}]' (types highpass, lowpass, bandpass, notch, peak, lowshelf, highshelf; freq 10 to 20000 Hz; gain -24 to 24 dB for peak and shelves only; q 0.1 to 30; at most 16 bands)"
    } else if message.contains("data-hf-volume-ramp") {
        "use data-hf-volume-ramp='[{\"at\":0,\"volume\":1},{\"at\":2,\"db\":-12,\"easing\":\"ease-in-out\"}]': at in clip seconds, strictly increasing (0 to 3600); volume 0 to 8 or db up to +18 (not both); omitted values inherit; it multiplies data-volume and the fades"
    } else if message.contains("<video>") {
        "use <video src=\"clip.mp4\" muted data-start=\"0\" data-duration=\"4\" style=\"width:..px;height:..px\"> with a local H.264 MP4 (closed GOPs, not fragmented, not mirrored), optional data-media-start, muted, data-volume, loop (needs data-duration), and data-hf-speed=\"0.5\" (0.05 to 16) or data-hf-speed-ramp='[{\"at\":0,\"speed\":1},{\"at\":1,\"speed\":0.25,\"easing\":\"ease-in-out\"}]' (speed 0 to 16, at strictly increasing); its AAC track plays unless muted; re-encode with `ffmpeg -i in -c:v libx264 -x264-params open-gop=0 -an out.mp4` if needed"
    } else if message.contains("audio")
        || message.contains("data-volume")
        || message.contains("data-hf-trim-start")
        || message.contains("data-hf-loop")
        || message.contains("data-hf-speed")
    {
        "use <audio src=\"file.wav\"> with a local WAV, FLAC, Ogg Vorbis, MP3, or AAC (MP4/M4A) file (mono or stereo), optional data-start, data-duration, data-volume (0 to 8), data-hf-fade-in, data-hf-fade-out, data-hf-trim-start, data-hf-loop (needs data-duration), and data-hf-speed (0.05 to 16) or data-hf-speed-ramp (JSON keyframes with at, speed 0 to 16, easing; a ramp ending at 0 needs data-duration); keep the clip inside the composition"
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

/// Decodes the first and last frame of every `<video>` source (so a codec or container problem
/// shows up here, not mid-render) and reports each one.
fn check_videos(c: &Composition) -> Result<Vec<Value>> {
    let mut seen = HashSet::new();
    let mut videos = Vec::new();
    for element in &c.elements {
        let Content::Video(video) = &element.content else {
            continue;
        };
        let index = &video.index;
        // Each source is decoded once; every clip that plays it is reported (speeds may differ).
        if seen.insert(&video.path) {
            let mut player = crate::video::VideoPlayer::open(&video.path, index.clone())?;
            player.frame(0)?;
            player.frame(index.frame_count() - 1)?;
        }
        let clip_length = element.end - element.start;
        videos.push(json!({
            "id": element.id,
            "src": video.path.display().to_string(),
            "width": index.width,
            "height": index.height,
            "frames": index.frame_count(),
            "keyframes": index.gops.len(),
            "fps": (index.fps() * 1000.0).round() / 1000.0,
            "duration": index.duration,
            "mediaStart": video.media_start,
            "mediaEnd": video.media_time(clip_length),
            "speed": video.speed.as_ref().map(|curve| curve.to_json()),
            "loop": video.looped,
            "rotation": index.rotation,
            "colour": {
                "matrix": if index.colour.matrix == crate::video::colour::Matrix::Bt709 { "bt709" } else { "bt601" },
                "range": if index.colour.full_range { "full" } else { "limited" },
                "tagged": index.colour.tagged,
            },
            "hasAudioTrack": index.has_audio,
            "audioMixed": video.audio,
        }));
    }
    Ok(videos)
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
                "mediaSeconds": clip.media_seconds,
                "speed": clip.speed,
                "effects": clip.effects,
            })
        })
        .collect();
    Ok(json!({"clips": clips, "peak": mix.peak, "clippedSamples": mix.clipped}))
}

/// The declared variables with the values in use, for the `check` report.
fn variables_json(c: &Composition) -> Value {
    use crate::model::VariableKind;
    let typed = |kind: VariableKind, text: &str| match kind {
        VariableKind::Number => text
            .parse::<f64>()
            .map_or_else(|_| json!(text), |n| json!(n)),
        VariableKind::Boolean => json!(text == "true"),
        _ => json!(text),
    };
    let declared: Vec<Value> = c
        .variables
        .declared
        .iter()
        .map(|v| {
            json!({
                "id": v.id,
                "type": v.kind.name(),
                "value": typed(v.kind, &v.value),
                "default": typed(v.kind, &v.default),
                "overridden": v.overridden,
                "uses": {"css": v.uses.css, "text": v.uses.text, "src": v.uses.src, "if": v.uses.condition},
            })
        })
        .collect();
    let unused: Vec<&str> = c
        .variables
        .declared
        .iter()
        .filter(|v| v.uses == crate::model::VariableUses::default())
        .map(|v| v.id.as_str())
        .collect();
    json!({"declared": declared, "unused": unused, "removedElements": c.variables.removed})
}

/// Validates the structure, image headers, video sources, and audio files, then renders a test frame.
pub fn check(path: &Path) -> Value {
    check_with(path, &VariableValues::new())
}

/// `check` with `values` for the composition's declared variables.
pub fn check_with(path: &Path, values: &VariableValues) -> Value {
    let run = || -> Result<Value> {
        let c = parse_file_with(path, values)?;
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
        let videos = check_videos(&c)?;
        let scale = 0.25_f32.max(1.0 / c.width.min(c.height) as f32);
        let mut r = Renderer::new(&c, scale)?;
        r.render_frame(0, 30)?;
        let clips: Vec<Value> = c
            .elements
            .iter()
            .filter(|e| e.parent.is_some() && (e.end - e.start) < c.duration)
            .map(|e| json!({"id": e.id, "tag": e.tag, "start": e.start, "end": e.end, "track": e.track}))
            .collect();
        let blends = c
            .elements
            .iter()
            .filter(|e| e.style.blend != crate::model::BlendMode::Normal)
            .count();
        let filters = c
            .elements
            .iter()
            .filter(|e| !e.style.filter_colors.is_empty() || e.style.drop_shadow.is_some())
            .count();
        let effects = json!({
            "motionBlur": c.motion_blur.map(|m| json!({"shutter": m.shutter, "samples": m.samples.map_or_else(|| json!("auto"), |n| json!(n))})),
            "rootGrade": c.grade.len(),
            "colorFilters": filters,
            "blendModes": blends,
            "animatedFilters": c.elements.iter().filter(|e| !e.motion.filter.is_static()).count(),
        });
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
            json!({"animations": animations, "ok": true, "width": c.width, "height": c.height, "duration": c.duration, "elements": c.elements.len(), "clips": clips, "audio": audio, "videos": videos, "effects": effects, "variables": variables_json(&c)}),
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

#[cfg(test)]
mod tests {
    use super::fix_hint;

    #[test]
    fn audio_effect_errors_get_their_own_hints() {
        let silent = "data-hf-volume-ramp, data-hf-eq, and data-hf-duck need sound: this <video> is muted or has no AAC audio track";
        assert!(fix_hint(silent).starts_with("audio effects need sound"));
        assert!(fix_hint("data-hf-duck references x").contains("data-hf-duck-amount"));
    }
}
