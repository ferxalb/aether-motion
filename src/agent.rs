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
pub use mcp::{run as run_mcp, serve};

fn fix_hint(message: &str) -> &'static str {
    if message.contains("<hf-body> in data-hf-3d")
        || message.contains("<hf-body id=")
        || message.contains("rigid bod")
    {
        "put <hf-body> directly in a data-hf-3d viewport, use type=\"fixed|dynamic\" and shape=\"box\" with size=\"w h d\" or shape=\"sphere\" with radius, and put visible meshes inside the body; use a viewport at most 60 seconds long"
    } else if message.contains("data-hf-3d") || message.contains("<hf-") {
        "put data-hf-3d on a sized <div> clip (optional data-hf-3d-antialias 1 to 4) and fill it with <hf-node>, <hf-body>, <hf-mesh shape=\"plane|box|pyramid|cone|cylinder|sphere|custom\">, <hf-light type=\"ambient|directional\">, and at most one <hf-camera>; place nodes with position, rotation, and scale (or data-hf-keyframes with x, y, z, rotateX, rotateY, rotate, scale; cameras use yaw, pitch, roll, zoom), color meshes with color and material=\"lambert|unlit\", and close every <hf-*> tag explicitly"
    } else if message.contains("2.5D values")
        || message.contains("data-hf-space")
        || message.contains("3D plane values")
        || message.contains("the plane ")
    {
        "put data-hf-space=\"perspective|orthographic\" (optional data-hf-perspective, 50 to 100000 px) on a non-root container; give its planes data-hf-z, data-hf-rotate-x, data-hf-rotate-y, and data-hf-backface=\"hidden\", or keyframe z, rotateX, and rotateY; move its camera with data-hf-camera keyframes or data-hf-shots (x, y, zoom, z, rotateX, rotateY, rotate); planes need a data-hf-space ancestor, and a followed subject needs a camera that does not turn"
    } else if message.contains("data-composition-src") {
        "use an empty scene clip directly inside the composition root with a unique id, data-start, data-track-index, and a relative data-composition-src HTML path; match the canvas size and set data-variable-values to a JSON object of variables declared by that file"
    } else if message.contains("data-composition-variables")
        || message.contains("data-var-")
        || message.contains("data-variable-values")
        || message.contains("variable values")
        || message.contains("variables file")
        || message.starts_with("variable ")
        || message.contains(": variable ")
    {
        "declare data-composition-variables='[{\"id\":\"title\",\"type\":\"string\",\"label\":\"Title\",\"default\":\"Hello\"}]' on <html> or the composition root (types string, number, color, boolean, enum, image, font); bind with var(--id) in CSS, data-var-text, data-var-src (img, video, audio, source), or data-var-if (id, !id, id=value, id!=value); pass values as a JSON object of declared ids with matching types"
    } else if message.contains("inline text")
        || message.contains("inline <")
        || message.contains("background-clip")
    {
        "inline runs (span, strong, b, code inside an element with its own text) may set only color, font-family, font-weight, letter-spacing, -webkit-text-fill-color, -webkit-text-stroke, and background with background-clip: text; move box styles, font-size, and animation to the text element or a separate element; background-clip: text needs a background and text of its own"
    } else if message.contains("data-hf-draw") {
        "put data-hf-draw on an inline <svg> with stroked shapes, with optional data-hf-draw-at, -duration (per shape), -stagger, and -easing; the drawing must end inside the clip"
    } else if message.contains("data-hf-motion-path") {
        "use data-hf-motion-path=\"M0 0 C ...\" (SVG path data in composition px; the element moves by the path's displacement from its first point) with optional -at, -duration, -easing, and -rotate=\"auto\"; it must end inside the clip, and -rotate cannot be combined with clip-path, mask-image, or data-hf-reveal"
    } else if message.contains("data-hf-reveal") {
        "use data-hf-reveal=from-left|from-right|from-top|from-bottom on a non-root element, with optional data-hf-reveal-at, -duration, and -easing; not with rotate"
    } else if message.contains("data-hf-counter") {
        "use data-hf-counter=<end value> on an empty element, with optional data-hf-counter-from, -duration (0.05 to 10), -delay, -decimals (0 to 6), -prefix, -suffix, -separator, and -easing"
    } else if message.contains("clip-path")
        || message.contains("clipPath")
        || message.contains("mask-image")
        || message.contains("mask shorthand")
    {
        "use clip-path: inset(t r b l [round Npx]) | circle(r|closest-side|farthest-side [at x y]) | ellipse(rx ry [at x y]) | polygon([evenodd,] x y, ...) | path([evenodd,] 'M...') with px or %, optionally with a box keyword (margin-box, border-box, padding-box, content-box), or a box keyword alone; animate basic shapes with clipPath in data-hf-keyframes using the same function, point count, units, fill rule, and box; use mask-image with one linear-gradient() or radial-gradient(); not on the root and not with mix-blend-mode on the same element"
    } else if message.contains("backdrop-filter") {
        "use backdrop-filter: blur(<px>) or none (at most 200px); no other filter functions are supported"
    } else if message.contains("animated filter") || message.contains("not animatable") {
        "animate filters with blur, brightness, contrast, saturate, grayscale, sepia, hueRotate, invert, filterOpacity, or dropShadow in data-hf-keyframes, or with filter in CSS @keyframes, on a non-root element; list each animated function at most once in the static filter, and when the static filter uses a custom order, list every animated function in it"
    } else if message.contains("mix-blend-mode") {
        "use mix-blend-mode: normal|multiply|screen|overlay|darken|lighten|color-dodge|color-burn|hard-light|soft-light|difference|exclusion|hue|saturation|color|luminosity|plus-lighter on a non-root element that is not inside overflow: hidden, rotation, a reveal, a wipe, or another filter"
    } else if message.contains("filter") || message.contains("drop-shadow") {
        "write filter as blur(<px>), brightness|contrast|saturate|grayscale|sepia|invert|opacity(<n or %>), hue-rotate(<angle>), and drop-shadow(x y [blur] color) in any order, with blur() and drop-shadow() at most once; on the root only color functions (except opacity) are allowed"
    } else if message.contains("data-hf-motion-blur") {
        "use data-hf-motion-blur=\"on\" on one non-root element (not nested in another blurred element, and not together with data-motion-blur on the root), with optional data-hf-motion-blur-shutter (degrees, above 0 to 720) and data-hf-motion-blur-samples (2 to 64)"
    } else if message.contains("data-hf-bloom") || message.contains("keyframe bloom") {
        "use data-hf-bloom=\"<intensity 0 to 8>\" on a non-root element, with optional data-hf-bloom-threshold (0 to below 1), data-hf-bloom-radius (1 to 200 px), and data-hf-bloom-tint; keyframe bloom animates the intensity and needs data-hf-bloom"
    } else if message.contains("data-motion-blur") {
        "set data-motion-blur=\"on\" on the composition root, with optional data-motion-blur-shutter (degrees, above 0 to 720, default 180) and data-motion-blur-samples (2 to 64; automatic when omitted)"
    } else if message.contains("data-hf-vignette") || message.contains("data-hf-grain") {
        "set data-hf-vignette and data-hf-grain on the composition root to a number between 0 and 1"
    } else if message.contains("data-hf-transition") || message.contains("transition ") {
        "on the incoming clip use data-hf-transition=crossfade|push-<dir>|cover-<dir>|wipe-<dir>|whip-<dir>|zoom-in|zoom-out|dip-to-black|dip-to-white|flash|blur|glitch|rgb-split (dir: left, right, up, down) and data-hf-transition-from=<outgoing id>; start it inside the outgoing clip (data-start=\"a-0.6\"), give the outgoing clip a data-duration, and draw it above on a higher data-track-index"
    } else if message.contains("data-hf-split") || message.contains("data-hf-stagger") {
        "use data-hf-split=words|chars on an element with plain text (no child elements other than <br>); add data-hf-split-wrap to wrap at the element's width; see AGENTS.md for data-hf-stagger, data-hf-split-from, the -x/-y/-scale/-rotate/-opacity start values, -order, -mask, and the -line-* options"
    } else if message.contains("data-hf-shots")
        || message.contains("data-hf-camera-")
        || message.contains("camera rig")
        || message.contains("camera option")
    {
        "on a non-root camera container use data-hf-shots='[{\"preset\":\"follow\",\"duration\":2},{\"preset\":\"dolly-in\",\"duration\":2,\"zoom\":1.3},{\"preset\":\"reveal\"}]' (presets static, follow, pan (needs to: [x, y]), dolly-in, dolly-out, reveal; duration required except on the last shot; fields zoom, to, anchor, follow, easing, blend, and from on the first shot) or data-hf-camera keyframes; follow needs data-hf-camera-subject=\"<id of a descendant>\" with optional -lag and -lookahead (0 to 5 s), -deadzone and -offset (\"x y\" px), -axis (both|x|y), -anchor (\"x y\" fractions), and -easing (smooth or a keyframe easing); the subject's plane cannot have data-hf-parallax 0"
    } else if message.contains("data-hf-camera") || message.contains("camera keyframe") {
        "use a JSON array of {at, zoom, x, y, easing} on a non-root container (easing may be smooth; anchorX, anchorY, and follow need data-hf-camera-subject); at in local seconds, strictly increasing; zoom between 0.05 and 50"
    } else if message.contains("data-hf-keyframes") {
        "use a JSON array of {at, x, y, opacity, easing} plus optional filter amounts blur (0 to 200 px), brightness, contrast, saturate (0 to 100), grayscale, sepia, invert (0 to 1), and hueRotate (degrees), filterOpacity (0 to 1), and dropShadow (a drop-shadow() string), and the effects glitch (0 to 1) and rgbSplit (-500 to 500 px); at in local seconds, strictly increasing and within the clip; check the indicated field"
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
        "use <video src=\"clip.mp4\" muted data-start=\"0\" data-duration=\"4\" style=\"width:..px;height:..px\"> with a local H.264 MP4 (no B-frames, closed GOPs, not fragmented, not mirrored), optional data-media-start, muted, data-volume, loop (needs data-duration), and data-hf-speed=\"0.5\" (0.05 to 16) or data-hf-speed-ramp='[{\"at\":0,\"speed\":1},{\"at\":1,\"speed\":0.25,\"easing\":\"ease-in-out\"}]' (speed 0 to 16, at strictly increasing); its AAC track plays unless muted; re-encode with `ffmpeg -i in.mp4 -c:v libx264 -bf 0 -x264-params open-gop=0 -c:a copy out.mp4` if needed (the error names the exact problem)"
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

/// Native 3D geometry, lighting, camera, optional physics bake, and rendering backend.
fn scenes3d_json(c: &Composition) -> Vec<Value> {
    use crate::scene3d::{NodeKind, Projection, Shading};
    let (backend, fallback) = crate::scene3d::backend::status();
    c.elements
        .iter()
        .filter_map(|e| {
            let Content::Scene3d(scene) = &e.content else {
                return None;
            };
            let mut meshes = Vec::new();
            let mut lights = Vec::new();
            let mut bodies = Vec::new();
            for node in &scene.nodes {
                match &node.kind {
                    NodeKind::Mesh { mesh, material, shape } => meshes.push(json!({
                        "id": node.id, "shape": shape, "triangles": mesh.triangles.len(),
                        "material": match material.shading { Shading::Lambert => "lambert", Shading::Unlit => "unlit" },
                        "cull": match material.cull { crate::scene3d::Cull::Back => "back", crate::scene3d::Cull::None => "none" },
                    })),
                    NodeKind::Light(light) => lights.push(json!({
                        "id": node.id,
                        "type": match light.kind { crate::scene3d::LightKind::Ambient => "ambient", _ => "directional" },
                        "animated": light.intensity.is_animated(),
                    })),
                    NodeKind::Body(body) => bodies.push(json!({
                        "id": node.id,
                        "type": if body.dynamic { "dynamic" } else { "fixed" },
                        "shape": match body.collider {
                            crate::scene3d::physics::Collider::Box(_) => "box",
                            crate::scene3d::physics::Collider::Sphere(_) => "sphere",
                        },
                        "ccd": body.ccd,
                    })),
                    _ => {}
                }
            }
            let (_, camera, index) = scene.camera();
            let projection = match camera.projection {
                Projection::Perspective { focal } => json!({"type": "perspective", "focal": focal}),
                Projection::FieldOfView { degrees } => json!({"type": "perspective", "fov": degrees}),
                Projection::Orthographic => json!({"type": "orthographic"}),
            };
            let mut report = json!({
                "id": e.id, "start": e.start, "end": e.end,
                "antialias": scene.antialias, "nodes": scene.nodes.len(),
                "triangles": scene.triangle_count(), "meshes": meshes, "lights": lights,
                "defaultLights": scene.default_lights,
                "camera": {"declared": index.is_some(), "projection": projection, "near": camera.near, "far": camera.far, "target": camera.target.is_some()},
                "animated": scene.is_animated(),
                "backend": backend, "gpuFallback": fallback,
            });
            if let Some(bake) = &scene.physics {
                report["physics"] = json!({
                    "solver": "rapier",
                    "tickRate": crate::scene3d::physics::TICK_RATE,
                    "pixelsPerMeter": crate::scene3d::physics::PX_PER_METER,
                    "ticks": bake.ticks(),
                    "poseBytes": bake.pose_bytes(),
                    "bodies": bodies,
                });
            }
            Some(report)
        })
        .collect()
}

/// Decodes and mixes the composition's audio without encoding, and reports clipping.
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
    let limiter = mix.limiter.map(|l| {
        json!({"ceilingDb": l.ceiling_db, "maxReductionDb": l.max_reduction_db, "limitedSamples": l.limited_samples})
    });
    Ok(json!({
        "clips": clips,
        "peak": mix.peak,
        "clippedSamples": mix.clipped,
        "limiter": limiter,
        "dither": c.audio_master.dither,
    }))
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
        let cameras: Vec<Value> = c
            .elements
            .iter()
            .filter_map(|e| e.motion.rig.as_ref().map(|rig| (e, rig)))
            .map(|(e, rig)| {
                let subject = rig.subject().map(|s| {
                    json!({"id": c.elements[s.element].id, "depth": s.depth,
                        "lag": s.settings.lag, "lookahead": s.settings.lookahead,
                        "deadzone": s.settings.deadzone, "offset": s.settings.offset,
                        "axis": match s.settings.axis { [true, false] => "x", [false, true] => "y", _ => "both" }})
                });
                let keys: Vec<Value> = rig
                    .keys()
                    .iter()
                    .map(|key| {
                        let pose = rig.pose(key.at);
                        json!({"at": key.at, "zoom": pose.zoom, "focus": pose.focus,
                            "anchor": key.anchor, "follow": key.follow})
                    })
                    .collect();
                json!({"id": e.id, "start": e.start, "end": e.end, "subject": subject, "keys": keys})
            })
            .collect();
        let scenes3d = scenes3d_json(&c);
        Ok(
            json!({"animations": animations, "cameras": cameras, "scenes3d": scenes3d, "ok": true, "width": c.width, "height": c.height, "duration": c.duration, "elements": c.elements.len(), "clips": clips, "audio": audio, "videos": videos, "effects": effects, "variables": variables_json(&c), "dependencies": c.dependencies.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()}),
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
