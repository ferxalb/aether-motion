//! Motion Scenes v1.1: reuse of compiled scenes (equal to fresh compilation, with the same limits),
//! dependency lists for preview, and `data-composition-fit` for scenes whose canvas differs.

mod common;

use aether_motion::encode::{assemble_parts, encode_native, render_part, EncodeOptions};
use aether_motion::model::Composition;
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;
use std::fs;
use std::path::{Path, PathBuf};

fn doc(dir: &Path, name: &str, size: (u32, u32), extra: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        format!(
            r##"<main data-composition-id="t" data-width="{}" data-height="{}" data-duration="4" {extra}>{body}</main>"##,
            size.0, size.1
        ),
    )
    .unwrap();
    path
}

const SIZE: (u32, u32) = (96, 54);

/// A scene with wrapped kinetic text (two-pass parse), a camera rig, keyframes, and a variable.
const LEAF: &str = r##"<div id="camera" data-hf-camera='[{"at":0,"zoom":1},{"at":3,"zoom":1.4,"easing":"smooth"}]'
         data-hf-camera-subject="dot" data-hf-camera-lag="0.2">
      <div id="dot" style="left:30px;top:20px;width:10px;height:10px;background:#0f0"
           data-hf-keyframes='[{"at":0},{"at":3,"x":20}]'></div>
    </div>
    <p id="words" data-var-text="word" data-hf-split="words" data-hf-split-wrap
       style="left:4px;top:30px;width:40px;font-size:8px;color:#fff"></p>"##;

const WORD: &str = r##"data-composition-variables='[{"id":"word","type":"string","label":"Word","default":"one two three four five"}]'"##;

fn frames(c: &Composition, times: &[f64]) -> Vec<Vec<u8>> {
    let mut r = Renderer::new(c, 1.0).unwrap();
    times
        .iter()
        .map(|&t| r.render_at(t).unwrap().to_vec())
        .collect()
}

