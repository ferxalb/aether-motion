//! `margin`: flex item spacing on both axes, alignment inside the margin box, and absolute offsets.

mod common;

use aether_hyperframes::parse::parse_file;
use common::{render, scratch};

const CANVAS: (usize, usize) = (200, 100);

fn red(p: [u8; 3]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

fn blue(p: [u8; 3]) -> bool {
    p[2] > 200 && p[0] < 60 && p[1] < 60
}

/// A 200x100 row with two 20x20 boxes (red, blue) and the given styles on each.
fn row(name: &str, container: &str, first: &str, second: &str) -> common::Shot {
    let body = format!(
        r#"<div class="clip" data-start="0" data-duration="1" style="left:0;top:0;width:200px;height:100px;display:flex;{container}">
        <div style="width:20px;height:20px;background:#f00;{first}"></div>
        <div style="width:20px;height:20px;background:#00f;{second}"></div></div>"#
    );
    render(name, CANVAS, 1.0, "", &body)
}

#[test]
fn main_axis_margins_push_items_apart() {
    let shot = row("m-main", "", "margin-right: 30px", "margin-left: 10px");
    let (red_left, _, red_right, _) = shot.bbox(red).unwrap();
    let (blue_left, ..) = shot.bbox(blue).unwrap();
    assert_eq!((red_left, red_right), (0, 19));
    assert_eq!(blue_left, 20 + 30 + 10);
}

#[test]
fn a_leading_margin_offsets_the_first_item() {
    let shot = row("m-lead", "", "margin: 12px 0 0 15px", "");
    let (left, top, ..) = shot.bbox(red).unwrap();
    assert_eq!((left, top), (15, 12));
    assert_eq!(shot.bbox(blue).unwrap().0, 15 + 20);
}

#[test]
fn cross_axis_alignment_uses_the_margin_box() {
    // Margin box is 20 + 10 + 10 = 40 tall, centered in 100: it starts at 30, the item at 40.
    let shot = row("m-cross", "align-items: center", "margin: 10px 0", "");
    assert_eq!(shot.bbox(red).unwrap().1, 40);
}

#[test]
fn margins_count_in_the_free_space_for_centering() {
    // Row content: 20 + 20 + 40 margin = 80 of 200, so it starts at 60.
    let shot = row(
        "m-center",
        "justify-content: center",
        "margin-right: 40px",
        "",
    );
    assert_eq!(shot.bbox(red).unwrap().0, 60);
}

#[test]
fn margins_shift_absolute_boxes_from_left_and_top() {
    let body = r#"<div class="clip" data-start="0" data-duration="1" style="left:10px;top:10px;width:20px;height:20px;margin:5px 0 0 7px;background:#f00"></div>"#;
    let shot = render("m-abs", CANVAS, 1.0, "", body);
    let (left, top, ..) = shot.bbox(red).unwrap();
    assert_eq!((left, top), (17, 15));
}

#[test]
fn negative_or_auto_margins_are_rejected() {
    let dir = scratch("m-error");
    for (name, value) in [("neg", "margin: -4px"), ("auto", "margin: 0 auto")] {
        let path = dir.join(format!("{name}.html"));
        std::fs::write(
            &path,
            format!(
                r#"<main data-composition-id="e" data-width="10" data-height="10" data-duration="1"><div class="clip" data-start="0" data-duration="1" style="{value}"></div></main>"#
            ),
        )
        .unwrap();
        assert!(parse_file(&path).is_err(), "{value} should be rejected");
    }
}
