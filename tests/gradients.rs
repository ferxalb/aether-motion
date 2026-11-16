//! General gradients (angles, corners, stops, alpha, radial) and `font-weight`.

mod common;

use aether_motion::parse::parse_file;
use common::{render, scratch, Shot};

const CANVAS: (usize, usize) = (100, 100);

fn paint(name: &str, background: &str) -> Shot {
    let body = format!(
        r#"<div class="clip" data-start="0" data-duration="1" style="left:0;top:0;width:100px;height:100px;background:{background}"></div>"#
    );
    render(name, CANVAS, 1.0, "", &body)
}

#[test]
fn an_angled_gradient_runs_along_its_diagonal() {
    let shot = paint("angle", "linear-gradient(to bottom right, #000, #fff)");
    let (start, middle, end) = (shot.at(2, 2)[0], shot.at(50, 50)[0], shot.at(97, 97)[0]);
    assert!(start < 12 && end > 243, "ends: {start} {end}");
    assert!(middle.abs_diff(128) <= 6, "middle: {middle}");
    let (tr, bl) = (shot.at(97, 2)[0], shot.at(2, 97)[0]);
    assert!(tr.abs_diff(128) <= 8 && bl.abs_diff(128) <= 8, "{tr} {bl}");
}

#[test]
fn intermediate_stops_are_honored() {
    let shot = paint(
        "stops",
        "linear-gradient(to right, #f00 0%, #00f 50%, #0f0 100%)",
    );
    let (left, mid, right) = (shot.at(1, 50), shot.at(50, 50), shot.at(98, 50));
    assert!(
        left[0] > 240 && mid[2] > 240 && right[1] > 240,
        "{left:?} {mid:?} {right:?}"
    );
}

#[test]
fn translucent_stops_blend_over_what_is_below() {
    let shot = paint(
        "alpha",
        "linear-gradient(to right, rgba(255,255,255,0), #fff)",
    );
    assert!(shot.at(1, 50)[0] < 8);
    assert!(shot.at(98, 50)[0] > 247);
    assert!(shot.at(50, 50)[0].abs_diff(128) <= 8);
}

#[test]
fn a_radial_gradient_is_brightest_at_its_center() {
    let shot = paint("radial", "radial-gradient(circle at center, #fff, #000)");
    assert!(shot.at(50, 50)[0] > 245);
    assert!(shot.at(50, 10)[0] < shot.at(50, 30)[0]);
    assert!(shot.at(2, 2)[0] < 16, "corner is the farthest point");
}

#[test]
fn gradients_are_deterministic() {
    let a = paint("det-a", "radial-gradient(#7894ff, #080d18)");
    let b = paint("det-b", "radial-gradient(#7894ff, #080d18)");
    assert_eq!(a.pixels, b.pixels);
}

#[test]
fn bold_text_is_wider_than_regular_text() {
    let width = |weight: &str| {
        let body = format!(
            r#"<div class="clip" data-start="0" data-duration="1" style="left:5px;top:5px;font-size:24px;color:#fff;white-space:nowrap;font-weight:{weight}">Hamburgefons</div>"#
        );
        let shot = render("weight", (300, 60), 1.0, "", &body);
        let (l, _, r, _) = shot.bbox(|p| p[0] > 128).unwrap();
        r - l
    };
    assert!(width("700") > width("400"), "bold must change the face");
}

#[test]
fn unsupported_values_fail_loudly() {
    let dir = scratch("errors");
    for (name, css) in [
        ("overflow", "overflow: scroll"),
        ("weight", "font-weight: heavy"),
        ("conic", "background: conic-gradient(red, blue)"),
    ] {
        let path = dir.join(format!("{name}.html"));
        std::fs::write(
            &path,
            format!(
                r#"<main data-composition-id="e" data-width="10" data-height="10" data-duration="1"><div class="clip" data-start="0" data-duration="1" style="{css}"></div></main>"#
            ),
        )
        .unwrap();
        assert!(parse_file(&path).is_err(), "{css} should be rejected");
    }
}
