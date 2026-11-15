//! Content-sized (auto) boxes in flex layouts: borders, padding, and percentage children.

mod common;

use common::render;

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLACK: [u8; 3] = [0, 0, 0];
const CLIP: &str = r#"data-start="0" data-duration="1""#;
const STAGE: &str = ".stage { left: 0; top: 0; width: 100%; height: 100%; display: flex;
                     justify-content: center; align-items: center }";

#[test]
fn auto_size_flex_container_includes_its_border() {
    let css = format!(
        "{STAGE} .box {{ display: flex; border: 10px solid #f00 }}
         .kid {{ width: 40px; height: 20px; background: #0f0 }}"
    );
    let body = format!(
        r#"<div class="clip stage" {CLIP}><div class="clip box" {CLIP}>
             <div class="clip kid" {CLIP}></div></div></div>"#
    );
    let shot = render("border", (200, 100), 1.0, &css, &body);
    // Border box 60x40 centered in 200x100: x 70..130, y 30..70; the child sits inside it.
    assert_eq!(shot.at(70, 30), RED);
    assert_eq!(shot.at(129, 69), RED);
    assert_eq!(shot.at(69, 30), BLACK);
    assert_eq!(shot.at(130, 69), BLACK);
    assert_eq!(shot.at(80, 40), GREEN);
    assert_eq!(shot.at(119, 59), GREEN);
}

#[test]
fn auto_size_text_includes_its_padding() {
    let css = format!("{STAGE} .t {{ font-size: 20px; padding: 10px; background: #f00 }}");
    let body =
        format!(r#"<div class="clip stage" {CLIP}><div class="clip t" {CLIP}>Hi</div></div>"#);
    let shot = render("padtext", (200, 100), 1.0, &css, &body);
    let (mut left, mut right, mut top, mut bottom) = (usize::MAX, 0, usize::MAX, 0);
    for y in 0..shot.height {
        for x in 0..shot.width {
            if shot.at(x, y) == RED {
                (left, right) = (left.min(x), right.max(x));
                (top, bottom) = (top.min(y), bottom.max(y));
            }
        }
    }
    // The red box is the text (about one em tall) plus 10px on every side, centered on the stage.
    let (width, height) = (right - left + 1, bottom - top + 1);
    assert_eq!(height, 40, "20px line plus 2 * 10px padding");
    assert!(width > 20, "text width plus padding, got {width}");
    assert!((left + right) as i32 / 2 - 100 <= 1 && 100 - (left + right) as i32 / 2 <= 1);
    assert!((top + bottom) / 2 == 49 || (top + bottom) / 2 == 50);
}

#[test]
fn percentage_children_add_nothing_to_an_auto_size_container() {
    // CSS treats the cyclic percentage as auto: the container is as wide as its fixed child (40px)
    // and the percentage child resolves against that width afterwards.
    let css = format!(
        "{STAGE} .box {{ display: flex; background: #00f }}
         .p {{ width: 50%; height: 20px; background: #0f0 }}
         .f {{ width: 40px; height: 20px; background: #f00 }}"
    );
    let body = format!(
        r#"<div class="clip stage" {CLIP}><div class="clip box" {CLIP}>
             <div class="clip p" {CLIP}></div><div class="clip f" {CLIP}></div></div></div>"#
    );
    let shot = render("percent", (200, 100), 1.0, &css, &body);
    assert_eq!(
        shot.at(80, 50),
        GREEN,
        "50% of the 40px container starts at its left edge"
    );
    assert_eq!(shot.at(99, 50), GREEN);
    assert_eq!(shot.at(100, 50), RED);
    assert_eq!(
        shot.at(60, 50),
        BLACK,
        "the container is not widened by the percentage"
    );
}
