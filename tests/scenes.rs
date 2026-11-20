//! External scene compilation, local clocks, variable scope, and deterministic native encoding.

mod common;

use aether_motion::audio::{mix::mix, SAMPLE_RATE};
use aether_motion::encode::{assemble_parts, encode_native, render_part, EncodeOptions};
use aether_motion::model::{Content, Element};
use aether_motion::parse::{parse_file, parse_file_with, values_from_json};
use aether_motion::render::Renderer;
use common::{scratch, write_wav};
use std::fs;
use std::path::{Path, PathBuf};

fn page(dir: &Path, name: &str, extra: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        format!(
            r##"<main data-composition-id="test" data-width="96" data-height="54" data-duration="4" {extra}>{body}</main>"##
        ),
    )
    .unwrap();
    path
}

const VARIABLES: &str = r##"data-composition-variables='[
    {"id":"word","type":"string","label":"Word","default":"Default"},
    {"id":"tone","type":"color","label":"Tone","default":"#0f0"},
    {"id":"picture","type":"image","label":"Picture","default":"tile.svg"}
]'"##;

fn pixel(renderer: &mut Renderer, time: f64, x: usize, y: usize) -> [u8; 4] {
    let index = (y * renderer.dimensions().0 as usize + x) * 4;
    renderer.render_at(time).unwrap()[index..index + 4]
        .try_into()
        .unwrap()
}

