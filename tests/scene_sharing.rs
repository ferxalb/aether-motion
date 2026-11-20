//! Motion Scenes v1.2: instances of a reused scene share its compiled element bodies, earlier
//! siblings under an opaque full-frame scene are skipped without changing a pixel, and a
//! persistent `SceneCache` reuses unchanged scenes across compiles with transitive invalidation.

mod common;

use aether_motion::encode::{assemble_parts, encode_native, render_part, EncodeOptions};
use aether_motion::model::Composition;
use aether_motion::parse::{parse_file, parse_file_cached, values_from_json, SceneCache};
use aether_motion::render::Renderer;
use common::scratch;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const SIZE: (u32, u32) = (96, 54);

fn doc(dir: &Path, name: &str, extra: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        format!(
            r##"<main data-composition-id="t" data-width="{}" data-height="{}" data-duration="4" {extra}>{body}</main>"##,
            SIZE.0, SIZE.1
        ),
    )
    .unwrap();
    path
}

/// A scene instance; `extra` may override the default start (0) and track (0).
fn instance(id: &str, src: &str, extra: &str) -> String {
    let mut attrs = extra.to_string();
    for (name, default) in [("data-start", "0"), ("data-track-index", "0")] {
        if !extra.contains(name) {
            attrs.push_str(&format!(" {name}='{default}'"));
        }
    }
    format!("<div id='{id}' class='clip' data-composition-src='{src}' {attrs}></div>")
}

/// Element indices whose ids start with `prefix`.
fn scoped<'c>(c: &'c Composition, prefix: &'c str) -> impl Iterator<Item = usize> + 'c {
    let mut ids: Vec<_> = c
        .ids
        .iter()
        .filter(move |(id, _)| id.starts_with(prefix))
        .map(|(_, &idx)| idx)
        .collect();
    ids.sort();
    ids.into_iter()
}

/// A scene with text, a wrapped kinetic line (two-pass parse), a following camera, and a variable.
const LEAF: &str = r##"<div id="camera" data-hf-camera='[{"at":0,"zoom":1},{"at":3,"zoom":1.4,"easing":"smooth"}]'
         data-hf-camera-subject="dot" data-hf-camera-lag="0.2">
      <div id="dot" style="left:30px;top:20px;width:10px;height:10px;background:#0f0"
           data-hf-keyframes='[{"at":0},{"at":3,"x":20}]'></div>
    </div>
    <p id="label" data-var-text="word" style="left:4px;top:4px;font-size:9px;color:#ff0"></p>
    <p id="words" data-hf-split="words" data-hf-split-wrap
       style="left:4px;top:30px;width:40px;font-size:8px;color:#fff">one two three four five</p>"##;

const WORD: &str = r##"data-composition-variables='[{"id":"word","type":"string","label":"Word","default":"leaf"}]' style='background:#123'"##;

#[test]
fn instances_share_bodies_and_keep_variables_and_cameras_separate() {
    let dir = scratch("share-bodies");
    doc(&dir, "leaf.html", WORD, LEAF);
    doc(
        &dir,
        "middle.html",
        "",
        &format!(
            "{}{}",
            instance("a", "leaf.html", ""),
            instance("b", "leaf.html", "data-start='1' data-duration='3'")
        ),
    );
    let entry = doc(
        &dir,
        "index.html",
        "",
        &format!(
            "{}{}{}",
            instance("m1", "middle.html", ""),
            instance("m2", "middle.html", "data-track-index='1'"),
            instance(
                "other",
                "leaf.html",
                r##"data-track-index='2' data-variable-values='{"word":"other"}'"##
            )
        ),
    );
    let c = parse_file(&entry).unwrap();
    let pairs =
        |a: &str, b: &str| -> Vec<(usize, usize)> { scoped(&c, a).zip(scoped(&c, b)).collect() };
    // The four default-valued leaves (two per middle) share every scoped body except the camera,
    // which follows its own instance's subject.
    for (x, y) in pairs("m1::a::", "m2::b::") {
        let (ex, ey) = (&c.elements[x], &c.elements[y]);
        let camera = ex.motion.rig.is_some();
        assert_eq!(ex.shares_body(ey), !camera, "{:?}", ex.id);
        assert_ne!(ex.start, ey.start);
    }
    let label = |id: &str| match &c.elements[c.ids[id]].content {
        aether_motion::model::Content::Text(text) => text.clone(),
        _ => panic!("{id} is not text"),
    };
    assert_eq!(label("m1::a::label"), "leaf");
    assert_eq!(label("other::label"), "other");
    // Other values compile separately and share nothing.
    for (x, y) in pairs("m1::a::", "other::") {
        assert!(!c.elements[x].shares_body(&c.elements[y]));
    }
    // Each camera follows the dot of its own instance.
    let subject = |camera: &str| {
        let rig = c.elements[c.ids[camera]].motion.rig.clone().unwrap();
        rig.subject().unwrap().element
    };
    assert_eq!(subject("m1::a::camera"), c.ids["m1::a::dot"]);
    assert_eq!(subject("m2::b::camera"), c.ids["m2::b::dot"]);
    // Copy-on-write: changing one instance leaves the shared original untouched.
    let mut copy = c.elements[c.ids["m1::a::label"]].clone();
    assert!(copy.shares_body(&c.elements[c.ids["m2::a::label"]]));
    copy.style.font_size = Some(30.0);
    assert!(!copy.shares_body(&c.elements[c.ids["m2::a::label"]]));
    assert_eq!(c.elements[c.ids["m2::a::label"]].style.font_size, Some(9.0));
    fs::remove_dir_all(dir).unwrap();
}

