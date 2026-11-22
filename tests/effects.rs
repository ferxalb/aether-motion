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
        "fx_blend_unknown",
        "",
        &format!(r#"<div {BOX} style="mix-blend-mode:vivid-light"></div>"#),
    );
    assert!(message.contains("vivid-light"), "{message}");
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

/// A 60x60 box of `color` at (20, 20) with the given extra style and attributes, over black.
fn animated_box(color: &str, style: &str, attrs: &str) -> String {
    format!(
        r#"<div {BOX} style="position:absolute; left:20px; top:20px; width:60px; height:60px; background:{color}; {style}" {attrs}></div>"#
    )
}

#[test]
fn keyframed_color_filters_interpolate_with_easing() {
    let body = animated_box(
        "#646464",
        "",
        r#"data-hf-keyframes='[{"at":0,"brightness":0},{"at":1,"brightness":2}]'"#,
    );
    let root = r#"style="background:#000000""#;
    // Gray 100 scaled by 0.5, 1, and 1.5; held at 2 after the last keyframe.
    assert_eq!(
        pixels("fx_kf_b1", root, &body, 0.25, &[(50, 50)])[0],
        [50, 50, 50]
    );
    assert_eq!(
        pixels("fx_kf_b2", root, &body, 0.5, &[(50, 50)])[0],
        [100, 100, 100]
    );
    assert_eq!(
        pixels("fx_kf_b3", root, &body, 0.75, &[(50, 50)])[0],
        [150, 150, 150]
    );
    assert_eq!(
        pixels("fx_kf_b4", root, &body, 1.5, &[(50, 50)])[0],
        [200, 200, 200]
    );
}

#[test]
fn animated_functions_follow_the_static_list() {
    // Static invert(1) comes first; the animated brightness absent from it is applied after.
    let body = animated_box(
        "#333333",
        "filter: invert(1)",
        r#"data-hf-keyframes='[{"at":0,"brightness":0.5},{"at":1}]'"#,
    );
    let px = pixels(
        "fx_kf_order",
        r#"style="background:#000000""#,
        &body,
        0.5,
        &[(50, 50)],
    );
    assert_eq!(px[0], [102, 102, 102], "invert 51 -> 204, then halve");
    // An animated function listed statically keeps its place and takes the animated amount.
    let body = animated_box(
        "#333333",
        "filter: brightness(1) invert(1)",
        r#"data-hf-keyframes='[{"at":0,"brightness":0.5},{"at":1}]'"#,
    );
    let px = pixels(
        "fx_kf_in_place",
        r#"style="background:#000000""#,
        &body,
        0.5,
        &[(50, 50)],
    );
    assert_eq!(px[0], [230, 230, 230], "halve 51 -> 25.5, then invert");
}

#[test]
fn animated_blur_changes_continuously() {
    let body = animated_box(
        "#ffffff",
        "",
        r#"data-hf-keyframes='[{"at":0,"blur":0},{"at":2,"blur":3}]'"#,
    );
    let root = r#"style="background:#000000""#;
    let mut previous = 0;
    // One pixel outside the left edge: more light leaks out as the blur grows, frame after frame
    // (blur 0.3 to 2.85 px; a whole-radius kernel would repeat values here).
    for step in 2..=19 {
        let time = f64::from(step) * 0.1;
        let value = pixels(&format!("fx_kf_blur{step}"), root, &body, time, &[(19, 50)])[0][0];
        assert!(value > previous, "t={time}: {value} after {previous}");
        previous = value;
    }
    // At blur 0 the element is drawn directly, exactly like no filter.
    let sharp = pixels("fx_kf_blur0", root, &body, 0.0, &[(19, 50), (20, 50)]);
    assert_eq!(sharp, vec![[0, 0, 0], [255, 255, 255]]);
}

