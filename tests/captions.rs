//! `data-hf-captions`: cues appear only while spoken, the spoken word takes the highlight color,
//! and bad inputs fail with explicit errors.
mod common;

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::{scratch, Shot};

const FPS: u32 = 10;

fn project(transcript_name: &str, transcript: &str, attrs: &str) -> std::path::PathBuf {
    let dir = scratch("captions");
    std::fs::write(dir.join(transcript_name), transcript).unwrap();
    let html = format!(
        r#"<!doctype html><html><body>
<div id="root" style="background:#000000" data-composition-id="c" data-width="640" data-height="360" data-duration="4">
<div id="subs" class="clip" data-start="0" data-duration="4" data-track-index="0"
     style="left:20px;top:100px;width:600px;height:80px;color:#ffffff;font-size:48px;font-weight:700;text-align:center"
     data-hf-captions="{transcript_name}" {attrs}></div>
</div></body></html>"#
    );
    let path = dir.join("index.html");
    std::fs::write(&path, html).unwrap();
    path
}

fn shot_at(path: &std::path::Path, seconds: f64) -> Shot {
    let composition = parse_file(path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let (width, height) = renderer.dimensions();
    let pixels = renderer
        .render_frame((seconds * FPS as f64) as usize, FPS)
        .unwrap()
        .to_vec();
    Shot { pixels, width: width as usize, height: height as usize }
}

fn is_red(p: [u8; 3]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}

fn is_white(p: [u8; 3]) -> bool {
    p.iter().all(|&c| c > 200)
}

const WORDS: &str = r#"[
  {"text":"One","start":0.5,"end":1.0},
  {"text":"two.","start":1.0,"end":1.5},
  {"text":"Three","start":2.5,"end":3.0}]"#;

#[test]
fn cues_follow_the_transcript_and_highlight_the_spoken_word() {
    let path = project("t.json", WORDS, r##"data-hf-captions-highlight="#ff0000""##);
    // Before the first word and in the gap between cues nothing is drawn.
    assert_eq!(shot_at(&path, 0.2).count(|p| is_white(p) || is_red(p)), 0);
    assert_eq!(shot_at(&path, 2.0).count(|p| is_white(p) || is_red(p)), 0);
    // While "One" is spoken the cue is visible with a highlighted word and white context.
    let first = shot_at(&path, 0.7);
    assert!(first.count(is_red) > 100, "spoken word should be highlighted");
    assert!(first.count(is_white) > 100, "other words keep the base color");
    // Moving to the next word moves the highlight to the right.
    let second = shot_at(&path, 1.2);
    let (l1, _, _, _) = first.bbox(is_red).unwrap();
    let (l2, _, _, _) = second.bbox(is_red).unwrap();
    assert!(l2 > l1 + 40, "highlight should move right: {l1} -> {l2}");
    // The last cue ends after its linger.
    assert!(shot_at(&path, 3.1).count(|p| is_red(p) || is_white(p)) > 0);
    assert_eq!(shot_at(&path, 3.6).count(|p| is_red(p) || is_white(p)), 0);
}

#[test]
fn plain_mode_never_uses_the_highlight_color() {
    let path = project(
        "t.json",
        WORDS,
        r##"data-hf-captions-mode="plain" data-hf-captions-highlight="#ff0000""##,
    );
    let frame = shot_at(&path, 0.7);
    assert_eq!(frame.count(is_red), 0);
    assert!(frame.count(is_white) > 100);
}

#[test]
fn srt_and_offset_are_supported() {
    let srt = "1\n00:00:00,500 --> 00:00:01,500\nHello there\n";
    let path = project("t.srt", srt, r#"data-hf-captions-offset="1""#);
    assert_eq!(shot_at(&path, 0.7).count(is_white), 0);
    assert!(shot_at(&path, 1.7).count(is_white) > 100);
}

#[test]
fn bad_inputs_report_explicit_errors() {
    let missing = project("t.json", WORDS, r#"data-hf-captions-mode="spin""#);
    let err = format!("{:#}", parse_file(&missing).unwrap_err());
    assert!(err.contains("data-hf-captions-mode"), "{err}");

    let unordered = project(
        "t.json",
        r#"[{"text":"b","start":2,"end":3},{"text":"a","start":1,"end":1.5}]"#,
        "",
    );
    let err = format!("{:#}", parse_file(&unordered).unwrap_err());
    assert!(err.contains("sort the transcript"), "{err}");

    let orphan = project("t.json", WORDS, "");
    let html = std::fs::read_to_string(&orphan)
        .unwrap()
        .replace(r#"data-hf-captions="t.json""#, r#"data-hf-captions-pop="1.2""#);
    std::fs::write(&orphan, html).unwrap();
    let err = format!("{:#}", parse_file(&orphan).unwrap_err());
    assert!(err.contains("requires data-hf-captions"), "{err}");
}
