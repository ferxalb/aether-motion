//! `data-hf-motion-path` (travel along a path) and `data-hf-draw` (self-drawing SVG strokes).

mod common;

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;

fn write(name: &str, body: &str) -> std::path::PathBuf {
    let html = format!(
        r#"<div id="root" data-composition-id="x" data-width="200" data-height="100" data-duration="2" style="background:#000000">{body}</div>"#
    );
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    path
}

fn frame(name: &str, body: &str, time: f64) -> Vec<u8> {
    let composition = parse_file(&write(name, body)).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    renderer.render_at(time).unwrap().to_vec()
}

fn error(name: &str, body: &str) -> String {
    format!("{:#}", parse_file(&write(name, body)).unwrap_err())
}

fn pixel(frame: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * 200 + x) * 4;
    [frame[i], frame[i + 1], frame[i + 2]]
}

const BLACK: [u8; 3] = [0, 0, 0];
const RED: [u8; 3] = [255, 0, 0];

fn mover(extra: &str, size: (u32, u32)) -> String {
    format!(
        r#"<div class="clip" data-start="0" data-duration="2" data-hf-motion-path="M10 10 H110" data-hf-motion-path-duration="1" data-hf-motion-path-easing="linear" {extra} style="position:absolute; left:20px; top:40px; width:{}px; height:{}px; background:#ff0000"></div>"#,
        size.0, size.1
    )
}

#[test]
fn motion_path_offsets_from_the_first_point() {
    let body = mover("", (10, 10));
    // At rest (the path's first point) the box covers x 20..30.
    let start = frame("mp", &body, 0.0);
    assert_eq!(pixel(&start, 25, 45), RED);
    let middle = frame("mp", &body, 0.5);
    assert_eq!(pixel(&middle, 75, 45), RED);
    assert_eq!(pixel(&middle, 25, 45), BLACK);
    assert_eq!(
        pixel(&frame("mp", &body, 1.5), 125, 45),
        RED,
        "the end holds"
    );
}

#[test]
fn motion_path_can_turn_with_the_path() {
    let body = r#"<div class="clip" data-start="0" data-duration="2" data-hf-motion-path="M0 0 V50" data-hf-motion-path-duration="1" data-hf-motion-path-rotate="auto" style="position:absolute; left:80px; top:20px; width:40px; height:6px; background:#ff0000"></div>"#;
    // Moving down turns the bar 90 degrees: it is tall and thin around its center (100, 23).
    let f = frame("mp_rot", body, 0.0);
    assert_eq!(pixel(&f, 100, 8), RED);
    assert_eq!(pixel(&f, 115, 23), BLACK);
}

#[test]
fn strokes_draw_along_their_length_then_match_the_static_svg() {
    let svg = |draw: &str| {
        format!(
            r#"<svg class="clip" data-start="0" data-duration="2" {draw} viewBox="0 0 200 100" style="position:absolute; left:0; top:0; width:200px; height:100px"><path d="M0 50 H200" stroke="red" stroke-width="10" fill="none"/></svg>"#
        )
    };
    let drawn = svg(r#"data-hf-draw data-hf-draw-duration="1" data-hf-draw-easing="linear""#);
    assert_eq!(pixel(&frame("draw", &drawn, 0.0), 100, 50), BLACK);
    let half = frame("draw", &drawn, 0.5);
    assert_eq!(pixel(&half, 50, 50), RED);
    assert_eq!(pixel(&half, 150, 50), BLACK);
    assert_eq!(
        frame("draw", &drawn, 1.5),
        frame("draw_static", &svg(""), 1.5),
        "a finished drawing is the static SVG"
    );
}

#[test]
fn strokes_stagger_in_document_order() {
    let body = r#"<svg class="clip" data-start="0" data-duration="2" data-hf-draw data-hf-draw-duration="0.5" data-hf-draw-stagger="0.5" data-hf-draw-easing="linear" viewBox="0 0 200 100" style="position:absolute; left:0; top:0; width:200px; height:100px">
  <line x1="0" y1="20" x2="200" y2="20" stroke="red" stroke-width="6"/>
  <line x1="0" y1="80" x2="200" y2="80" stroke="red" stroke-width="6"/>
</svg>"#;
    let f = frame("draw_stagger", body, 0.6);
    assert_eq!(pixel(&f, 190, 20), RED, "the first line is complete");
    assert_eq!(pixel(&f, 10, 80), RED, "the second line has started");
    assert_eq!(pixel(&f, 100, 80), BLACK);
}

#[test]
fn rejects_invalid_vector_motion() {
    assert!(error(
        "mp_bad",
        &mover("", (10, 10)).replace("M10 10 H110", "Q nonsense")
    )
    .contains("data-hf-motion-path"));
    assert!(error(
        "draw_div",
        r#"<div class="clip" data-start="0" data-duration="2" data-hf-draw></div>"#
    )
    .contains("inline <svg>"));
    assert!(error(
        "draw_long",
        r#"<svg class="clip" data-start="0" data-duration="2" data-hf-draw data-hf-draw-duration="3" viewBox="0 0 10 10" style="width:10px;height:10px"><path d="M0 0 H10" stroke="red"/></svg>"#
    )
    .contains("after the clip"));
    // A clip on an element the path turns now turns with it.
    parse_file(&write(
        "mp_clip",
        &mover(r#"data-hf-motion-path-rotate="auto""#, (10, 10)).replace(
            "background:#ff0000",
            "background:#ff0000; clip-path: circle(5px)",
        ),
    ))
    .unwrap();
    assert!(error(
        "mp_orphan",
        r#"<div class="clip" data-start="0" data-duration="2" data-hf-draw-at="1"></div>"#
    )
    .contains("requires data-hf-draw"));
}
