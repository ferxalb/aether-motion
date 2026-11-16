//! `filter: blur()`: softens the whole element, spreads past its box, and rejects unsupported forms.

mod common;

use aether_motion::parse::parse_file;
use common::{render, scratch};

/// Writes `html` into a scratch directory and returns the file path.
fn write(name: &str, html: &str) -> std::path::PathBuf {
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    path
}

const CLIP: &str = r#"data-start="0" data-duration="1""#;
const CANVAS: (usize, usize) = (160, 160);
const BASE: &str = ".b { left: 50px; top: 50px; width: 60px; height: 60px; background: #fff }";

fn shot(name: &str, style: &str) -> common::Shot {
    let body = format!(r#"<div class="clip b" style="{style}" {CLIP}></div>"#);
    render(name, CANVAS, 1.0, BASE, &body)
}

#[test]
fn blur_softens_edges_and_spreads_past_the_box() {
    let sharp = shot("blur_sharp", "");
    let soft = shot("blur_soft", "filter: blur(6px)");
    assert_eq!(sharp.at(49, 80), [0, 0, 0]);
    assert_eq!(sharp.at(50, 80), [255, 255, 255]);
    assert!(soft.at(46, 80)[0] > 0, "light leaks outside the box");
    assert!(soft.at(53, 80)[0] < 255, "the inside edge darkens");
    assert!(soft.at(46, 80)[0] < soft.at(53, 80)[0]);
    assert_eq!(soft.at(80, 80), [255, 255, 255], "the core stays solid");
    assert_eq!(soft.at(5, 5), [0, 0, 0], "far away nothing changes");
}

#[test]
fn blur_is_deterministic_and_none_is_a_no_op() {
    let a = shot("blur_a", "filter: blur(4px); border-radius: 12px");
    let b = shot("blur_b", "filter: blur(4px); border-radius: 12px");
    assert_eq!(a.pixels, b.pixels);
    let none = shot("blur_none", "filter: none");
    assert_eq!(none.pixels, shot("blur_plain", "").pixels);
}

#[test]
fn blur_includes_shadows_and_children() {
    let body = format!(
        r#"<div class="clip b" style="filter: blur(5px); box-shadow: 0 0 0 10px #f00" {CLIP}><div style="left: 20px; top: 20px; width: 20px; height: 20px; background: #00f"></div></div>"#
    );
    let s = render("blur_children", CANVAS, 1.0, BASE, &body);
    assert!(s.at(45, 80)[0] > 0, "the spread shadow is blurred too");
    assert!(s.at(80, 80)[2] > 0, "the child is drawn into the layer");
}

#[test]
fn unsupported_filters_fail_explicitly() {
    let cases = [
        (".a { filter: grayscale(1) }", "filter"),
        (".a { filter: blur(300px) }", "limited to 200px"),
        (".a { filter: blur(2px) brightness(2) }", "filter"),
    ];
    for (i, (css, expected)) in cases.iter().enumerate() {
        let path = write(
            &format!("filter_err{i}"),
            &format!(
                r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="1"><style>{css}</style><div class="clip a" data-start="0" data-duration="1"></div></div>"#
            ),
        );
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(error.contains(expected), "{css}: {error}");
    }
}

#[test]
fn blur_on_the_root_is_rejected() {
    let path = write(
        "filter_root",
        r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="1" style="filter: blur(3px)"></div>"#,
    );
    let error = format!("{:#}", parse_file(&path).unwrap_err());
    assert!(error.contains("root"), "{error}");
}
