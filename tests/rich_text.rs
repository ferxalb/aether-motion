//! Inline rich text (`<span>`, `<strong>`, `<b>`, `<code>`), `background-clip: text`, and
//! `-webkit-text-stroke` / `-webkit-text-fill-color`.

mod common;

use aether_motion::model::Content;
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;

fn write(name: &str, body: &str) -> std::path::PathBuf {
    let html = format!(
        r#"<div id="root" data-composition-id="x" data-width="400" data-height="100" data-duration="1" style="background:#000000">{body}</div>"#
    );
    let path = scratch(name).join("c.html");
    std::fs::write(&path, html).unwrap();
    path
}

fn frame(name: &str, body: &str) -> Vec<u8> {
    let composition = parse_file(&write(name, body)).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    renderer.render_at(0.0).unwrap().to_vec()
}

fn error(name: &str, body: &str) -> String {
    format!("{:#}", parse_file(&write(name, body)).unwrap_err())
}

/// Columns `[x0, x1)` holding any pixel whose red, green, and blue satisfy `test`.
fn columns(frame: &[u8], test: impl Fn([u8; 3]) -> bool) -> Option<(usize, usize)> {
    let mut found: Option<(usize, usize)> = None;
    for y in 0..100 {
        for x in 0..400 {
            let i = (y * 400 + x) * 4;
            if test([frame[i], frame[i + 1], frame[i + 2]]) {
                found = Some(found.map_or((x, x + 1), |(a, b)| (a.min(x), b.max(x + 1))));
            }
        }
    }
    found
}

const TEXT: &str = r#"class="clip" data-start="0" data-duration="1" style="position:absolute; left:0; top:10px; width:400px; font-size:40px; color:#ffffff""#;

#[test]
fn spans_join_the_text_without_extra_spaces() {
    let composition = parse_file(&write(
        "rt_join",
        &format!(
            r#"<div {TEXT}> Make<span style="color:#ff0000">s</span>  <b>bold</b> <br> next </div>"#
        ),
    ))
    .unwrap();
    let element = composition
        .elements
        .iter()
        .find(|e| matches!(e.content, Content::Text(_)))
        .unwrap();
    let Content::Text(text) = &element.content else {
        unreachable!()
    };
    assert_eq!(text, "Makes bold\nnext");
    assert_eq!(element.spans.len(), 2);
    assert_eq!((element.spans[0].start, element.spans[0].end), (4, 5));
    assert_eq!(element.spans[1].style.font_weight, Some(700));
    // The inline children are runs, not elements of their own.
    assert_eq!(composition.elements.len(), 2);
}

#[test]
fn a_span_recolors_only_its_run() {
    let f = frame(
        "rt_color",
        &format!(r#"<div {TEXT}>white <span style="color:#ff0000">red</span></div>"#),
    );
    let red = columns(&f, |[r, g, b]| r > 200 && g < 40 && b < 40).unwrap();
    let white = columns(&f, |[r, g, b]| r > 200 && g > 200 && b > 200).unwrap();
    assert!(white.1 <= red.0, "white {white:?} comes before red {red:?}");
}

#[test]
fn bold_runs_are_wider() {
    let plain = frame(
        "rt_plain",
        &format!(r#"<div {TEXT}>abc <span>wide</span></div>"#),
    );
    let bold = frame(
        "rt_bold",
        &format!(r#"<div {TEXT}>abc <strong>wide</strong></div>"#),
    );
    let any = |[r, _, _]: [u8; 3]| r > 100;
    assert!(columns(&bold, any).unwrap().1 > columns(&plain, any).unwrap().1);
}

#[test]
fn plain_text_is_unchanged_by_an_unstyled_span() {
    let plain = frame("rt_same_a", &format!(r#"<div {TEXT}>same text</div>"#));
    let spanned = frame(
        "rt_same_b",
        &format!(r#"<div {TEXT}>same <span>text</span></div>"#),
    );
    assert_eq!(plain, spanned);
}

#[test]
fn background_clip_text_fills_the_glyphs_with_the_gradient() {
    let f = frame(
        "rt_clip",
        r#"<div class="clip" data-start="0" data-duration="1" style="position:absolute; left:0; top:10px; width:400px; height:80px; font-size:60px; font-weight:700; color:transparent; background:linear-gradient(90deg, #ff0000, #0000ff); background-clip:text">MMMMMMMM</div>"#,
    );
    // The box is not painted; glyphs on the left are red and on the right blue.
    assert_eq!(&f[(5 * 400 + 395) * 4..(5 * 400 + 395) * 4 + 3], &[0, 0, 0]);
    let reddish = columns(&f, |[r, _, b]| r > 150 && b < 100).unwrap();
    let bluish = columns(&f, |[r, _, b]| b > 150 && r < 100).unwrap();
    assert!(reddish.0 < bluish.0);
}

#[test]
fn text_stroke_outlines_transparent_glyphs() {
    let f = frame(
        "rt_stroke",
        r#"<div class="clip" data-start="0" data-duration="1" style="position:absolute; left:0; top:0; width:400px; font-size:90px; font-weight:700; color:transparent; -webkit-text-stroke:3px #00ff00">O</div>"#,
    );
    let ring = columns(&f, |[r, g, b]| g > 200 && r < 50 && b < 50).unwrap();
    // The inside of the O stays empty: scan its middle row.
    let row = (ring.0..ring.1)
        .map(|x| f[(45 * 400 + x) * 4 + 1])
        .collect::<Vec<_>>();
    assert!(row.iter().any(|&g| g > 200));
    assert!(row[row.len() / 2] < 20, "{row:?}");
}

#[test]
fn rejects_box_properties_on_inline_runs() {
    assert!(error(
        "rt_pad",
        &format!(r#"<div {TEXT}>a <span style="padding:4px">b</span></div>"#)
    )
    .contains("padding"));
    assert!(error(
        "rt_size",
        &format!(r#"<div {TEXT}>a <span style="font-size:10px">b</span></div>"#)
    )
    .contains("font-size"));
    assert!(error(
        "rt_container",
        r#"<div class="clip" data-start="0" data-duration="1" style="background:linear-gradient(90deg,#fff,#000); background-clip:text"><div>x</div></div>"#
    )
    .contains("text of its own"));
}

#[test]
fn a_span_without_sibling_text_stays_an_element() {
    let composition = parse_file(&write(
        "rt_legacy",
        r#"<div class="clip" data-start="0" data-duration="1"><span>alone</span></div>"#,
    ))
    .unwrap();
    assert_eq!(composition.elements.len(), 3);
    assert!(composition.elements.iter().all(|e| e.spans.is_empty()));
}
