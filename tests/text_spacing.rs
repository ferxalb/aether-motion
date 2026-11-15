//! `letter-spacing`: extra advance between glyphs, inheritance, and errors.

mod common;

use aether_hyperframes::parse::parse_file;
use common::{render, scratch};

fn ink_width(name: &str, style: &str) -> usize {
    let body = format!(
        r#"<div class="clip" data-start="0" data-duration="1" style="left:20px;top:20px;font-size:40px;color:#fff;white-space:nowrap;{style}">HHH</div>"#
    );
    let shot = render(name, (300, 100), 1.0, "", &body);
    let (left, _, right, _) = shot.bbox(|p| p[0] > 128).unwrap();
    right - left
}

#[test]
fn spacing_adds_a_gap_after_each_glyph() {
    let plain = ink_width("sp-plain", "");
    let wide = ink_width("sp-wide", "letter-spacing: 10px");
    assert!(
        (wide as i32 - plain as i32 - 20).abs() <= 2,
        "two gaps of 10px: plain {plain}, wide {wide}"
    );
}

#[test]
fn negative_spacing_tightens_and_normal_resets() {
    let plain = ink_width("sp-base", "");
    assert!(ink_width("sp-neg", "letter-spacing: -3px") < plain);
    assert_eq!(ink_width("sp-normal", "letter-spacing: normal"), plain);
}

#[test]
fn spacing_is_inherited() {
    let body = r#"<div class="clip" data-start="0" data-duration="1" style="left:20px;top:20px;width:260px;display:flex;letter-spacing:10px">
        <div style="font-size:40px;color:#fff;white-space:nowrap">HHH</div></div>"#;
    let shot = render("sp-inherit", (300, 100), 1.0, "", body);
    let (left, _, right, _) = shot.bbox(|p| p[0] > 128).unwrap();
    assert!(right - left > ink_width("sp-ref", "") + 15);
}

#[test]
fn percentages_are_rejected() {
    let dir = scratch("sp-error");
    let path = dir.join("e.html");
    std::fs::write(
        &path,
        r#"<main data-composition-id="e" data-width="10" data-height="10" data-duration="1"><div class="clip" data-start="0" data-duration="1" style="letter-spacing: 5%">x</div></main>"#,
    )
    .unwrap();
    assert!(parse_file(&path).is_err());
}
