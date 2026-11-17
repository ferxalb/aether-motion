//! `clip-path` shapes (static and keyframed) and `mask-image` gradients.

mod common;

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;

fn write(name: &str, body: &str) -> std::path::PathBuf {
    let html = format!(
        r#"<div id="root" data-composition-id="x" data-width="100" data-height="100" data-duration="2" style="background:#0000ff">{body}</div>"#
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
    let i = (y * 100 + x) * 4;
    [frame[i], frame[i + 1], frame[i + 2]]
}

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn red_box(style: &str, extra: &str) -> String {
    format!(
        r#"<div class="clip" data-start="0" data-duration="2" {extra} style="position:absolute; left:0; top:0; width:100px; height:100px; background:#ff0000; {style}"></div>"#
    )
}

#[test]
fn circle_and_inset_cut_the_element() {
    let f = frame(
        "cp_circle",
        &red_box("clip-path: circle(30px at 50% 50%)", ""),
        0.0,
    );
    assert_eq!(pixel(&f, 50, 50), RED);
    assert_eq!(pixel(&f, 50, 22), RED);
    assert_eq!(pixel(&f, 50, 18), BLUE);
    assert_eq!(pixel(&f, 5, 5), BLUE);

    let f = frame(
        "cp_inset",
        &red_box("clip-path: inset(10px 20% 30px 0)", ""),
        0.0,
    );
    assert_eq!(pixel(&f, 0, 10), RED);
    assert_eq!(pixel(&f, 79, 69), RED);
    assert_eq!(pixel(&f, 80, 50), BLUE);
    assert_eq!(pixel(&f, 40, 9), BLUE);
    assert_eq!(pixel(&f, 40, 70), BLUE);
}

#[test]
fn polygon_edges_are_antialiased() {
    let f = frame(
        "cp_poly",
        &red_box("clip-path: polygon(0 0, 100% 0, 0 100%)", ""),
        0.0,
    );
    assert_eq!(pixel(&f, 10, 10), RED);
    assert_eq!(pixel(&f, 90, 90), BLUE);
    // A pixel the diagonal cuts in half mixes the two colors.
    let edge = pixel(&f, 49, 50);
    assert!(edge[0] > 60 && edge[0] < 200 && edge[2] > 60, "{edge:?}");
}

#[test]
fn keyframed_clip_path_animates_and_holds() {
    let body = red_box(
        "",
        r#"data-hf-keyframes='[{"at":0,"clipPath":"circle(0% at 50% 50%)"},{"at":1,"clipPath":"circle(75% at 50% 50%)"}]'"#,
    );
    assert_eq!(pixel(&frame("cp_anim", &body, 0.0), 50, 50), BLUE);
    let middle = frame("cp_anim", &body, 0.5);
    assert_eq!(pixel(&middle, 50, 50), RED);
    assert_eq!(pixel(&middle, 2, 2), BLUE);
    assert_eq!(pixel(&frame("cp_anim", &body, 1.5), 2, 2), RED);

    // Consecutive frames must not be reused while the clip moves.
    let composition = parse_file(&write("cp_seq", &body)).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let a = renderer.render_frame(10, 30).unwrap().to_vec();
    let b = renderer.render_frame(11, 30).unwrap().to_vec();
    assert_ne!(a, b);
}

#[test]
fn clip_applies_after_the_filter() {
    // The blur would spread red past the circle; the clip cuts it.
    let f = frame(
        "cp_blur",
        &red_box("clip-path: circle(20px at 50% 50%); filter: blur(6px)", ""),
        0.0,
    );
    assert_eq!(pixel(&f, 50, 75), BLUE);
    assert_eq!(pixel(&f, 50, 50), RED);
}

#[test]
fn mask_image_fades_with_the_gradient_alpha() {
    let f = frame(
        "mask",
        &red_box(
            "mask-image: linear-gradient(to bottom, #000000, transparent)",
            "",
        ),
        0.0,
    );
    let top = pixel(&f, 50, 0);
    let middle = pixel(&f, 50, 50);
    let bottom = pixel(&f, 50, 99);
    assert!(top[0] >= 250 && top[2] <= 5, "{top:?}");
    assert!((120..=135).contains(&middle[0]), "{middle:?}");
    assert!(bottom[0] <= 5 && bottom[2] >= 250, "{bottom:?}");
}

#[test]
fn rejects_unsupported_combinations() {
    assert!(error(
        "cp_rot",
        &red_box("clip-path: circle(10px); transform: rotate(10deg)", "")
    )
    .contains("rotate"));
    assert!(error(
        "cp_blend",
        &red_box("clip-path: circle(10px); mix-blend-mode: screen", "")
    )
    .contains("mix-blend-mode"));
    assert!(error(
        "cp_units",
        &red_box(
            "",
            r#"data-hf-keyframes='[{"at":0,"clipPath":"circle(0px)"},{"at":1,"clipPath":"circle(50%)"}]'"#
        )
    )
    .contains("same unit"));
    assert!(error("cp_path", &red_box("clip-path: path('M0 0 L10 10')", "")).contains("path()"));
    assert!(error("mask_url", &red_box("mask-image: url(a.png)", "")).contains("url()"));
}
