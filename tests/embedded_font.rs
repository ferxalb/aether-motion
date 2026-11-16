//! Embedded Roboto: generic families and unknown fonts render identically on every machine.

mod common;

use common::render;

fn ink(name: &str, family: &str) -> (usize, usize, usize, usize) {
    let body = format!(
        r#"<div class="clip" data-start="0" data-duration="1" style="left:20px;top:20px;font-size:40px;color:#fff;white-space:nowrap;font-family:{family}">Embedded gjy</div>"#
    );
    render(name, (400, 100), 1.0, "", &body)
        .bbox(|p| p[0] > 128)
        .unwrap()
}

#[test]
fn generic_families_and_unknown_fonts_use_the_embedded_face() {
    let reference = ink("ef-sans", "sans-serif");
    assert_eq!(ink("ef-serif", "serif"), reference);
    assert_eq!(ink("ef-mono", "monospace"), reference);
    assert_eq!(ink("ef-unknown", "NoSuchFont123, sans-serif"), reference);
    assert_eq!(ink("ef-unknown-only", "NoSuchFont123"), reference);
}