#[test]
fn instances_keep_nested_clocks_variables_ids_and_asset_paths_local() {
    let dir = scratch("scene-scope");
    fs::create_dir(dir.join("shots")).unwrap();
    fs::write(
        dir.join("shots/tile.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><path fill="#0ff" d="M0 0h4v4H0z"/></svg>"##,
    )
    .unwrap();
    page(
        &dir,
        "shots/leaf.html",
        VARIABLES,
        r##"<style>#plate {background:var(--tone);width:12px;height:12px}</style>
        <div id="plate" data-start="0.25" data-duration="1.7"
             data-hf-keyframes='[{"at":0,"x":0},{"at":1,"x":16}]'></div>
        <img id="tile" data-var-src="picture" style="left:42px;width:4px;height:4px">
        <p id="word" data-start="plate" data-var-text="word"
           style="width:60px;top:25px;font-size:8px" data-hf-split="words" data-hf-split-wrap></p>"##,
    );
    page(
        &dir,
        "middle.html",
        "",
        r##"<div id="leaf" data-composition-src="shots/leaf.html" data-start="0.5"
                 data-duration="2" data-track-index="0"
                 data-variable-values='{"word":"Inside","tone":"#00f"}'></div>"##,
    );
    let entry = page(
        &dir,
        "index.html",
        VARIABLES,
        r##"<style>#plate {background:#fff}</style>
        <p id="word" data-var-text="word" style="top:41px;font-size:6px"></p>
        <div id="first" data-composition-src="shots/leaf.html" data-start="0.5"
             data-duration="2" data-track-index="1" data-variable-values='{"word":"First","tone":"#f00"}'></div>
        <div id="second" data-composition-src="middle.html" data-start="1.5"
             data-duration="2.5" data-track-index="2"></div>"##,
    );
    let root_values = values_from_json(r##"{"word":"Outside","tone":"#fff"}"##).unwrap();
    let c = parse_file_with(&entry, &root_values).unwrap();
    let node = |id: &str| -> &Element { &c.elements[c.ids[id]] };
    assert!(matches!(&node("word").content, Content::Text(text) if text == "Outside"));
    assert_ne!(c.ids["first::word"], c.ids["second::leaf::word"]);
    assert_eq!(node("first::word").start, 2.45);
    assert_eq!(node("second::leaf::word").start, 3.95);
    for (id, text) in [("first::word", "First"), ("second::leaf::word", "Inside")] {
        let row = &c.elements[node(id).children[0]];
        assert!(row.children.iter().any(|&idx| {
            matches!(&c.elements[idx].content, Content::Text(piece) if piece == text)
        }));
    }
    assert!(matches!(
        &node("second::leaf::tile").content,
        Content::Image(path) if path == &dir.join("shots/tile.svg")
    ));
    let mut r = Renderer::new(&c, 1.0).unwrap();
    assert_eq!(pixel(&mut r, 0.74, 3, 2), [0, 0, 0, 255]);
    assert_eq!(pixel(&mut r, 0.75, 3, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&mut r, 1.25, 3, 2), [0, 0, 0, 255]);
    assert_eq!(pixel(&mut r, 1.25, 10, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&mut r, 2.25, 3, 2), [0, 0, 255, 255]);
    assert_eq!(pixel(&mut r, 2.25, 18, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&mut r, 2.5, 18, 2), [0, 0, 0, 255]);
    assert_eq!(pixel(&mut r, 2.5, 43, 2), [0, 255, 255, 255]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_invalid_imports_with_scene_context() {
    let dir = scratch("scene-errors");
    page(&dir, "leaf.html", VARIABLES, "");
    page(&dir, "finished.html", "data-hf-vignette='0.2'", "");
    fs::write(
        dir.join("small.html"),
        "<div data-composition-id='small' data-width='32' data-height='32' data-duration='2'></div>",
    )
    .unwrap();
    let check = |src: &str, attrs: &str, expected: &str| {
        let entry = page(
            &dir,
            "index.html",
            "",
            &format!("<div id='shot' data-composition-src='{src}' {attrs}></div>"),
        );
        let error = format!("{:#}", parse_file(&entry).unwrap_err());
        assert!(error.contains("data-composition-src on #shot"), "{error}");
        assert!(error.contains(expected), "{error}");
    };
    let valid = "data-start='0' data-track-index='0'";
    check("https://example.com/leaf.html", valid, "relative local");
    check("/tmp/leaf.html", valid, "relative local");
    check("leaf.html#part", valid, "relative local");
    check("missing.html", valid, "could not read");
    check("small.html", valid, "dimensions must match");
    check("finished.html", valid, "top-level composition");
    check("./index.html", valid, "cycle");
    check("leaf.html", "", "set data-start and data-track-index");
    check(
        "leaf.html",
        &format!("{valid} data-duration='5'"),
        "at most the scene's duration",
    );
    check(
        "leaf.html",
        &format!("{valid} data-variable-values='[]'"),
        "JSON object",
    );
    check(
        "leaf.html",
        &format!(r##"{valid} data-variable-values='{{"unknown":0}}'"##),
        "does not declare",
    );
    for depth in 1..=9 {
        page(
            &dir,
            &format!("{depth}.html"),
            "",
            &format!(
                "<div id='child' data-composition-src='{}.html' {valid}></div>",
                depth + 1
            ),
        );
    }
    page(&dir, "10.html", "", "");
    check("1.html", valid, "at most 8 nested levels");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn scene_camera_transition_and_audio_match_native_and_segmented_rendering() {
    let dir = scratch("scene-render");
    let shot = r##"<div id="camera" data-hf-camera='[{"at":0,"zoom":1},{"at":4,"easing":"smooth"}]'
                            data-hf-camera-subject="dot" data-hf-camera-lag="0">
         <div data-hf-parallax="0.25"><div style="left:10px;width:12px;height:8px;background:#fff"></div></div>
         <div id="dot" style="left:18px;top:15px;width:12px;height:12px;background:#0f0"
              data-hf-keyframes='[{"at":0},{"at":4,"x":22}]'></div>
      </div>"##;
    page(&dir, "first.html", "style='background:#123'", "");
    write_wav(
        &dir.join("tone.wav"),
        SAMPLE_RATE,
        1,
        &vec![1400; 3 * SAMPLE_RATE as usize],
    );
    page(
        &dir,
        "second.html",
        "style='background:#541'",
        &format!(
            "{shot}<audio id='tone' src='tone.wav' data-start='0.25'></audio>
             <audio id='bed' src='tone.wav' data-hf-duck='tone'></audio>
             <audio id='late' src='tone.wav' data-start='3.75' data-hf-duck='tone'></audio>"
        ),
    );
    let entry = page(
        &dir,
        "index.html",
        "",
        r##"<div id="first" data-composition-src="first.html" data-start="0" data-duration="2" data-track-index="1"></div>
           <div id="second" data-composition-src="second.html" data-start="first-1"
                data-duration="2.5" data-track-index="2"
                data-hf-transition="wipe-right" data-hf-transition-from="first"></div>
           <audio id="base" src="tone.wav" data-start="0" data-duration="0.25"></audio>"##,
    );
    let c = parse_file(&entry).unwrap();
    let camera = &c.elements[c.ids["second::camera"]];
    let rig = camera.motion.rig.as_ref().unwrap();
    assert_eq!(rig.subject().unwrap().element, c.ids["second::dot"]);
    assert!(rig.is_bound());
    assert_eq!(camera.start, 1.0);
    let mut r = Renderer::new(&c, 1.0).unwrap();
    assert_eq!(pixel(&mut r, 1.0, 93, 51), [17, 34, 51, 255]);
    assert_eq!(pixel(&mut r, 1.5, 3, 51), [85, 68, 17, 255]);
    assert_eq!(pixel(&mut r, 1.5, 93, 51), [17, 34, 51, 255]);
    assert_eq!(pixel(&mut r, 2.0, 93, 51), [85, 68, 17, 255]);
    assert_eq!(pixel(&mut r, 3.5, 93, 51), [0, 0, 0, 255]);

    let reference =
        parse_file(&page(&dir, "inline.html", "style='background:#541'", shot)).unwrap();
    let mut baseline = Renderer::new(&reference, 1.0).unwrap();
    for t in [2.25, 2.0, 3.375, 2.5] {
        assert_eq!(
            r.render_at(t).unwrap(),
            baseline.render_at(t - 1.0).unwrap()
        );
    }

    let mixed = mix(&c, 4 * SAMPLE_RATE as usize).unwrap();
    assert_eq!(mixed.clips[1].id.as_deref(), Some("second::tone"));
    assert_eq!(mixed.clips[1].start_frame, SAMPLE_RATE as usize * 5 / 4);
    assert_eq!(mixed.clips[1].frames, SAMPLE_RATE as usize * 9 / 4);
    assert_eq!(c.audio[2].effects.duck.as_ref().unwrap().sources, vec![1]);
    assert_eq!(mixed.clips[3].frames, 0);
    assert_eq!(mixed.left[7 * SAMPLE_RATE as usize / 2], 0.0);

    let single = dir.join("single.mp4");
    let parallel = dir.join("parallel.mp4");
    let assembled = dir.join("assembled.mp4");
    let parts = [dir.join("tail.aseg"), dir.join("head.aseg")];
    encode_native(&mut r, &single, 8, 1).unwrap();
    encode_native(&mut r, &parallel, 8, 2).unwrap();
    render_part(&mut r, &parts[0], &EncodeOptions::new(8, 2), 2..4).unwrap();
    render_part(&mut r, &parts[1], &EncodeOptions::new(8, 1), 0..2).unwrap();
    assemble_parts(&c, &parts, &assembled, 192_000).unwrap();
    let bits = fs::read(single).unwrap();
    assert_eq!(bits, fs::read(parallel).unwrap());
    assert_eq!(bits, fs::read(assembled).unwrap());
    fs::remove_dir_all(dir).unwrap();
}