#[test]
fn css_keyframes_animate_filter_lists() {
    let html = format!(
        r#"<style>@keyframes g {{ from {{ filter: grayscale(0) }} to {{ filter: blur(0px) grayscale(1) }} }} .a {{ animation: g 1s linear forwards }}</style>{}"#,
        animated_box("#ff0000", "", "").replacen(r#"class="clip""#, r#"class="clip a""#, 1)
    );
    let root = r#"style="background:#000000""#;
    let start = pixels("fx_css_f0", root, &html, 0.0, &[(50, 50)])[0];
    let middle = pixels("fx_css_f1", root, &html, 0.5, &[(50, 50)])[0];
    let end = pixels("fx_css_f2", root, &html, 1.5, &[(50, 50)])[0];
    assert_eq!(start, [255, 0, 0]);
    assert!(
        middle[0] < 255 && middle[1] > 0 && middle[1] < end[1],
        "{middle:?}"
    );
    assert_eq!(end, [54, 54, 54]);
}

#[test]
fn invalid_filter_animations_are_rejected() {
    let cases = [
        (
            "",
            animated_box("#fff", "", r#"data-hf-keyframes='[{"at":0,"blur":300}]'"#),
            "out of range",
        ),
        (
            "",
            animated_box(
                "#fff",
                "filter: sepia(1) sepia(0.5)",
                r#"data-hf-keyframes='[{"at":0,"sepia":0}]'"#,
            ),
            "appears 2 times",
        ),
        (
            r#"data-hf-keyframes='[{"at":0,"blur":2}]'"#,
            String::new(),
            "root",
        ),
        (
            "",
            animated_box(
                "#fff",
                "filter: sepia(1) blur(2px)",
                r#"data-hf-keyframes='[{"at":0,"dropShadow":"0 0 4px red"}]'"#,
            ),
            "list drop-shadow()",
        ),
        (
            "",
            animated_box(
                "#fff",
                "",
                r#"data-hf-keyframes='[{"at":0,"dropShadow":"0 0 4px 2px red"}]'"#,
            ),
            "spread",
        ),
        (
            "",
            animated_box(
                "#fff",
                "",
                r#"data-hf-keyframes='[{"at":0,"dropShadow":4}]'"#,
            ),
            "must be a string",
        ),
    ];
    for (i, (root, body, expected)) in cases.iter().enumerate() {
        let message = error(&format!("fx_kf_err{i}"), root, body);
        assert!(message.contains(expected), "{expected}: {message}");
    }
}

#[test]
fn single_pass_color_path_matches_the_plane_path() {
    // A transparent drop shadow forces the plane path without changing any pixel.
    let points: Vec<(usize, usize)> = (0..100)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .collect();
    for (i, style) in [
        "filter: sepia(0.7) hue-rotate(40deg) contrast(1.3); border-radius: 18px",
        "filter: invert(0.4) saturate(2); mix-blend-mode: overlay",
        "filter: grayscale(0.5) opacity(0.6); mix-blend-mode: multiply",
    ]
    .iter()
    .enumerate()
    {
        let body = |extra: &str| {
            format!(
                r#"<div {BOX} style="position:absolute; left:10px; top:15px; width:70px; height:60px; background: linear-gradient(90deg, #ff3366, #33ddff); {style}{extra}"></div>"#
            )
        };
        let root = r#"style="background: linear-gradient(180deg, #202040, #c0a060)""#;
        let fused = pixels(&format!("fx_fused{i}"), root, &body(""), 0.0, &points);
        // Append the shadow to the end of the `filter` value (the first `;` after it).
        let plain = body("");
        let at = plain.find("filter: ").unwrap();
        let end = at + plain[at..].find(';').unwrap();
        let planes_body = format!(
            "{} drop-shadow(0 0 0 transparent){}",
            &plain[..end],
            &plain[end..]
        );
        assert_ne!(planes_body, body(""));
        let planes = pixels(&format!("fx_planes{i}"), root, &planes_body, 0.0, &points);
        assert_eq!(fused, planes, "{style}");
    }
}

#[test]
fn shared_tile_caches_keep_layers_consistent_across_frames() {
    // The box moves 1 px per frame (10 fps), so its gradient keeps one size and is cached from the
    // second frame on, inside the effect layer, while the first frame draws it directly.
    let html = format!(
        r#"<div id="root" data-composition-id="x" data-width="100" data-height="100" data-duration="1" style="background:#102030"><div {BOX} style="position:absolute; left:10px; top:10px; width:60px; height:60px; background: radial-gradient(ellipse farthest-side at 40% 40%, rgba(255, 90, 140, 0.9), rgba(111, 141, 255, 0.35) 60%, rgba(111, 141, 255, 0)); filter: hue-rotate(30deg) contrast(1.2)" data-hf-keyframes='[{{"at":0,"x":0}},{{"at":1,"x":10}}]'></div></div>"#
    );
    let html = html.replace(r#"data-duration="2""#, r#"data-duration="1""#);
    let path = scratch("fx_shared_cache").join("c.html");
    std::fs::write(&path, html).unwrap();
    let composition = parse_file(&path).unwrap();
    let mut sequential = Renderer::new(&composition, 1.0).unwrap();
    for frame in 0..6 {
        let warm = sequential.render_frame(frame, 10).unwrap().to_vec();
        let cold = Renderer::new(&composition, 1.0)
            .unwrap()
            .render_frame(frame, 10)
            .unwrap()
            .to_vec();
        let worst = warm
            .iter()
            .zip(&cold)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 1, "frame {frame}: cached layer differs by {worst}");
    }
}

/// Renders a 100x100 composition at time 0 and returns all of its RGBA pixels.
fn frame(name: &str, root: &str, body: &str) -> Vec<u8> {
    let html = format!(
        r#"<div id="root" data-composition-id="x" data-width="100" data-height="100" data-duration="2" {root}>{body}</div>"#
    );
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    renderer.render_at(0.0).unwrap().to_vec()
}

#[test]
fn filter_functions_run_in_the_order_written() {
    let shadowed = |name: &str, filter: &str| {
        let body = format!(
            r#"<div {BOX} style="position:absolute; left:20px; top:20px; width:40px; height:40px; background:#ffffff; filter: {filter}"></div>"#
        );
        pixels(
            name,
            r#"style="background:#808080""#,
            &body,
            0.0,
            &[(40, 40), (65, 65)],
        )
    };
    // Grayscale after the drop shadow also grays the shadow: Rec. 709 luma of blue is 0.0722.
    assert_eq!(
        shadowed(
            "fx_order_late",
            "drop-shadow(10px 10px 0 #0000ff) grayscale(1)"
        ),
        vec![[255, 255, 255], [18, 18, 18]]
    );
    assert_eq!(
        shadowed(
            "fx_order_usual",
            "grayscale(1) drop-shadow(10px 10px 0 #0000ff)"
        ),
        vec![[255, 255, 255], [0, 0, 255]]
    );
    // A blur after the drop shadow blurs the element and its shadow together, like a blurred
    // wrapper around an element with that shadow (which rounds once more to 8 bits in between).
    let single = frame(
        "fx_order_blur",
        r#"style="background:#808080""#,
        &format!(
            r#"<div {BOX} style="position:absolute; left:20px; top:20px; width:40px; height:40px; background:#ffffff; filter: drop-shadow(3px 3px 0 #000000) blur(3px)"></div>"#
        ),
    );
    let nested = frame(
        "fx_order_nested",
        r#"style="background:#808080""#,
        &format!(
            r#"<div {BOX} style="position:absolute; left:20px; top:20px; width:40px; height:40px; filter: blur(3px)"><div style="position:absolute; left:0; top:0; width:40px; height:40px; background:#ffffff; filter: drop-shadow(3px 3px 0 #000000)"></div></div>"#
        ),
    );
    let worst = single
        .iter()
        .zip(&nested)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 2, "largest channel difference {worst}");
    // The shadow's far corner is soft, not the hard edge of the usual order without blur.
    let corner = (62 * 100 + 62) * 4;
    assert!(
        single[corner] > 0 && single[corner] < 128,
        "{}",
        single[corner]
    );
}

#[test]
fn a_custom_filter_order_lists_its_animated_functions() {
    let message = error(
        "fx_order_appended",
        "",
        &animated_box(
            "#fff",
            "filter: drop-shadow(1px 1px 0 #000) sepia(1)",
            r#"data-hf-keyframes='[{"at":0,"invert":0},{"at":1,"invert":1}]'"#,
        ),
    );
    assert!(message.contains("custom order"), "{message}");
    // Listed functions animate in place.
    let body = animated_box(
        "#fff",
        "filter: drop-shadow(1px 1px 0 #000) invert(0)",
        r#"data-hf-keyframes='[{"at":0,"invert":0},{"at":1,"invert":1}]'"#,
    );
    let html = format!(
        r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="2">{body}</div>"#
    );
    let path = scratch("fx_order_listed").join("c.html");
    std::fs::write(&path, html).unwrap();
    assert!(parse_file(&path).is_ok());
}

#[test]
fn opacity_filter_animates_and_fades_a_later_drop_shadow() {
    // White over black: the filter's opacity scales the element's alpha.
    let at = |name: &str, style: &str, attrs: &str, time: f64, points: &[(usize, usize)]| {
        pixels(
            name,
            r#"style="background:#000000""#,
            &format!(
                r#"<div {BOX} style="position:absolute; left:20px; top:20px; width:40px; height:40px; background:#ffffff; {style}" {attrs}></div>"#
            ),
            time,
            points,
        )
    };
    let keys = r#"data-hf-keyframes='[{"at":0,"filterOpacity":1},{"at":1,"filterOpacity":0}]'"#;
    assert_eq!(at("fx_op0", "", keys, 0.0, &[(40, 40)]), vec![[255; 3]]);
    assert_eq!(at("fx_op5", "", keys, 0.5, &[(40, 40)]), vec![[128; 3]]);
    assert_eq!(at("fx_op1", "", keys, 1.0, &[(40, 40)]), vec![[0; 3]]);
    // An opacity listed before a white drop shadow fades the shadow with the element: half
    // alpha alone at (65, 65), and over the half-alpha shadow 0.5 + 0.5 * 0.5 at (40, 40).
    let style = "filter: opacity(1) drop-shadow(10px 10px 0 #ffffff)";
    assert_eq!(
        at("fx_op_shadow", style, keys, 0.5, &[(40, 40), (65, 65)]),
        vec![[191; 3], [128; 3]]
    );
    // CSS @keyframes interpolate opacity() like the other color functions.
    let css = pixels(
        "fx_op_css",
        r#"style="background:#000000""#,
        r#"<style>@keyframes f { from { filter: opacity(1) } to { filter: opacity(0) } } .a { animation: f 1s linear both }</style><div class="clip a" data-start="0" data-duration="2" style="position:absolute; left:20px; top:20px; width:40px; height:40px; background:#ffffff"></div>"#,
        0.25,
        &[(40, 40)],
    );
    assert_eq!(css, vec![[191; 3]]);
    let message = error(
        "fx_op_range",
        "",
        &animated_box(
            "#fff",
            "",
            r#"data-hf-keyframes='[{"at":0,"filterOpacity":2}]'"#,
        ),
    );
    assert!(message.contains("out of range"), "{message}");
}

#[test]
fn drop_shadow_animates_smoothly_and_lands_on_the_static_shadow() {
    let render = |name: &str, style: &str, attrs: &str, time: f64| {
        let html = format!(
            r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="2" style="background:#808080"><div {BOX} style="position:absolute; left:20px; top:20px; width:40px; height:40px; background:#ffffff; {style}" {attrs}></div></div>"#
        );
        let path = scratch(name).join("c.html");
        std::fs::write(&path, html).unwrap();
        let composition = parse_file(&path).unwrap();
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        renderer.render_at(time).unwrap().to_vec()
    };
    let red = |frame: &[u8], x: usize, y: usize| frame[(y * 100 + x) * 4];
    let keys = r#"data-hf-keyframes='[{"at":0,"dropShadow":"0 0 0 #000"},{"at":1,"dropShadow":"10px 10px 0 #000"}]'"#;
    // At the end it is exactly the static shadow.
    assert_eq!(
        render("fx_ds_end", "", keys, 1.0),
        render(
            "fx_ds_static",
            "filter: drop-shadow(10px 10px 0 #000)",
            "",
            0.0
        )
    );
    // A quarter of the way, the offset is 2.5 px: the shadow's right edge at x = 62.5 covers
    // pixel 61 fully and pixel 62 half (black over gray 128).
    let quarter = render("fx_ds_quarter", "", keys, 0.25);
    assert_eq!(red(&quarter, 61, 45), 0);
    assert_eq!(red(&quarter, 62, 45), 64);
    assert_eq!(red(&quarter, 63, 45), 128);
    // CSS @keyframes interpolate the color premultiplied, from a transparent shadow.
    let html = r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="2" style="background:#808080"><style>@keyframes s { from { filter: drop-shadow(0 0 0 transparent) } to { filter: drop-shadow(10px 10px 0 #000) } } .a { animation: s 1s linear both }</style><div class="clip a" data-start="0" data-duration="2" style="position:absolute; left:20px; top:20px; width:40px; height:40px; background:#ffffff"></div></div>"#;
    let path = scratch("fx_ds_css").join("c.html");
    std::fs::write(&path, html).unwrap();
    let composition = parse_file(&path).unwrap();
    let css = Renderer::new(&composition, 1.0)
        .unwrap()
        .render_at(0.5)
        .unwrap()
        .to_vec();
    assert_eq!(red(&css, 63, 63), 64, "half alpha at a 5 px offset");
    assert_eq!(red(&css, 66, 66), 128, "nothing past the 5 px offset");
    // `none` fades the static shadow out.
    let gone = render(
        "fx_ds_none",
        "filter: drop-shadow(10px 10px 0 #000)",
        r#"data-hf-keyframes='[{"at":0},{"at":1,"dropShadow":"none"}]'"#,
        1.0,
    );
    assert_eq!(red(&gone, 65, 65), 128);
}

#[test]
fn non_separable_blend_modes_parse_and_render() {
    let blended = |mode: &str| {
        let body = format!(
            r#"<div {BOX} style="position:absolute; left:20px; top:20px; width:40px; height:40px; background:#ff0000; mix-blend-mode: {mode}"></div>"#
        );
        pixels(
            &format!("fx_blend_{mode}"),
            r#"style="background:#808080""#,
            &body,
            0.0,
            &[(40, 40), (5, 5)],
        )
    };
    // Red has luminosity 0.3; gray has no saturation.
    assert_eq!(blended("luminosity"), vec![[77; 3], [128; 3]]);
    assert_eq!(blended("hue"), vec![[128; 3], [128; 3]]);
    assert_eq!(blended("saturation"), vec![[128; 3], [128; 3]]);
    let color = blended("color")[0];
    assert!(color[0] > color[1] && color[1] == color[2], "{color:?}");
}
