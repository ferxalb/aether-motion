//! `overflow: hidden`: content is clipped to the padding box, rounded corners included.

mod common;

use common::render;

fn red(p: [u8; 3]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

const CANVAS: (usize, usize) = (120, 100);

/// A 40x40 box at (10, 10) holding a 100x100 red child that starts at the box corner.
fn clipped(name: &str, box_style: &str, child_style: &str) -> common::Shot {
    let body = format!(
        r#"<div class="clip" data-start="0" data-duration="1" style="left:10px;top:10px;width:40px;height:40px;overflow:hidden;{box_style}">
        <div style="left:0;top:0;width:100px;height:100px;background:#f00;{child_style}"></div></div>"#
    );
    render(name, CANVAS, 1.0, "", &body)
}

#[test]
fn a_child_larger_than_its_box_is_cut_at_the_box_edge() {
    let shot = clipped("ov-rect", "", "");
    assert_eq!(shot.bbox(red), Some((10, 10, 49, 49)));
}

#[test]
fn a_child_moved_outside_is_hidden_and_one_moved_halfway_is_cut() {
    assert_eq!(clipped("ov-out", "", "left:60px").bbox(red), None);
    let half = clipped("ov-half", "", "left:20px;top:0;width:60px;height:20px");
    assert_eq!(half.bbox(red), Some((30, 10, 49, 29)));
}

#[test]
fn without_overflow_the_child_paints_everywhere() {
    let body = r#"<div class="clip" data-start="0" data-duration="1" style="left:10px;top:10px;width:40px;height:40px">
        <div style="left:0;top:0;width:100px;height:100px;background:#f00"></div></div>"#;
    let shot = render("ov-visible", CANVAS, 1.0, "", body);
    assert_eq!(shot.bbox(red), Some((10, 10, 109, 99)));
}

#[test]
fn rounded_corners_clip_the_content_too() {
    let shot = clipped("ov-round", "border-radius:16px", "");
    assert!(!red(shot.at(10, 10)), "the corner pixel is outside the arc");
    assert!(red(shot.at(30, 30)));
    assert!(red(shot.at(30, 10)), "the straight edge stays filled");
}

#[test]
fn the_border_stays_visible_and_content_clips_inside_it() {
    let shot = clipped("ov-border", "border:4px solid #00f", "");
    assert_eq!(
        shot.at(11, 30),
        [0, 0, 255],
        "border is drawn over the box edge"
    );
    assert_eq!(shot.bbox(red), Some((14, 14, 45, 45)));
}

#[test]
fn clipping_works_at_a_different_output_scale() {
    let shot = render(
        "ov-scale",
        CANVAS,
        2.0,
        "",
        r#"<div class="clip" data-start="0" data-duration="1" style="left:10px;top:10px;width:40px;height:40px;overflow:hidden">
        <div style="left:0;top:0;width:100px;height:100px;background:#f00"></div></div>"#,
    );
    assert_eq!(shot.bbox(red), Some((20, 20, 99, 99)));
}
