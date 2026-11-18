//! Typography added in 0.1.5: `font-kerning: normal` (GPOS pair kerning, also across the letters of
//! `data-hf-split="chars"`), `font-style: italic` with a synthetic oblique when no italic face
//! exists, `<em>`/`<i>` runs, and multi-line `data-hf-split` with `<br>`.

mod common;

use aether_motion::model::Content;
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;

const W: usize = 600;
const H: usize = 200;

fn write(name: &str, body: &str) -> std::path::PathBuf {
    let html = format!(
        r#"<div data-composition-id="t" data-width="{W}" data-height="{H}" data-duration="2" style="background:#000000">{body}</div>"#
    );
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    path
}

fn frame(name: &str, body: &str, time: f64) -> Vec<u8> {
    let composition = parse_file(&write(name, body)).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    renderer.render_at(time).unwrap().to_vec()
}

/// Bounding box `(x0, y0, x1, y1)` of bright pixels.
fn ink(frame: &[u8]) -> Option<(usize, usize, usize, usize)> {
    let mut found: Option<(usize, usize, usize, usize)> = None;
    for y in 0..H {
        for x in 0..W {
            if frame[(y * W + x) * 4] > 128 {
                found = Some(found.map_or((x, y, x + 1, y + 1), |(a, b, c, d)| {
                    (a.min(x), b.min(y), c.max(x + 1), d.max(y + 1))
                }));
            }
        }
    }
    found
}

const TEXT: &str =
    "position:absolute; left:10px; top:20px; font-size:80px; color:#ffffff; white-space:nowrap";

#[test]
fn kerning_tightens_pairs_only_when_asked() {
    let width = |name: &str, extra: &str| {
        let body = format!(
            r#"<div class="clip" data-start="0" data-duration="2" style="{TEXT};{extra}">AVATAR</div>"#
        );
        let (x0, _, x1, _) = ink(&frame(name, &body, 0.5)).unwrap();
        x1 - x0
    };
    let plain = width("kern_auto", "");
    let none = width("kern_none", "font-kerning:none");
    let kerned = width("kern_normal", "font-kerning:normal");
    // `auto` keeps the unkerned layout of earlier versions.
    assert_eq!(plain, none);
    // A-V, V-A, A-T, and T-A each tighten by about 3 to 5 px at 80 px.
    assert!(kerned + 10 < plain, "kerned {kerned} vs plain {plain}");
}

#[test]
fn split_letters_kern_like_the_whole_word() {
    let whole = format!(
        r#"<div class="clip" data-start="0" data-duration="2" style="{TEXT}; font-kerning:normal">AVA</div>"#
    );
    let split = format!(
        r#"<div class="clip" data-start="0" data-duration="2" data-hf-split="chars" data-hf-split-from="fade" data-hf-split-duration="0.1" data-hf-stagger="0" style="{TEXT}; font-kerning:normal">AVA</div>"#
    );
    let a = ink(&frame("split_kern_whole", &whole, 1.0)).unwrap();
    let b = ink(&frame("split_kern_pieces", &split, 1.0)).unwrap();
    // Pieces are placed at whole pixels of the same advances, so the extents agree within a pixel.
    assert!(
        a.0.abs_diff(b.0) <= 1 && a.2.abs_diff(b.2) <= 1,
        "{a:?} vs {b:?}"
    );
}

#[test]
fn italic_without_an_italic_face_is_slanted_in_place() {
    let body = |style: &str| {
        format!(
            r#"<div class="clip" data-start="0" data-duration="2" style="{TEXT};{style}">I</div>"#
        )
    };
    let upright = frame("italic_off", &body(""), 0.5);
    let italic = frame("italic_on", &body("font-style:italic"), 0.5);
    let column = |frame: &[u8], y: usize| {
        let lit: Vec<usize> = (0..W).filter(|&x| frame[(y * W + x) * 4] > 128).collect();
        (lit[0] + lit[lit.len() - 1]) as f32 / 2.0
    };
    let (_, top, _, bottom) = ink(&upright).unwrap();
    let (rise_upright, rise_italic) = (
        column(&upright, top + 2) - column(&upright, bottom - 3),
        column(&italic, top + 2) - column(&italic, bottom - 3),
    );
    // The stem of "I" is vertical upright and leans right by tan(14°) when slanted.
    assert!(rise_upright.abs() < 1.0, "{rise_upright}");
    let expected = 0.2493 * (bottom - 3 - (top + 2)) as f32;
    assert!(
        (rise_italic - expected).abs() < 2.0,
        "{rise_italic} vs {expected}"
    );
    // The ink sits on the same baseline.
    let (_, _, _, italic_bottom) = ink(&italic).unwrap();
    assert!(italic_bottom.abs_diff(bottom) <= 1);
}

