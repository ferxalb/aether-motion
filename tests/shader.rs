//! `data-hf-shader` procedural layers: placement, opacity and edges, reduced resolution, uniforms
//! and keyframes, children drawn on top, determinism across renderers, and parse errors.

mod common;

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;

fn write(name: &str, body: &str) -> std::path::PathBuf {
    let html = format!(
        r#"<div id="root" data-composition-id="x" data-width="100" data-height="80" data-duration="2" style="background:#000000">{body}</div>"#
    );
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    path
}

fn frames(name: &str, body: &str, times: &[f64]) -> Vec<Vec<u8>> {
    let composition = parse_file(&write(name, body)).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    times
        .iter()
        .map(|&t| renderer.render_at(t).unwrap().to_vec())
        .collect()
}

fn error(name: &str, body: &str) -> String {
    format!("{:#}", parse_file(&write(name, body)).unwrap_err())
}

fn pixel(frame: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * 100 + x) * 4;
    [frame[i], frame[i + 1], frame[i + 2]]
}

fn layer(program: &str, extra: &str, style: &str) -> String {
    format!(
        r#"<div class="clip" data-start="0" data-duration="2" data-hf-shader="{program}" data-hf-shader-dither="off" {extra} style="position:absolute; left:10px; top:20px; width:50px; height:40px; {style}"></div>"#
    )
}

#[test]
fn fills_the_padding_box_with_the_program() {
    let f = &frames("sh_fill", &layer("rgb(1, 0.5, 0)", "", ""), &[0.0])[0];
    assert_eq!(pixel(f, 10, 20), [255, 128, 0]);
    assert_eq!(pixel(f, 59, 59), [255, 128, 0]);
    assert_eq!(pixel(f, 9, 20), [0, 0, 0]);
    assert_eq!(pixel(f, 60, 30), [0, 0, 0]);
    assert_eq!(pixel(f, 30, 60), [0, 0, 0]);
}

#[test]
fn uv_spans_the_box_and_alpha_and_opacity_blend() {
    let f = &frames("sh_uv", &layer("vec3(uv.x)", "", ""), &[0.0])[0];
    // The first pixel center sits at uv.x = 0.5 / 50, the last at 49.5 / 50.
    assert_eq!(pixel(f, 10, 30)[0], 3);
    assert_eq!(pixel(f, 59, 30)[0], 252);
    let f = &frames(
        "sh_alpha",
        &layer("rgba(1, 1, 1, 0.5)", "", "opacity: 0.5"),
        &[0.0],
    )[0];
    assert_eq!(pixel(f, 30, 30), [64, 64, 64]);
}

#[test]
fn time_and_keyframed_uniforms_animate() {
    let body = layer(
        "vec3(fract(t * 0.25) * glow)",
        r#"data-hf-shader-uniforms='{"glow": 0}' data-hf-shader-keyframes='[{"at": 0, "glow": 0}, {"at": 1, "glow": 1}]'"#,
        "",
    );
    let shots = frames("sh_time", &body, &[0.0, 1.0, 1.5]);
    assert_eq!(pixel(&shots[0], 30, 30)[0], 0);
    assert_eq!(pixel(&shots[1], 30, 30)[0], 64);
    assert_eq!(pixel(&shots[2], 30, 30)[0], 96);
}

#[test]
fn reduced_resolution_interpolates_smoothly() {
    let full = &frames("sh_full", &layer("vec3(uv.x)", "", ""), &[0.0])[0];
    let half = &frames(
        "sh_half",
        &layer("vec3(uv.x)", r#"data-hf-shader-resolution="0.5""#, ""),
        &[0.0],
    )[0];
    // A linear ramp interpolates exactly, apart from clamping at the box edges.
    for x in 12..58 {
        let (a, b) = (pixel(full, x, 30)[0], pixel(half, x, 30)[0]);
        assert!(a.abs_diff(b) <= 1, "x {x}: {a} vs {b}");
    }
}

#[test]
fn children_draw_over_the_shader() {
    let body = r#"<div class="clip" data-start="0" data-duration="2" data-hf-shader="rgb(0, 0, 1)" style="position:absolute; left:0; top:0; width:100px; height:80px"><div style="position:absolute; left:40px; top:30px; width:20px; height:20px; background:#ff0000"></div></div>"#;
    let f = &frames("sh_child", body, &[0.0])[0];
    assert_eq!(pixel(f, 5, 5), [0, 0, 255]);
    assert_eq!(pixel(f, 50, 40), [255, 0, 0]);
}

#[test]
fn noise_is_identical_across_renderers_and_frames() {
    let program = "let n = fbm(vec3(uv * 6, t), 5); let c = palette(n, #808080, #808080, vec3(1), vec3(0, 0.33, 0.67)); rgba(c * (0.6 + voronoi(px / 9) * 0.6), 0.9)";
    let body = format!(
        r#"<div class="clip" data-start="0" data-duration="2" data-hf-shader="{program}" data-hf-shader-resolution="0.5" style="position:absolute; left:3.5px; top:2.25px; width:90px; height:70px"></div>"#
    );
    let first = frames("sh_det_a", &body, &[0.4, 1.2, 0.4]);
    let second = frames("sh_det_b", &body, &[1.2]);
    assert_eq!(first[1], second[0]);
    // A frame drawn again after another one (kept grid replaced) is unchanged.
    assert_eq!(first[0], first[2]);
    assert_ne!(first[0], first[1]);
}

#[test]
fn rejects_invalid_shaders() {
    for (program, extra, message) in [
        ("uv", "", "2-component"),
        ("rgb(uv.x, 1)", "", "needs 3 components"),
        ("noise(uv", "", "expected ')'"),
        ("rgb(speed)", "", "unknown name 'speed'"),
        ("1", r#"data-hf-shader-resolution="0.3""#, "resolution"),
        ("1", r#"data-hf-shader-size="2""#, "unknown attribute"),
        ("1", r#"data-hf-shader-uniforms='{"uv": 1}'"#, "built-in"),
        (
            "1",
            r#"data-hf-shader-uniforms='{"a": 1}' data-hf-shader-keyframes='[{"at": 0, "b": 1}]'"#,
            "not a declared uniform",
        ),
    ] {
        let body = format!(
            r#"<div class="clip" data-start="0" data-duration="2" data-hf-shader="{program}" {extra} style="width:10px; height:10px"></div>"#
        );
        let error = error("sh_err", &body);
        assert!(error.contains(message), "{program} {extra}: {error}");
    }
    let error = error(
        "sh_text",
        r#"<div class="clip" data-start="0" data-duration="2" data-hf-shader="1">Hello</div>"#,
    );
    assert!(error.contains("own text"), "{error}");
}
