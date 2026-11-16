//! Multi-line text: wrapping, `max-width`, `line-height`, `white-space`, and flex sizing.
//!
//! Assertions use structure (line counts, edges, relative offsets) instead of absolute glyph
//! metrics, so they hold for any system font.

mod common;

use aether_motion::parse::parse_file;
use common::{render, scratch};

const WORDS: &str = "HHHH HHHH HHHH HHHH";

/// One absolutely positioned text clip whose style is `extra`.
fn text_clip(extra: &str) -> String {
    format!(r#"<div class="clip t" style="{extra}" data-start="0" data-duration="1">{WORDS}</div>"#)
}

const BASE: &str = ".t { left: 0; top: 0; font-size: 20px; line-height: 1.5 }";
const CANVAS: (usize, usize) = (480, 320);

/// Width of the text on a single line, measured by painting it with `white-space: nowrap`.
fn single_line_width() -> usize {
    let shot = render(
        "single",
        CANVAS,
        1.0,
        BASE,
        &text_clip("white-space: nowrap"),
    );
    let bands = shot.bands();
    assert_eq!(bands.len(), 1);
    let (left, right) = shot.extent(bands[0]);
    right - left + 1
}

#[test]
fn text_wraps_at_an_explicit_width_and_nowrap_overrides_it() {
    let full = single_line_width();
    let narrow = format!("width: {}px", full * 55 / 100);
    let wide = format!("width: {}px", full * 110 / 100);
    let lines =
        |name: &str, style: &str| render(name, CANVAS, 1.0, BASE, &text_clip(style)).bands();
    assert!(lines("narrow", &narrow).len() >= 2, "narrow box wraps");
    assert_eq!(lines("wide", &wide).len(), 1, "wide box keeps one line");
    assert_eq!(
        lines("nowrap", &format!("{narrow}; white-space: nowrap")).len(),
        1,
        "nowrap ignores the box width"
    );
}

#[test]
fn max_width_caps_an_absolute_text_box() {
    let full = single_line_width();
    let limit = full * 55 / 100;
    let shot = render(
        "maxw",
        CANVAS,
        1.0,
        BASE,
        &text_clip(&format!("max-width: {limit}px")),
    );
    let bands = shot.bands();
    assert!(bands.len() >= 2);
    for band in bands {
        let (_, right) = shot.extent(band);
        assert!(right < limit + 2, "line ends at {right}, limit {limit}");
    }
}

#[test]
fn max_width_caps_a_shrink_to_fit_flex_item() {
    let full = single_line_width();
    let limit = full * 55 / 100;
    let css = format!(
        "{BASE} .col {{ left: 0; top: 0; width: 100%; height: 100%; display: flex;
           flex-direction: column; align-items: flex-start }}
         .t {{ max-width: {limit}px }}"
    );
    let body = format!(
        r#"<div class="clip col" data-start="0" data-duration="1">
             <div class="clip t" data-start="0" data-duration="1">{WORDS}</div></div>"#
    );
    let shot = render("flexmax", CANVAS, 1.0, &css, &body);
    let bands = shot.bands();
    assert!(bands.len() >= 2);
    assert!(bands.iter().all(|&b| shot.extent(b).1 < limit + 2));
}

#[test]
fn line_breaks_do_not_depend_on_output_scale() {
    let full = single_line_width();
    let style = format!("width: {}px", full * 70 / 100);
    let at = |scale: f32| {
        render("scale", CANVAS, scale, BASE, &text_clip(&style))
            .bands()
            .len()
    };
    assert!(at(1.0) >= 2);
    assert_eq!(at(1.0), at(0.5));
    assert_eq!(at(1.0), at(2.0));
}

#[test]
fn css_scale_does_not_rewrap_text() {
    let full = single_line_width();
    let width = full * 70 / 100;
    let lines = |transform: &str| {
        let style = format!("left: 40px; top: 20px; width: {width}px; {transform}");
        render("zoom", CANVAS, 1.0, BASE, &text_clip(&style))
            .bands()
            .len()
    };
    let plain = lines("transform: scale(1)");
    assert!(plain >= 2);
    assert_eq!(plain, lines("transform: scale(1.3)"));
    assert_eq!(plain, lines("transform: scale(0.8)"));
}

