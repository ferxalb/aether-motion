//! Light, focus, and glitch transitions, and the `glitch` / `rgbSplit` keyframe effects.

mod common;

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;

/// Two full-size scenes, red then blue, joined by `kind` over 1.0..2.0 s of a 3 s composition.
fn scenes(kind: &str) -> String {
    format!(
        r#"<div id="root" data-composition-id="x" data-width="100" data-height="100" data-duration="3" style="background:#00ff00">
  <div id="a" class="clip" data-start="0" data-duration="2" data-track-index="0" style="position:absolute; left:0; top:0; width:100px; height:100px; background:#ff0000"></div>
  <div id="b" class="clip" data-start="a-1" data-duration="2" data-track-index="1" data-hf-transition="{kind}" data-hf-transition-from="a" data-hf-transition-easing="linear" style="position:absolute; left:0; top:0; width:100px; height:100px; background:#0000ff"></div>
</div>"#
    )
}

fn frame(name: &str, html: &str, time: f64) -> Vec<u8> {
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    renderer.render_at(time).unwrap().to_vec()
}

fn pixel(frame: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * 100 + x) * 4;
    [frame[i], frame[i + 1], frame[i + 2]]
}

#[test]
fn dips_pass_through_a_solid_color() {
    let html = scenes("dip-to-white");
    assert_eq!(pixel(&frame("dip_w", &html, 0.5), 50, 50), [255, 0, 0]);
    assert_eq!(pixel(&frame("dip_w", &html, 1.5), 50, 50), [255, 255, 255]);
    let quarter = pixel(&frame("dip_w", &html, 1.25), 50, 50);
    assert_eq!(quarter[0], 255);
    assert!((126..=129).contains(&quarter[1]), "{quarter:?}");
    assert_eq!(pixel(&frame("dip_w", &html, 2.5), 50, 50), [0, 0, 255]);

    let html = scenes("dip-to-black");
    assert_eq!(pixel(&frame("dip_b", &html, 1.5), 50, 50), [0, 0, 0]);
    let late = pixel(&frame("dip_b", &html, 1.75), 50, 50);
    assert_eq!(late[..2], [0, 0]);
    assert!((126..=129).contains(&late[2]), "{late:?}");
}

#[test]
fn flash_peaks_white_at_the_cut() {
    let html = scenes("flash");
    assert_eq!(pixel(&frame("flash", &html, 1.5), 50, 50), [255, 255, 255]);
    assert_eq!(pixel(&frame("flash", &html, 2.0), 50, 50), [0, 0, 255]);
}

#[test]
fn glitch_is_deterministic_and_settles() {
    let html = scenes("glitch");
    let a = frame("glitch", &html, 1.45);
    assert_eq!(a, frame("glitch", &html, 1.45), "same instant, same pixels");
    // Before the cut only the outgoing scene shows, after the window only the incoming one.
    assert_eq!(pixel(&a, 50, 50), [255, 0, 0]);
    assert_eq!(pixel(&frame("glitch", &html, 2.2), 50, 50), [0, 0, 255]);
}

#[test]
fn keyframed_rgb_split_separates_red_and_blue() {
    let html = r#"<div id="root" data-composition-id="x" data-width="100" data-height="100" data-duration="2" style="background:#000000">
  <div class="clip" data-start="0" data-duration="2" style="position:absolute; left:40px; top:40px; width:20px; height:20px; background:#ffffff"
       data-hf-keyframes='[{"at":0,"rgbSplit":4},{"at":1,"rgbSplit":0}]'></div>
</div>"#;
    let f = frame("split", html, 0.0);
    assert_eq!(pixel(&f, 38, 50), [255, 0, 0], "red moves left");
    assert_eq!(pixel(&f, 61, 50), [0, 0, 255], "blue moves right");
    assert_eq!(pixel(&f, 50, 50), [255, 255, 255]);
    let settled = frame("split", html, 1.5);
    assert_eq!(pixel(&settled, 38, 50), [0, 0, 0]);
}

#[test]
fn keyframed_glitch_moves_slices() {
    let html = |glitch: f32| {
        format!(
            r#"<div id="root" data-composition-id="x" data-width="100" data-height="100" data-duration="2" style="background:#000000">
  <div class="clip" data-start="0" data-duration="2" style="position:absolute; left:0; top:0; width:100px; height:100px; background:linear-gradient(to right, #000000, #ffffff)"
       data-hf-keyframes='[{{"at":0,"glitch":{glitch}}}]'></div>
</div>"#
        )
    };
    assert_ne!(frame("kg1", &html(1.0), 0.5), frame("kg0", &html(0.0), 0.5));
}

#[test]
fn rejects_unknown_kinds_and_bad_amounts() {
    let path = scratch("fx_bad").join("c.html");
    std::fs::write(&path, scenes("whip-sideways")).unwrap();
    assert!(format!("{:#}", parse_file(&path).unwrap_err()).contains("whip-<dir>"));
    let html = r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="2"><div class="clip" data-start="0" data-duration="2" data-hf-keyframes='[{"at":0,"glitch":2}]'></div></div>"#;
    std::fs::write(&path, html).unwrap();
    assert!(format!("{:#}", parse_file(&path).unwrap_err()).contains("glitch 0 to 1"));
}