#[test]
fn em_and_i_runs_and_elements_are_italic() {
    let path = write(
        "em_runs",
        &format!(
            r#"<div class="clip" data-start="0" data-duration="2" style="{TEXT}">a <em>b</em> <i>c</i></div><em class="clip" data-start="0" data-duration="2" style="position:absolute; top:120px">d</em>"#
        ),
    );
    let composition = parse_file(&path).unwrap();
    let rich = composition
        .elements
        .iter()
        .find(|e| matches!(&e.content, Content::Text(t) if t.starts_with('a')))
        .unwrap();
    assert_eq!(rich.spans.len(), 2);
    assert!(rich.spans.iter().all(|s| s.style.font_italic == Some(true)));
    let em = composition.elements.iter().find(|e| e.tag == "em").unwrap();
    assert_eq!(em.style.font_italic, Some(true));
    Renderer::new(&composition, 1.0)
        .unwrap()
        .render_at(0.5)
        .unwrap();
}

#[test]
fn split_text_with_br_stacks_lines_and_continues_the_stagger() {
    let path = write(
        "split_lines",
        r#"<h1 class="clip" data-start="0" data-duration="2" data-hf-split="words" data-hf-stagger="0.2" data-hf-split-duration="0.2" data-hf-split-from="fade" style="position:absolute; left:0; top:0; width:600px; font-size:40px; color:#ffffff; text-align:center">one two<br>three<br></h1>"#,
    );
    let composition = parse_file(&path).unwrap();
    let words: Vec<_> = composition
        .elements
        .iter()
        .filter(|e| matches!(&e.content, Content::Text(t) if !t.is_empty()))
        .collect();
    assert_eq!(words.len(), 3, "a trailing <br> adds no line");
    // "three" (third piece) enters after the two words of the first line.
    assert_eq!(words[2].motion.pose(0.0).opacity, 0.0);
    assert_eq!(words[2].motion.pose(0.6).opacity, 1.0);
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let pixels = renderer.render_at(1.5).unwrap().to_vec();
    let rows: Vec<usize> = (0..H)
        .filter(|&y| (0..W).any(|x| pixels[(y * W + x) * 4] > 128))
        .collect();
    let (first, last) = (rows[0], rows[rows.len() - 1]);
    // Two lines, 40 px apart (line-height 1), centered.
    assert!(last - first > 50, "{first}..{last}");
    let centers: Vec<f32> = [first + 5, last - 5]
        .iter()
        .map(|&y| {
            let lit: Vec<usize> = (0..W).filter(|&x| pixels[(y * W + x) * 4] > 128).collect();
            (lit[0] + lit[lit.len() - 1]) as f32 / 2.0
        })
        .collect();
    assert!(
        centers.iter().all(|c| (c - 300.0).abs() < 20.0),
        "{centers:?}"
    );
    // Child elements other than <br> are still refused.
    let error = parse_file(&write(
        "split_child",
        r#"<h1 class="clip" data-start="0" data-duration="2" data-hf-split="words">a <span>b</span></h1>"#,
    ))
    .unwrap_err();
    assert!(format!("{error:#}").contains("<br>"), "{error:#}");
}

#[test]
fn inline_runs_reject_font_kerning() {
    let error = parse_file(&write(
        "kern_run",
        &format!(r#"<div class="clip" data-start="0" data-duration="2" style="{TEXT}">a <span style="font-kerning:normal">b</span></div>"#),
    ))
    .unwrap_err();
    assert!(format!("{error:#}").contains("font-kerning"), "{error:#}");
}

#[test]
fn split_rows_keep_blank_lines_and_inherit_text_align() {
    let path = write(
        "split_inherit",
        r#"<div class="clip" data-start="0" data-duration="2" style="position:absolute; left:0; top:0; width:600px; text-align:center"><h1 data-hf-split="words" data-hf-stagger="0" data-hf-split-duration="0.1" data-hf-split-from="fade" style="font-size:40px; color:#ffffff">a<br><br>b</h1></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let h1 = composition
        .elements
        .iter()
        .position(|e| e.tag == "h1" && e.style.column)
        .unwrap();
    // Three rows: "a", the blank line from the second break, and "b".
    assert_eq!(composition.elements[h1].children.len(), 3);
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let pixels = renderer.render_at(1.0).unwrap().to_vec();
    let (x0, y0, x1, y1) = ink(&pixels).unwrap();
    // Centered by the parent's text-align, and spanning three 40 px lines.
    assert!(((x0 + x1) as f32 / 2.0 - 300.0).abs() < 20.0, "{x0}..{x1}");
    assert!(y1 - y0 > 80, "{y0}..{y1}");
}
