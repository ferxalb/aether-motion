//! `<br>`: forced line breaks inside an element's direct text.

mod common;

use common::render;

fn bands(name: &str, content: &str) -> Vec<(usize, usize)> {
    let body = format!(
        r#"<div class="clip" data-start="0" data-duration="1" style="left:10px;top:10px;width:280px;font-size:20px;color:#fff;line-height:1.5">{content}</div>"#
    );
    render(name, (300, 200), 1.0, "", &body).bands()
}

#[test]
fn br_splits_text_into_separate_lines() {
    assert_eq!(bands("br-one", "AAA").len(), 1);
    assert_eq!(bands("br-two", "AAA<br>AAA").len(), 2);
    assert_eq!(bands("br-three", "AAA<br/>AAA<br>AAA").len(), 3);
}

#[test]
fn a_trailing_br_adds_no_line_and_a_double_br_adds_a_blank_one() {
    assert_eq!(bands("br-trail", "AAA<br>").len(), 1);
    let found = bands("br-blank", "AAA<br><br>AAA");
    assert_eq!(found.len(), 2);
    let single = bands("br-adjacent", "AAA<br>AAA");
    assert!(
        found[1].0 > single[1].0,
        "the blank line pushes the last line down"
    );
}

#[test]
fn each_line_still_wraps_at_the_box_width() {
    let long = "AAA BBB CCC DDD EEE FFF GGG HHH III JJJ KKK LLL MMM NNN";
    assert!(bands("br-wrap", &format!("{long}<br>AAA")).len() >= 3);
}