/// The frame at `t` drawn without occlusion culling (a layout probe draws every element, and the
/// next call at the same instant reuses those pixels), and drawn normally with its count of
/// skipped subtrees.
fn with_and_without_culling(c: &Composition, scale: f32, t: f64) -> (Vec<u8>, Vec<u8>, usize) {
    let mut reference = Renderer::new(c, scale).unwrap();
    reference.probe_text_at(t).unwrap();
    let full = reference.render_at(t).unwrap().to_vec();
    assert_eq!(reference.stats().occluded, 0);
    let mut culled = Renderer::new(c, scale).unwrap();
    let frame = culled.render_at(t).unwrap().to_vec();
    (full, frame, culled.stats().occluded)
}

#[test]
fn opaque_scenes_skip_what_they_cover_without_changing_pixels() {
    let dir = scratch("occlusion");
    // Busy artwork under the cover, and inside the cover a backdrop blur and a blend mode that
    // read what is behind them.
    doc(
        &dir,
        "under.html",
        "style='background:rgba(255,0,255,0.5)'",
        r##"<div style="left:10px;top:5px;width:60px;height:40px;background:linear-gradient(90deg,#f00,#00f);border-radius:6px"
               data-hf-keyframes='[{"at":0},{"at":4,"x":20}]'></div>
          <p style="left:2px;top:30px;font-size:10px;color:#fff">under</p>"##,
    );
    let cover = |name: &str, background: &str| {
        doc(
            &dir,
            name,
            &format!("style='background:{background}'"),
            r##"<div style="left:20px;top:10px;width:40px;height:30px;background:rgba(255,255,255,0.2);backdrop-filter:blur(3px)"></div>
              <div style="left:50px;top:20px;width:30px;height:20px;background:#0a0;mix-blend-mode:screen"></div>
              <p style="left:4px;top:4px;font-size:9px;color:#ff0">cover</p>"##,
        );
    };
    cover("cover.html", "#204060");
    cover("glass.html", "rgba(32,64,96,0.9)");
    doc(&dir, "small.html", "style='background:#204060'", "");
    let small_square = dir.join("square.html");
    fs::write(
        &small_square,
        r#"<main data-composition-id="s" data-width="54" data-height="54" data-duration="4" style="background:#204060"></main>"#,
    )
    .unwrap();
    let film = |name: &str, top: &str| {
        doc(
            &dir,
            name,
            "style='background:#000'",
            &format!(
                "{}{}{top}",
                instance("u1", "under.html", ""),
                instance("u2", "under.html", "data-track-index='1' data-duration='4'")
            ),
        )
    };
    let top = |src: &str, extra: &str| {
        format!("<div id='top' class='clip' data-composition-src='{src}' data-start='1' data-duration='3' data-track-index='2' {extra}></div>")
    };
    let cases = [
        // Covers from its first frame: both earlier instances are skipped.
        ("plain", top("cover.html", ""), 2, 2.0),
        // Fading in: still translucent at 1.3 s.
        ("fade", top("cover.html", "data-hf-fade-in='0.5'"), 0, 1.3),
        (
            "fade-done",
            top("cover.html", "data-hf-fade-in='0.5'"),
            2,
            2.0,
        ),
        // A translucent background never covers.
        ("glass", top("glass.html", ""), 0, 2.0),
        // A push moves the incoming host while it runs (here, the whole clip).
        (
            "push",
            top(
                "cover.html",
                "data-hf-transition='push-left' data-hf-transition-from='u2'",
            ),
            0,
            3.5,
        ),
        // A letterboxed scene leaves bands uncovered; a cropped one covers.
        (
            "contain",
            top("square.html", "data-composition-fit='contain'"),
            0,
            2.0,
        ),
        (
            "cover-fit",
            top("square.html", "data-composition-fit='cover'"),
            2,
            2.0,
        ),
        // The host itself moving (keyframes) keeps everything drawn.
        (
            "moving-host",
            top(
                "cover.html",
                r#"data-hf-keyframes='[{"at":0,"x":5},{"at":3,"x":0}]'"#,
            ),
            0,
            2.0,
        ),
        // A filter on the host draws the scene through an effect layer.
        (
            "filtered",
            top("small.html", "style='filter:brightness(1.1)'"),
            0,
            2.0,
        ),
    ];
    for (name, top, skipped, t) in cases {
        let c = parse_file(&film(&format!("film-{name}.html"), &top)).unwrap();
        for scale in [1.0, 0.37] {
            let (full, frame, occluded) = with_and_without_culling(&c, scale, t);
            assert!(full == frame, "{name} at scale {scale}: pixels differ");
            // A fractional output size leaves the last column partly uncovered by the box.
            let expected = if scale == 1.0 { skipped } else { 0 };
            assert_eq!(occluded, expected, "{name} at scale {scale}");
        }
    }
    // Encodes with skipped subtrees are byte-identical across threads and segments.
    let c = parse_file(&film("film.html", &top("cover.html", ""))).unwrap();
    let mut r = Renderer::new(&c, 1.0).unwrap();
    let out = |name: &str| dir.join(name);
    encode_native(&mut r, &out("one.mp4"), 8, 1).unwrap();
    encode_native(&mut r, &out("four.mp4"), 8, 4).unwrap();
    let parts = [out("a.aseg"), out("b.aseg")];
    render_part(&mut r, &parts[0], &EncodeOptions::new(8, 2), 0..1).unwrap();
    render_part(&mut r, &parts[1], &EncodeOptions::new(8, 1), 1..4).unwrap();
    assemble_parts(&c, &parts, &out("parts.mp4"), 192_000).unwrap();
    let bits = fs::read(out("one.mp4")).unwrap();
    assert_eq!(bits, fs::read(out("four.mp4")).unwrap());
    assert_eq!(bits, fs::read(out("parts.mp4")).unwrap());
    fs::remove_dir_all(dir).unwrap();
}