#[test]
fn reused_scenes_compile_exactly_like_fresh_copies() {
    let dir = scratch("scene-reuse");
    for name in ["leaf.html", "leaf-b.html", "leaf-c.html", "leaf-d.html"] {
        doc(
            &dir,
            name,
            SIZE,
            &format!("{WORD} style='background:#123'"),
            LEAF,
        );
    }
    let middle = |leaf: [&str; 2]| {
        format!(
            r##"<div id="a" data-composition-src="{}" data-start="0" data-duration="2" data-track-index="0"></div>
               <div id="b" data-composition-src="{}" data-start="1" data-duration="3" data-track-index="1"
                    data-variable-values='{{"word":"alpha beta gamma delta epsilon zeta"}}'></div>"##,
            leaf[0], leaf[1]
        )
    };
    doc(
        &dir,
        "middle.html",
        SIZE,
        "",
        &middle(["leaf.html", "leaf.html"]),
    );
    doc(
        &dir,
        "middle-b.html",
        SIZE,
        "",
        &middle(["leaf-b.html", "leaf-c.html"]),
    );
    let entry = |middles: [&str; 2], leaf: &str| {
        format!(
            r##"<div id="m1" data-composition-src="{}" data-start="0" data-track-index="0"></div>
               <div id="l" data-composition-src="{leaf}" data-start="0.5" data-duration="3" data-track-index="1"></div>
               <div id="m2" data-composition-src="{}" data-start="l-0.5" data-duration="1" data-track-index="2"
                    data-hf-transition="crossfade" data-hf-transition-from="l"></div>"##,
            middles[0], middles[1]
        )
    };
    let reused = parse_file(&doc(
        &dir,
        "reused.html",
        SIZE,
        "",
        &entry(["middle.html", "middle.html"], "leaf.html"),
    ))
    .unwrap();
    // Every instance below points at a different file, so nothing can be reused.
    doc(
        &dir,
        "middle-c.html",
        SIZE,
        "",
        &middle(["leaf-d.html", "leaf-b.html"]),
    );
    let fresh = parse_file(&doc(
        &dir,
        "fresh.html",
        SIZE,
        "",
        &entry(["middle-b.html", "middle-c.html"], "leaf-d.html"),
    ))
    .unwrap();
    assert_eq!(
        format!("{:?}", reused.elements),
        format!("{:?}", fresh.elements)
    );
    assert_eq!(reused.ids, fresh.ids);
    let times = [0.0, 0.75, 1.5, 2.25, 3.0, 3.9];
    assert_eq!(frames(&reused, &times), frames(&fresh, &times));
    let names: Vec<_> = reused
        .dependencies
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["leaf.html", "middle.html", "reused.html"]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reuse_keeps_cycle_depth_instance_and_size_limits() {
    let dir = scratch("scene-reuse-limits");
    let instance = |id: &str, src: &str, extra: &str| {
        format!("<div id='{id}' data-composition-src='{src}' data-start='0' data-track-index='0' {extra}></div>")
    };
    let error = |entry: &Path| format!("{:#}", parse_file(entry).unwrap_err());

    // `x` imports `a` only when its flag is set; `a` (compiled first) imports `x` without it, so
    // the cached `a` contains `x` and reusing it inside `x` is a cycle.
    let flag = r##"data-composition-variables='[{"id":"deep","type":"boolean","label":"Deep","default":false}]'"##;
    doc(
        &dir,
        "x.html",
        SIZE,
        flag,
        &instance("a", "a.html", "data-var-if='deep'"),
    );
    doc(&dir, "a.html", SIZE, "", &instance("x", "x.html", ""));
    let cyclic = doc(
        &dir,
        "cycle.html",
        SIZE,
        "",
        &format!(
            "{}{}",
            instance("first", "a.html", ""),
            instance(
                "second",
                "x.html",
                r##"data-variable-values='{"deep":true}'"##
            )
        ),
    );
    assert!(error(&cyclic).contains("cycle"), "{}", error(&cyclic));

    // A chain of eight levels fits once; reused one level deeper it does not.
    for level in 1..8 {
        doc(
            &dir,
            &format!("c{level}.html"),
            SIZE,
            "",
            &instance("n", &format!("c{}.html", level + 1), ""),
        );
    }
    doc(&dir, "c8.html", SIZE, "", "");
    doc(&dir, "deeper.html", SIZE, "", &instance("n", "c1.html", ""));
    let ok = doc(&dir, "chain.html", SIZE, "", &instance("a", "c1.html", ""));
    parse_file(&ok).unwrap();
    let deep = doc(
        &dir,
        "deep.html",
        SIZE,
        "",
        &format!(
            "{}{}",
            instance("a", "c1.html", ""),
            instance("b", "deeper.html", "")
        ),
    );
    assert!(
        error(&deep).contains("at most 8 nested levels"),
        "{}",
        error(&deep)
    );

    // Reused instances still count: 16 x (1 + 16) = 272 > 256.
    doc(&dir, "leaf.html", SIZE, "", "");
    let leaves: String = (0..16)
        .map(|i| instance(&format!("l{i}"), "leaf.html", ""))
        .collect();
    doc(&dir, "group.html", SIZE, "", &leaves);
    let groups: String = (0..16)
        .map(|i| instance(&format!("g{i}"), "group.html", ""))
        .collect();
    let many = doc(&dir, "many.html", SIZE, "", &groups);
    assert!(
        error(&many).contains("at most 256 instances"),
        "{}",
        error(&many)
    );

    // The flattened tree is bounded: 1001 elements x 200 instances passes 200,000.
    let boxes = "<div style='width:1px;height:1px'></div>".repeat(1000);
    doc(&dir, "heavy.html", SIZE, "", &boxes);
    let heavy: String = (0..200)
        .map(|i| instance(&format!("h{i}"), "heavy.html", ""))
        .collect();
    let big = doc(&dir, "big.html", SIZE, "", &heavy);
    assert!(
        error(&big).contains("more than 200000 elements"),
        "{}",
        error(&big)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dependencies_list_scenes_and_assets_for_preview() {
    let dir = scratch("scene-deps");
    fs::create_dir_all(dir.join("shots/cels")).unwrap();
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><path fill="#0ff" d="M0 0h4v4H0z"/></svg>"##;
    fs::write(dir.join("shots/tile.svg"), svg).unwrap();
    fs::write(dir.join("shots/swap.svg"), svg).unwrap();
    let png = {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    };
    for frame in 0..2 {
        fs::write(dir.join(format!("shots/cels/c{frame}.png")), &png).unwrap();
    }
    fs::write(
        dir.join("shots/words.json"),
        r#"[{"text":"Hi","start":0,"end":0.5}]"#,
    )
    .unwrap();
    common::write_wav(&dir.join("shots/tone.wav"), 48_000, 1, &[0; 4800]);
    let picture = r##"data-composition-variables='[{"id":"pic","type":"image","label":"Pic","default":"tile.svg"}]'"##;
    doc(
        &dir,
        "shots/scene.html",
        SIZE,
        picture,
        r##"<img data-var-src="pic" style="width:4px;height:4px">
            <img data-hf-sequence="cels/c{frame}.png" style="left:8px;width:2px;height:2px">
            <div data-hf-captions="words.json" style="left:0;top:40px;width:96px;height:12px;font-size:8px"></div>
            <audio src="tone.wav"></audio>"##,
    );
    let entry = doc(
        &dir,
        "index.html",
        SIZE,
        "",
        r##"<div id="s" data-composition-src="shots/scene.html" data-start="0" data-track-index="0"
                 data-variable-values='{"pic":"swap.svg"}'></div>"##,
    );
    let c = parse_file(&entry).unwrap();
    let shots = dir.join("shots");
    let expected: Vec<PathBuf> = vec![
        entry.clone(),
        shots.join("cels"),
        shots.join("cels/c0.png"),
        shots.join("cels/c1.png"),
        shots.join("scene.html"),
        shots.join("swap.svg"),
        shots.join("tone.wav"),
        shots.join("words.json"),
    ];
    let mut expected = expected;
    expected.sort();
    assert_eq!(c.dependencies, expected);
    fs::remove_dir_all(dir).unwrap();
}

fn diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap()
}

#[test]
fn fitted_scenes_match_the_scene_rendered_at_that_size() {
    let dir = scratch("scene-fit");
    // A 2x canvas (192x108) whose content scales to the 96x54 host; rendering the scene itself at
    // scale 0.5 is the reference.
    let body = r##"<div id="camera" data-hf-camera='[{"at":0,"zoom":1},{"at":3,"zoom":1.5,"easing":"smooth"}]'
             data-hf-camera-subject="dot" data-hf-camera-lag="0.2">
          <div style="left:20px;top:20px;width:60px;height:40px;background:#fff"></div>
          <div id="dot" style="left:100px;top:50px;width:20px;height:20px;background:#0f0"
               data-hf-keyframes='[{"at":0},{"at":3,"x":40,"y":20}]'></div>
        </div>"##;
    let big = doc(
        &dir,
        "big.html",
        (192, 108),
        "style='background:#246'",
        body,
    );
    let entry = doc(
        &dir,
        "index.html",
        SIZE,
        "style='background:#f00'",
        r##"<div id="s" data-composition-src="big.html" data-composition-fit="contain"
                 data-start="0" data-track-index="0"></div>"##,
    );
    let c = parse_file(&entry).unwrap();
    let reference = parse_file(&big).unwrap();
    let mut fitted = Renderer::new(&c, 1.0).unwrap();
    let mut scaled = Renderer::new(&reference, 0.5).unwrap();
    for t in [0.0, 1.0, 2.0, 3.5] {
        let a = fitted.render_at(t).unwrap().to_vec();
        let b = scaled.render_at(t).unwrap().to_vec();
        assert!(diff(&a, &b) <= 2, "t={t}: max difference {}", diff(&a, &b));
    }

    // Segmented and multi-threaded encodes of a fitted scene are byte-identical.
    let out = |name: &str| dir.join(name);
    encode_native(&mut fitted, &out("one.mp4"), 8, 1).unwrap();
    encode_native(&mut fitted, &out("two.mp4"), 8, 2).unwrap();
    let parts = [out("a.aseg"), out("b.aseg")];
    render_part(&mut fitted, &parts[0], &EncodeOptions::new(8, 1), 0..2).unwrap();
    render_part(&mut fitted, &parts[1], &EncodeOptions::new(8, 2), 2..4).unwrap();
    assemble_parts(&c, &parts, &out("parts.mp4"), 192_000).unwrap();
    let bits = fs::read(out("one.mp4")).unwrap();
    assert_eq!(bits, fs::read(out("two.mp4")).unwrap());
    assert_eq!(bits, fs::read(out("parts.mp4")).unwrap());

    // A square scene: contain letterboxes, cover fills, none keeps 1:1 centered.
    doc(&dir, "square.html", (27, 27), "style='background:#00f'", "");
    let pixel = |fit: &str, x: usize, y: usize| {
        let entry = doc(
            &dir,
            "square-host.html",
            SIZE,
            "style='background:#f00'",
            &format!("<div id='s' data-composition-src='square.html' data-composition-fit='{fit}' data-start='0' data-track-index='0'></div>"),
        );
        let c = parse_file(&entry).unwrap();
        let mut r = Renderer::new(&c, 1.0).unwrap();
        let frame = r.render_at(0.0).unwrap();
        let i = (y * 96 + x) * 4;
        [frame[i], frame[i + 1], frame[i + 2]]
    };
    let (red, blue) = ([255, 0, 0], [0, 0, 255]);
    assert_eq!(pixel("contain", 20, 27), red);
    assert_eq!(pixel("contain", 22, 1), blue);
    assert_eq!(pixel("contain", 73, 52), blue);
    assert_eq!(pixel("contain", 76, 27), red);
    assert_eq!(pixel("cover", 1, 1), blue);
    assert_eq!(pixel("cover", 94, 52), blue);
    assert_eq!(pixel("none", 33, 27), red);
    assert_eq!(pixel("none", 36, 15), blue);
    assert_eq!(pixel("none", 36, 12), red);

    let error = |attrs: &str| {
        let entry = doc(
            &dir,
            "bad.html",
            SIZE,
            "",
            &format!("<div id='s' data-composition-src='square.html' {attrs} data-start='0' data-track-index='0'></div>"),
        );
        format!("{:#}", parse_file(&entry).unwrap_err())
    };
    assert!(error("").contains("set data-composition-fit"));
    assert!(error("data-composition-fit='fill'").contains("unevenly"));
    assert!(error("data-composition-fit='stretch'").contains("contain, cover, or none"));

    // Matching canvases ignore the attribute and compile exactly as without it.
    doc(&dir, "same.html", SIZE, WORD, LEAF);
    let with = |fit: &str| {
        let entry = doc(
            &dir,
            "same-host.html",
            SIZE,
            "",
            &format!("<div id='s' data-composition-src='same.html' {fit} data-start='0' data-track-index='0'></div>"),
        );
        format!("{:?}", parse_file(&entry).unwrap().elements)
    };
    assert_eq!(with(""), with("data-composition-fit='cover'"));
    fs::remove_dir_all(dir).unwrap();
}
