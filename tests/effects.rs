//! `filter` color functions and `drop-shadow`, root grading, `mix-blend-mode`, and motion blur.

mod common;

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;

/// Renders a 100x100 composition with a red background at `time` and returns its RGB pixel at (x, y)
/// for every requested point.
fn pixels(
    name: &str,
    root: &str,
    body: &str,
    time: f64,
    points: &[(usize, usize)],
) -> Vec<[u8; 3]> {
    let html = format!(
        r#"<div id="root" data-composition-id="x" data-width="100" data-height="100" data-duration="2" {root}>{body}</div>"#
    );
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let frame = renderer.render_at(time).unwrap().to_vec();
    points
        .iter()
        .map(|&(x, y)| {
            let i = (y * 100 + x) * 4;
            [frame[i], frame[i + 1], frame[i + 2]]
        })
        .collect()
}

fn error(name: &str, root: &str, body: &str) -> String {
    let html = format!(
        r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="2" {root}>{body}</div>"#
    );
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    format!("{:#}", parse_file(&path).unwrap_err())
}

const BOX: &str = r#"class="clip" data-start="0" data-duration="2""#;

#[test]
fn color_functions_recolor_the_element_only() {
    let body = format!(
        r#"<div {BOX} style="position:absolute; left:20px; top:20px; width:60px; height:60px; background:#ff0000; filter: grayscale(1)"></div>"#
    );
    let px = pixels(
        "fx_gray",
        r#"style="background:#0000ff""#,
        &body,
        0.0,
        &[(50, 50), (5, 5)],
    );
    // Rec. 709 luma of pure red, rounded: 0.2126 * 255.
    assert_eq!(px[0], [54, 54, 54]);
    assert_eq!(px[1], [0, 0, 255], "the backdrop is untouched");
}

#[test]
fn root_filter_grades_the_whole_frame() {
    let body = format!(
        r#"<div {BOX} style="position:absolute; left:0; top:0; width:50px; height:100px; background:#ffffff"></div>"#
    );
    let px = pixels(
        "fx_root",
        r#"style="background:#000000; filter: invert(1)""#,
        &body,
        0.0,
        &[(10, 10), (90, 10)],
    );
    assert_eq!(px, vec![[0, 0, 0], [255, 255, 255]]);
    let message = error("fx_root_opacity", r#"style="filter: opacity(0.5)""#, "");
    assert!(message.contains("opacity()"), "{message}");
}

#[test]
fn drop_shadow_follows_the_painted_shape() {
    let body = format!(
        r#"<div {BOX} style="position:absolute; left:20px; top:20px; width:40px; height:40px; background:#ffffff; filter: drop-shadow(10px 10px 0 #000000)"></div>"#
    );
    let px = pixels(
        "fx_drop",
        r#"style="background:#808080""#,
        &body,
        0.0,
        &[(40, 40), (65, 65), (65, 25), (10, 10)],
    );
    assert_eq!(px[0], [255, 255, 255], "the element stays on top");
    assert_eq!(px[1], [0, 0, 0], "the hard shadow is offset by 10 px");
    assert_eq!(px[2], [128, 128, 128], "no shadow above the offset box");
    assert_eq!(px[3], [128, 128, 128]);
}

#[test]
fn blend_modes_mix_with_the_backdrop() {
    let layer = |mode: &str| {
        format!(
            r#"<div {BOX} style="position:absolute; left:0; top:0; width:100px; height:100px; background:#808080; mix-blend-mode:{mode}"></div>"#
        )
    };
    let root = r#"style="background:#ff8000""#;
    let multiply = pixels("fx_mul", root, &layer("multiply"), 0.0, &[(50, 50)]);
    let screen = pixels("fx_scr", root, &layer("screen"), 0.0, &[(50, 50)]);
    let difference = pixels("fx_diff", root, &layer("difference"), 0.0, &[(50, 50)]);
    assert_eq!(multiply[0], [128, 64, 0]);
    assert_eq!(screen[0], [255, 192, 128]);
    assert_eq!(difference[0], [127, 0, 128]);
}

#[test]
fn blend_inside_an_isolating_ancestor_is_rejected() {
    let body = format!(
        r#"<div id="card" {BOX} style="position:absolute; width:50px; height:50px; overflow:hidden"><div {BOX} style="mix-blend-mode:multiply"></div></div>"#
    );
    let message = error("fx_blend_iso", "", &body);
    assert!(message.contains("#card"), "{message}");
    let message = error("fx_blend_root", r#"style="mix-blend-mode: screen""#, "");
    assert!(message.contains("root"), "{message}");
    let message = error(
        "fx_blend_hue",
        "",
        &format!(r#"<div {BOX} style="mix-blend-mode:hue"></div>"#),
    );
    assert!(message.contains("hue"), "{message}");
}

#[test]
fn motion_blur_smears_movement_and_keeps_still_frames_sharp() {
    // A 10 px white bar crossing at 80 px/s; at 30 fps a 360-degree shutter spans 2.67 px.
    let body = format!(
        r#"<div {BOX} style="position:absolute; left:0; top:0; width:10px; height:100px; background:#ffffff" data-hf-keyframes='[{{"at":0,"x":0}},{{"at":1,"x":80}},{{"at":2,"x":80}}]'></div>"#
    );
    let root = r#"style="background:#000000" data-motion-blur="on" data-motion-blur-shutter="360" data-motion-blur-samples="16""#;
    let sharp = pixels(
        "fx_mb_sharp",
        r#"style="background:#000000""#,
        &body,
        0.5,
        &[(38, 5), (51, 5)],
    );
    let blurred = pixels("fx_mb", root, &body, 0.5, &[(38, 5), (45, 5), (51, 5)]);
    assert_eq!(sharp, vec![[0, 0, 0], [0, 0, 0]]);
    assert!(
        blurred[0][0] > 0 && blurred[0][0] < 255,
        "the trailing edge is smeared: {blurred:?}"
    );
    assert_eq!(blurred[1], [255, 255, 255], "the core stays solid");
    assert!(
        blurred[2][0] > 0 && blurred[2][0] < 255,
        "the leading edge is smeared: {blurred:?}"
    );
    // Once the bar stops, frames are sharp again.
    let still = pixels(
        "fx_mb_still",
        root,
        &body,
        1.5,
        &[(79, 5), (85, 5), (90, 5)],
    );
    assert_eq!(still, vec![[0, 0, 0], [255, 255, 255], [0, 0, 0]]);
    let message = error(
        "fx_mb_bad",
        r#"data-motion-blur="on" data-motion-blur-samples="100""#,
        "",
    );
    assert!(message.contains("2 to 64"), "{message}");
}