#[test]
fn text_align_centers_each_line_in_the_box() {
    let full = single_line_width();
    let width = full * 80 / 100;
    let style = format!("left: 20px; width: {width}px; text-align: center");
    let shot = render("center", CANVAS, 1.0, BASE, &text_clip(&style));
    let bands = shot.bands();
    assert!(bands.len() >= 2);
    let middle = 20.0 + width as f32 / 2.0;
    for band in bands {
        let (left, right) = shot.extent(band);
        let center = (left + right) as f32 / 2.0;
        assert!(
            (center - middle).abs() < 3.0,
            "line centered at {center}, box at {middle}"
        );
    }
}

#[test]
fn line_height_sets_the_distance_between_lines() {
    let full = single_line_width();
    let width = full * 55 / 100;
    let pitch = |line_height: &str| {
        let style = format!("width: {width}px; line-height: {line_height}");
        let bands = render("pitch", CANVAS, 1.0, BASE, &text_clip(&style)).bands();
        assert!(bands.len() >= 2);
        bands[1].0 as i32 - bands[0].0 as i32
    };
    assert!((pitch("30px") - 30).abs() <= 1);
    assert!((pitch("60px") - 60).abs() <= 1);
    assert!(
        (pitch("2") - 40).abs() <= 1,
        "unitless values scale with the font size"
    );
}

#[test]
fn flex_column_reserves_the_wrapped_height() {
    // Three words, each on its own line (the box is narrower than any word), 30px apart.
    let css = ".col { left: 0; top: 0; width: 100%; height: 100%; display: flex;
               flex-direction: column; align-items: flex-start }
             .t { font-size: 20px; line-height: 30px; max-width: 10px }
             .mark { width: 20px; height: 10px; background: #f00 }";
    let body = r#"<div class="clip col" data-start="0" data-duration="1">
         <div class="clip t" data-start="0" data-duration="1">HHHH HHHH HHHH</div>
         <div class="clip mark" data-start="0" data-duration="1"></div></div>"#;
    let shot = render("column", CANVAS, 1.0, css, body);
    assert_ne!(shot.at(5, 88), [255, 0, 0], "text block ends at y=90");
    assert_eq!(
        shot.at(5, 91),
        [255, 0, 0],
        "the next item starts right below it"
    );
    assert_eq!(shot.at(5, 99), [255, 0, 0]);
    assert_ne!(shot.at(5, 101), [255, 0, 0]);
}

#[test]
fn flex_grow_rewraps_text_at_its_final_width() {
    let css = ".row { left: 0; top: 0; width: 160px; height: 200px; display: flex;
                align-items: flex-start }
              .t { font-size: 16px; line-height: 1.5; flex: 1 }";
    let body = format!(
        r#"<div class="clip row" data-start="0" data-duration="1">
             <div class="clip t" data-start="0" data-duration="1">{WORDS}</div>
             <div class="clip t" data-start="0" data-duration="1">{WORDS}</div></div>"#
    );
    let shot = render("grow", CANVAS, 1.0, css, &body);
    // Each text gets 80px; four 4-letter words cannot share fewer than three lines there.
    assert!(shot.bands().len() >= 3);
    let (_, right) = shot.extent((0, shot.height));
    assert!(
        right < 160 + 2,
        "text stays inside the row, right edge {right}"
    );
}

#[test]
fn text_wrapping_errors_are_explicit() {
    let cases = [
        (".t { white-space: pre }", "white-space"),
        (".t { line-height: normal }", "line-height"),
        (".t { line-height: 0 }", "line-height"),
        (".t { max-width: -4px }", "max-width"),
    ];
    for (i, (css, expected)) in cases.iter().enumerate() {
        let html = format!(
            r#"<html><head><style>{css}</style></head><body>
            <main data-composition-id="e" data-width="64" data-height="64" data-duration="1">
            <div class="clip t" data-start="0" data-duration="1">Hi</div></main></body></html>"#
        );
        let path = scratch(&format!("err{i}")).join("e.html");
        std::fs::write(&path, html).unwrap();
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(error.contains(expected), "{css}: {error}");
    }
}
