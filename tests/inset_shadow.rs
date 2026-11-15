//! Inset `box-shadow`: placement under borders and content, mixing with outer layers, and determinism.

mod common;

use common::render;

const CLIP: &str = r#"data-start="0" data-duration="1""#;
const CANVAS: (usize, usize) = (120, 120);
const BASE: &str = ".b { left: 20px; top: 20px; width: 80px; height: 80px; background: #fff }";

fn shot(name: &str, style: &str) -> common::Shot {
    let body = format!(r#"<div class="clip b" style="{style}" {CLIP}></div>"#);
    render(name, CANVAS, 1.0, BASE, &body)
}

#[test]
fn an_inset_shadow_darkens_the_inside_edge_only() {
    let s = shot("inset_edge", "box-shadow: inset 6px 0 0 #000");
    assert_eq!(s.at(22, 60), [0, 0, 0], "exposed band on the left");
    assert_eq!(s.at(60, 60), [255, 255, 255], "interior stays white");
    let plain = shot("inset_plain", "");
    assert_eq!(s.at(10, 60), plain.at(10, 60), "nothing is cast outside");
}

#[test]
fn the_border_sits_above_the_inset_shadow() {
    let s = shot(
        "inset_border",
        "border: 4px solid #f00; box-shadow: inset 0 0 0 10px #00f",
    );
    assert_eq!(s.at(21, 60), [255, 0, 0], "border stays on top");
    assert_eq!(
        s.at(27, 60),
        [0, 0, 255],
        "shadow fills the padding box edge"
    );
}

#[test]
fn inset_and_outer_layers_mix_and_render_deterministically() {
    let style = "box-shadow: inset 0 4px 10px #0008, 0 10px 20px #000a";
    let a = shot("inset_mix_a", style);
    let b = shot("inset_mix_b", style);
    assert_eq!(a.pixels, b.pixels);
    assert!(a.at(60, 21)[0] < 255, "inset top shade");
    assert!(a.at(60, 108)[0] < 255, "outer shadow below the box");
}