/// Moves the modification time of `path` a minute back, so a persistent cache may keep scenes
/// that read it (files modified within two seconds of a compile are never trusted).
fn settle(path: &Path) {
    let file = fs::File::open(path).unwrap();
    let past = SystemTime::now() - Duration::from_secs(60);
    file.set_modified(past).unwrap();
}

fn settle_all(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            settle_all(&path);
        }
        settle(&path);
    }
}

fn compiled(entry: &Path, cache: &mut SceneCache) -> Composition {
    let c = parse_file_cached(entry, &values_from_json("{}").unwrap(), cache).unwrap();
    let fresh = parse_file(entry).unwrap();
    assert_eq!(format!("{:?}", c.elements), format!("{:?}", fresh.elements));
    assert_eq!(c.dependencies, fresh.dependencies);
    c
}

/// Whether the element `id` of `a` and `b` share their body (the scene was reused).
fn kept(a: &Composition, b: &Composition, id: &str) -> bool {
    a.elements[a.ids[id]].shares_body(&b.elements[b.ids[id]])
}

#[test]
fn watch_cache_reuses_unchanged_scenes_and_invalidates_importers() {
    let dir = scratch("watch-cache");
    let leaf = doc(&dir, "leaf.html", WORD, LEAF);
    let middle = doc(
        &dir,
        "middle.html",
        "",
        &format!(
            "<p id='title' style='font-size:8px;color:#fff'>middle</p>{}",
            instance("l", "leaf.html", "")
        ),
    );
    let other = doc(
        &dir,
        "other.html",
        "",
        "<p id='note' style='color:#0ff'>other</p>",
    );
    let entry = doc(
        &dir,
        "index.html",
        "",
        &format!(
            "{}{}",
            instance("m", "middle.html", ""),
            instance("o", "other.html", "data-track-index='1'")
        ),
    );
    settle_all(&dir);
    let mut cache = SceneCache::persistent();
    let first = compiled(&entry, &mut cache);
    // The leaf wraps text, so it and the middle scene are kept for both parses; the other scene
    // compiles the same on both and is kept once.
    assert_eq!(cache.len(), 5);
    let second = compiled(&entry, &mut cache);
    for id in ["m::title", "m::l::label", "m::l::words", "o::note"] {
        assert!(kept(&first, &second, id), "{id}");
    }

    // Editing the leaf recompiles it and the middle scene that imports it, not the other scene.
    fs::write(
        &leaf,
        fs::read_to_string(&leaf).unwrap().replace("five", "six"),
    )
    .unwrap();
    settle(&leaf);
    let third = compiled(&entry, &mut cache);
    assert!(!kept(&second, &third, "m::l::label"));
    assert!(!kept(&second, &third, "m::title"));
    assert!(kept(&second, &third, "o::note"));

    // Editing only the middle scene keeps the leaf.
    fs::write(
        &middle,
        fs::read_to_string(&middle)
            .unwrap()
            .replace(">middle<", ">mid<"),
    )
    .unwrap();
    settle(&middle);
    let fourth = compiled(&entry, &mut cache);
    assert!(!kept(&third, &fourth, "m::title"));
    assert!(kept(&third, &fourth, "m::l::label"));

    // A file edited just now may still be changing, so nothing that read it is kept.
    fs::write(
        &other,
        fs::read_to_string(&other)
            .unwrap()
            .replace("other<", "again<"),
    )
    .unwrap();
    let fifth = compiled(&entry, &mut cache);
    let sixth = compiled(&entry, &mut cache);
    assert!(!kept(&fifth, &sixth, "o::note"));
    assert!(kept(&fifth, &sixth, "m::l::label"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn watch_cache_recovers_when_missing_assets_appear() {
    let dir = scratch("watch-missing");
    fs::create_dir_all(dir.join("cels")).unwrap();
    let png = |path: &Path, rgb: [u8; 3]| {
        image::RgbaImage::from_pixel(2, 2, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
            .save(path)
            .unwrap();
    };
    png(&dir.join("cels/c0.png"), [255, 0, 0]);
    doc(
        &dir,
        "scene.html",
        "",
        r##"<img id="pic" src="pic.png" style="width:20px;height:20px">
            <img id="cels" data-hf-sequence="cels/c{frame}.png" data-hf-sequence-fps="1" style="left:30px;width:20px;height:20px">
            <div id="cc" data-hf-captions="words.json" style="left:0;top:40px;width:96px;height:12px;font-size:8px"></div>"##,
    );
    let entry = doc(&dir, "index.html", "", &instance("s", "scene.html", ""));
    settle_all(&dir);
    let mut cache = SceneCache::persistent();
    let values = values_from_json("{}").unwrap();
    // The transcript is missing: the compile fails, and its path is among those to watch.
    let error = parse_file_cached(&entry, &values, &mut cache).unwrap_err();
    assert!(format!("{error:#}").contains("words.json"), "{error:#}");
    assert!(cache.last_reads().contains(&dir.join("words.json")));
    assert!(cache.last_reads().contains(&dir.join("scene.html")));

    fs::write(
        dir.join("words.json"),
        r#"[{"text":"Hi","start":0,"end":0.5}]"#,
    )
    .unwrap();
    settle_all(&dir);
    // Now it compiles; the image is still missing, which only drawing reports.
    let first = compiled(&entry, &mut cache);
    assert!(first.dependencies.contains(&dir.join("pic.png")));
    assert!(Renderer::new(&first, 1.0).unwrap().render_at(0.0).is_err());
    let second = compiled(&entry, &mut cache);
    assert!(kept(&first, &second, "s::pic"));

    // The image appears: the scene that referred to it is compiled again and draws.
    png(&dir.join("pic.png"), [0, 255, 0]);
    settle(&dir.join("pic.png"));
    let third = compiled(&entry, &mut cache);
    assert!(!kept(&second, &third, "s::pic"));
    let mut r = Renderer::new(&third, 1.0).unwrap();
    let frame = r.render_at(0.0).unwrap();
    assert_eq!(&frame[..3], &[0, 255, 0]);

    // A new sequence frame extends the sequence: the folder listing changed.
    png(&dir.join("cels/c1.png"), [0, 0, 255]);
    settle_all(&dir);
    let fourth = compiled(&entry, &mut cache);
    assert!(!kept(&third, &fourth, "s::cels"));
    let mut r = Renderer::new(&fourth, 1.0).unwrap();
    let frame = r.render_at(1.5).unwrap();
    let at = (5 * SIZE.0 as usize + 35) * 4;
    assert_eq!(&frame[at..at + 3], &[0, 0, 255]);
    fs::remove_dir_all(dir).unwrap();
}
