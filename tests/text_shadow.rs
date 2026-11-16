//! `text-shadow`: offset, blur, inheritance, scale, rotation, determinism, and errors.

mod common;

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::{render, scratch, Shot};

const CLIP: &str = r#"data-start="0" data-duration="1""#;
const CANVAS: (usize, usize) = (240, 120);
const BASE: &str =
    ".t { left: 30px; top: 30px; font-size: 40px; color: #fff; white-space: nowrap }";

fn white(p: [u8; 3]) -> bool {
    p.iter().all(|&c| c > 200)
}

/// Red that is not covered by the (white) text.
fn red(p: [u8; 3]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

fn shot(name: &str, scale: f32, style: &str) -> Shot {
    let body = format!(r#"<div class="clip t" style="{style}" {CLIP}>HHH</div>"#);
    render(name, CANVAS, scale, BASE, &body)
}

#[test]
fn a_hard_shadow_is_the_text_shifted_by_its_offset() {
    let plain = shot("plain", 1.0, "");
    let shadowed = shot("hard", 1.0, "text-shadow: 6px 5px 0 #f00");
    let text = plain.bbox(white).unwrap();
    assert_eq!(
        shadowed.bbox(white),
        Some(text),
        "the text itself does not move"
    );
    let (_, _, right, bottom) = shadowed.bbox(red).unwrap();
    assert!(
        right.abs_diff(text.2 + 6) <= 1,
        "right edge {right} vs {}",
        text.2
    );
    assert!(
        bottom.abs_diff(text.3 + 5) <= 1,
        "bottom edge {bottom} vs {}",
        text.3
    );
}

#[test]
fn blur_spreads_the_shadow_beyond_the_hard_edge() {
    let hard = shot("blur0", 1.0, "text-shadow: 0 0 0 #f00");
    let soft = shot("blur8", 1.0, "text-shadow: 0 0 8px #f00");
    let reddish = |p: [u8; 3]| p[0] > 12 && p[1] < 60 && p[2] < 60;
    // A hard shadow without offset hides entirely behind the text.
    let (hl, ht, hr, hb) = hard.bbox(white).unwrap();
    let (sl, st, sr, sb) = soft.bbox(reddish).unwrap();
    assert!(sl + 4 <= hl && st + 4 <= ht && sr >= hr + 4 && sb >= hb + 4);
    assert!(soft.count(reddish) > hard.count(reddish));
}

#[test]
fn shadow_offset_and_blur_scale_with_the_output() {
    let at1 = shot("s1", 1.0, "text-shadow: 6px 5px 0 #f00");
    let at2 = shot("s2", 2.0, "text-shadow: 6px 5px 0 #f00");
    let (t1, t2) = (at1.bbox(white).unwrap(), at2.bbox(white).unwrap());
    let (r1, r2) = (at1.bbox(red).unwrap(), at2.bbox(red).unwrap());
    assert!((r2.2 as i32 - t2.2 as i32).abs_diff(2 * (r1.2 as i32 - t1.2 as i32)) <= 2);
    assert!((r2.3 as i32 - t2.3 as i32).abs_diff(2 * (r1.3 as i32 - t1.3 as i32)) <= 2);
}

#[test]
fn text_shadow_is_inherited_and_none_resets_it() {
    let css = format!(
        "{BASE} .col {{ left: 0; top: 0; width: 100%; height: 100%; display: flex; flex-direction: column;
           text-shadow: 6px 6px 0 #f00 }} .a, .b {{ position: relative; left: 0; top: 0 }}"
    );
    let body = format!(
        r#"<div class="clip col" {CLIP}>
             <div class="clip a" {CLIP}>HH</div>
             <div class="clip b" style="text-shadow: none" {CLIP}>HH</div></div>"#
    );
    let shot = render("inherit", CANVAS, 1.0, &css, &body);
    let text = shot.bbox(white).unwrap();
    let middle = (text.1 + text.3) / 2;
    let upper = shot.count(red);
    assert!(upper > 0, "the inherited shadow is painted");
    // The second line sits in the lower half and has no red pixels near it.
    let lower_red = (0..shot.width)
        .flat_map(|x| (middle + 6..shot.height).map(move |y| (x, y)))
        .filter(|&(x, y)| red(shot.at(x, y)))
        .count();
    assert_eq!(
        lower_red, 0,
        "`none` removes the shadow from the second line"
    );
}

#[test]
fn a_rotated_text_keeps_a_shadow_that_leaves_its_box() {
    // The shadow lands 40px below the box; a clipped layer would lose it.
    let css =
        ".t { left: 60px; top: 10px; width: 120px; height: 30px; font-size: 24px; color: #fff;
                     white-space: nowrap; transform: rotate(4deg); text-shadow: 0 40px 0 #f00 }";
    let body = format!(r#"<div class="clip t" {CLIP}>HHH</div>"#);
    let shot = render("rotated", CANVAS, 1.0, css, &body);
    let (_, _, _, bottom) = shot.bbox(red).expect("shadow is painted");
    assert!(
        bottom > 10 + 30 + 20,
        "shadow reaches y={bottom}, below the 40px box"
    );
}

#[test]
fn shadows_do_not_depend_on_render_order() {
    let html = r#"<html><head><style>
        @keyframes rise { from { transform: translate(0, 30px); opacity: 0 } to { transform: translate(0, 0); opacity: 1 } }
        .t { left: 20px; top: 20px; width: 160px; font-size: 28px; line-height: 1.3; text-align: center;
             color: #fff; text-shadow: 3px 4px 6px #f00; animation: rise 0.8s linear both }
        </style></head><body>
        <main data-composition-id="o" data-width="240" data-height="120" data-duration="1">
        <div class="clip t" data-start="0" data-duration="1">HHHH HHHH HHHH</div></main></body></html>"#;
    let path = scratch("order").join("o.html");
    std::fs::write(&path, html).unwrap();
    let composition = parse_file(&path).unwrap();
    let mut warm = Renderer::new(&composition, 1.0).unwrap();
    for t in [0.1, 0.35, 0.6] {
        warm.render_at(t).unwrap();
    }
    let after_others = warm.render_at(0.5).unwrap().to_vec();
    let mut fresh = Renderer::new(&composition, 1.0).unwrap();
    assert_eq!(
        after_others,
        fresh.render_at(0.5).unwrap(),
        "same bytes in any order"
    );
    let mut forked = fresh.fork().unwrap();
    assert_eq!(
        after_others,
        forked.render_at(0.5).unwrap(),
        "and in a forked renderer"
    );
}

#[test]
fn text_shadow_errors_are_explicit() {
    let cases = [
        ("text-shadow: 1px 1px #000, 2px 2px #fff", "several shadows"),
        ("text-shadow: inset 1px 1px #000", "inset"),
        ("text-shadow: 1px 1px 2px 3px #000", "no spread"),
        ("text-shadow: 1px 1px -2px #000", "blur"),
        ("text-shadow: 1px", "expected"),
    ];
    for (i, (style, expected)) in cases.iter().enumerate() {
        let html = format!(
            r#"<html><body><main data-composition-id="e" data-width="64" data-height="64" data-duration="1">
            <div class="clip" style="{style}" {CLIP}>Hi</div></main></body></html>"#
        );
        let path = scratch(&format!("err{i}")).join("e.html");
        std::fs::write(&path, html).unwrap();
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(error.contains(expected), "{style}: {error}");
    }
}
