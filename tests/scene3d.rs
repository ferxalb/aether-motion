//! Native 3D scenes (`data-hf-3d`): primitives, depth testing, back-face culling, flat Lambert and
//! unlit shading, perspective and orthographic cameras, timeline animation, composition with 2D
//! effects, 2.5D spaces, cameras, and transitions, deterministic output across render order,
//! threads, and segments, the `check` report, and the errors that keep scenes well-formed.

mod common;

use aether_motion::agent::check;
use aether_motion::encode::{assemble_parts, encode_native, render_part, EncodeOptions};
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::{scratch, Shot};
use std::path::{Path, PathBuf};

const FPS: u32 = 12;

fn write(dir: &Path, name: &str, html: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, html).unwrap();
    path
}

/// A 320x180 black composition lasting 2 s with one full-frame scene holding `nodes`.
fn scene(attrs: &str, nodes: &str) -> String {
    format!(
        "<div data-composition-id='s3d' data-width='320' data-height='180' data-duration='2' style='background:#000'>\
         <div id='world' class='clip' data-start='0' data-duration='2' data-track-index='1' data-hf-3d {attrs} \
          style='position:absolute;left:0;top:0;width:320px;height:180px'>{nodes}</div></div>"
    )
}

fn shot_at(html: &str, time: f64) -> Shot {
    let dir = scratch("scene3d");
    let path = write(&dir, "index.html", html);
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let (w, h) = renderer.dimensions();
    let pixels = renderer.render_at(time).unwrap().to_vec();
    std::fs::remove_dir_all(dir).unwrap();
    Shot {
        pixels,
        width: w as usize,
        height: h as usize,
    }
}

fn red(p: [u8; 3]) -> bool {
    p[0] > 160 && p[1] < 60 && p[2] < 60
}

fn green(p: [u8; 3]) -> bool {
    p[1] > 160 && p[0] < 60 && p[2] < 60
}

const UNLIT_RED: &str = "material='unlit' color='#ff0000'";
const UNLIT_GREEN: &str = "material='unlit' color='#00ff00'";

#[test]
fn a_box_at_the_origin_is_drawn_at_its_css_size() {
    let html = scene(
        "",
        &format!("<hf-mesh shape='box' size='100 60 40' {UNLIT_RED}></hf-mesh>"),
    );
    let shot = shot_at(&html, 0.0);
    // The front face sits 20 px toward the camera (1000 px away): 1000/980 magnification.
    let (l, t, r, b) = shot.bbox(red).unwrap();
    let w = (r - l + 1) as f64;
    let h = (b - t + 1) as f64;
    assert!((w - 100.0 * 1000.0 / 980.0).abs() <= 1.5, "width {w}");
    assert!((h - 60.0 * 1000.0 / 980.0).abs() <= 1.5, "height {h}");
    assert_eq!(shot.at(5, 5), [0, 0, 0]);
}

#[test]
fn nearer_meshes_hide_farther_ones_in_any_document_order() {
    let near =
        format!("<hf-mesh shape='box' size='60 60 10' position='0 0 200' {UNLIT_GREEN}></hf-mesh>");
    let far = format!(
        "<hf-mesh shape='box' size='200 120 10' position='0 0 -200' {UNLIT_RED}></hf-mesh>"
    );
    for nodes in [format!("{near}{far}"), format!("{far}{near}")] {
        let shot = shot_at(&scene("", &nodes), 0.0);
        assert!(green(shot.at(160, 90)), "center {:?}", shot.at(160, 90));
        assert!(red(shot.at(160 - 70, 90)));
    }
}

#[test]
fn intersecting_meshes_are_split_per_pixel() {
    // Two slabs crossing in an X seen from above: each wins on one side of the crossing line.
    let nodes = format!(
        "<hf-mesh shape='box' size='200 100 4' rotation='0 30 0' {UNLIT_RED}></hf-mesh>\
         <hf-mesh shape='box' size='200 100 4' rotation='0 -30 0' {UNLIT_GREEN}></hf-mesh>"
    );
    let shot = shot_at(&scene("", &nodes), 0.0);
    // rotateY(30) turns the right end away: on the right the -30 slab (green) is nearer.
    assert!(green(shot.at(220, 90)), "right {:?}", shot.at(220, 90));
    assert!(red(shot.at(100, 90)), "left {:?}", shot.at(100, 90));
}

#[test]
fn back_faces_are_culled_unless_cull_is_none() {
    // A plane facing up seen from straight above is visible; flipped, it shows its back.
    let camera = "<hf-camera position='0 -800 0' target='0 0 0'></hf-camera>";
    let flipped = |cull: &str| {
        scene(
            "",
            &format!("{camera}<hf-mesh shape='plane' size='200 200' rotation='180 0 0' {cull} {UNLIT_RED}></hf-mesh>"),
        )
    };
    assert_eq!(shot_at(&flipped(""), 0.0).count(red), 0);
    assert!(shot_at(&flipped("cull='none'"), 0.0).count(red) > 1000);
    let up = scene(
        "",
        &format!("{camera}<hf-mesh shape='plane' size='200 200' {UNLIT_RED}></hf-mesh>"),
    );
    assert!(shot_at(&up, 0.0).count(red) > 1000);
}

#[test]
fn lambert_shading_follows_directional_and_ambient_lights() {
    let lit = |lights: &str| {
        let html = scene(
            "",
            &format!("{lights}<hf-mesh shape='box' size='120 120 120' rotation='0 45 0' color='#ffffff'></hf-mesh>"),
        );
        let shot = shot_at(&html, 0.0);
        (shot.at(130, 90), shot.at(190, 90))
    };
    // Light travelling to +x (from the left): the left face is bright, the right one dark.
    let (left, right) =
        lit("<hf-light type='directional' direction='1 0 -0.2' intensity='1'></hf-light>");
    assert!(left[0] > 150 && right[0] < 10, "{left:?} {right:?}");
    // Ambient alone lights both faces equally.
    let (left, right) = lit("<hf-light type='ambient' intensity='0.5' color='#ffffff'></hf-light>");
    assert_eq!(left, right);
    assert!(left[0] > 150 && left[0] < 220, "{left:?}");
    // Without lights, the default rig applies.
    let (left, right) = lit("");
    assert!(left[0] > 40 && right[0] > 40 && left != right);
}

#[test]
fn orthographic_cameras_ignore_distance() {
    let nodes = |z: &str| {
        format!(
            "<hf-camera projection='orthographic' position='0 0 1000'></hf-camera>\
             <hf-mesh shape='box' size='80 80 2' position='0 0 {z}' {UNLIT_RED}></hf-mesh>"
        )
    };
    let near = shot_at(&scene("", &nodes("400")), 0.0).count(red);
    let far = shot_at(&scene("", &nodes("-3000")), 0.0).count(red);
    assert_eq!(near, far);
    assert!((near as i64 - 80 * 80).abs() < 200, "{near}");
}

#[test]
fn near_and_far_planes_clip_geometry() {
    let nodes = |z: &str| {
        format!(
            "<hf-camera near='100' far='2000'></hf-camera>\
             <hf-mesh shape='box' size='20 20 2' position='0 0 {z}' {UNLIT_RED}></hf-mesh>"
        )
    };
    assert!(shot_at(&scene("", &nodes("0")), 0.0).count(red) > 100);
    assert_eq!(shot_at(&scene("", &nodes("950")), 0.0).count(red), 0);
    assert_eq!(shot_at(&scene("", &nodes("-1500")), 0.0).count(red), 0);
}

#[test]
fn keyframes_animate_nodes_and_the_camera() {
    let html = scene(
        "",
        &format!(
            "<hf-camera data-hf-keyframes='[{{\"at\":0,\"yaw\":0}},{{\"at\":2,\"yaw\":10}}]'></hf-camera>\
             <hf-node data-hf-keyframes='[{{\"at\":0,\"x\":-80}},{{\"at\":1,\"x\":0,\"easing\":\"smooth\"}}]'>\
             <hf-mesh shape='sphere' radius='20' detail='2' {UNLIT_RED}></hf-mesh></hf-node>"
        ),
    );
    let center = |t: f64| {
        let (l, _, r, _) = shot_at(&html, t).bbox(red).unwrap();
        (l + r) as f64 / 2.0
    };
    let (a, b, c) = (center(0.0), center(1.0), center(1.9));
    assert!((a - (160.0 - 80.0)).abs() < 2.0, "{a}");
    // At 1 s the group is at the origin and the camera has panned right 5 degrees.
    assert!(b < 160.0 && b > 60.0, "{b}");
    assert!(c < b, "the panning camera moves the subject left: {b} {c}");
}

#[test]
fn renders_do_not_depend_on_order_threads_or_segments() {
    let dir = scratch("scene3d-determinism");
    let html = write(
        &dir,
        "index.html",
        "<div data-composition-id='d' data-width='160' data-height='96' data-duration='2.5' style='background:#101820'>\
             <div id='world' class='clip' data-start='0' data-duration='2.5' data-track-index='1' data-hf-3d data-hf-3d-antialias='3' \
              style='position:absolute;left:0;top:0;width:160px;height:96px'>\
             <hf-light type='directional' direction='0.4 0.8 -0.5'></hf-light>\
             <hf-camera position='0 -300 700' target='0 0 0' data-hf-keyframes='[{\"at\":0,\"x\":-300},{\"at\":2.5,\"x\":300,\"easing\":\"smooth\"}]'></hf-camera>\
             <hf-mesh shape='plane' size='800 800' position='0 60 0' color='#335'></hf-mesh>\
             <hf-node data-hf-keyframes='[{\"at\":0,\"rotateY\":0},{\"at\":2.5,\"rotateY\":200}]'>\
             <hf-mesh shape='cone' radius='40' height='120' segments='9' color='#e96'></hf-mesh>\
             <hf-mesh shape='sphere' radius='30' position='90 20 0' color='#6be'></hf-mesh></hf-node></div></div>"
    );
    let composition = parse_file(&html).unwrap();
    let frame = |renderer: &mut Renderer, n: usize| renderer.render_frame(n, FPS).unwrap().to_vec();
    let mut a = Renderer::new(&composition, 1.0).unwrap();
    let mut b = Renderer::new(&composition, 1.0).unwrap();
    let forward: Vec<_> = (0..6).map(|n| frame(&mut a, n * 5)).collect();
    let backward: Vec<_> = (0..6).rev().map(|n| frame(&mut b, n * 5)).collect();
    for (x, y) in forward.iter().zip(backward.iter().rev()) {
        assert!(x == y, "frames differ with render order");
    }
    assert!(forward[0] != forward[1], "the scene animates");

    let encode = |threads: u32, name: &str| {
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        let path = dir.join(name);
        encode_native(&mut renderer, &path, FPS, threads).unwrap();
        std::fs::read(path).unwrap()
    };
    let one = encode(1, "one.mp4");
    assert_eq!(one, encode(4, "four.mp4"), "thread count changed the bytes");
    let parts = [dir.join("a.aseg"), dir.join("b.aseg")];
    for (part, range) in parts.iter().zip([0..1, 1..3]) {
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        render_part(&mut renderer, part, &EncodeOptions::new(FPS, 2), range).unwrap();
    }
    let assembled = dir.join("assembled.mp4");
    assemble_parts(&composition, &parts, &assembled, 192_000).unwrap();
    assert_eq!(
        one,
        std::fs::read(&assembled).unwrap(),
        "segments changed the bytes"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn scenes_composite_with_2d_cameras_spaces_effects_and_transitions() {
    let mesh =
        format!("<hf-mesh shape='box' size='80 80 80' rotation='20 30 0' {UNLIT_RED}></hf-mesh>");
    // Inside a zooming 2D camera: the scene scales with it.
    let zoomed = format!(
        "<div data-composition-id='z' data-width='320' data-height='180' data-duration='2' style='background:#000'>\
         <div id='cam' style='position:absolute;left:0;top:0;width:320px;height:180px' data-hf-camera='[{{\"at\":0,\"zoom\":1}},{{\"at\":1,\"zoom\":2}}]'>\
         <div data-hf-3d style='position:absolute;left:0;top:0;width:320px;height:180px'>{mesh}</div></div></div>"
    );
    let small = shot_at(&zoomed, 0.0).count(red);
    let large = shot_at(&zoomed, 1.0).count(red);
    assert!(large > small * 3, "{small} -> {large}");
    // As a tilted plane of a 2.5D space, with a filter and per-element motion blur.
    let spaced = format!(
        "<div data-composition-id='p' data-width='320' data-height='180' data-duration='2' style='background:#000'>\
         <div data-hf-space style='position:absolute;left:0;top:0;width:320px;height:180px'>\
         <div data-hf-3d data-hf-rotate-y='35' data-hf-z='-100' data-hf-motion-blur='on' style='position:absolute;left:40px;top:20px;width:240px;height:140px;filter:brightness(1.1)'>{mesh}</div></div></div>"
    );
    assert!(shot_at(&spaced, 0.5).count(red) > 500);
    // Joined to a 2D scene by a crossfade: halfway, the box is a dimmed red.
    let joined = format!(
        "<div data-composition-id='j' data-width='320' data-height='180' data-duration='2' style='background:#000'>\
         <div id='a' class='clip' data-start='0' data-duration='1.2' data-track-index='1' style='position:absolute;left:0;top:0;width:320px;height:180px;background:#000'></div>\
         <div id='b' class='clip' data-start='0.8' data-duration='1.2' data-track-index='2' data-hf-transition='crossfade' data-hf-transition-from='a' data-hf-transition-easing='linear' data-hf-3d style='position:absolute;left:0;top:0;width:320px;height:180px'>{mesh}</div></div>"
    );
    let mid = shot_at(&joined, 1.0).at(160, 90);
    assert!(mid[0] > 90 && mid[0] < 170 && mid[1] < 20, "{mid:?}");
}

#[test]
fn root_motion_blur_blurs_moving_scenes_only() {
    let html = |blur: &str| {
        format!(
            "<div data-composition-id='m' data-width='320' data-height='180' data-duration='2' {blur} style='background:#000'>\
             <div data-hf-3d style='position:absolute;left:0;top:0;width:320px;height:180px'>\
             <hf-mesh shape='box' size='40 40 40' {UNLIT_RED} data-hf-keyframes='[{{\"at\":0,\"x\":-300}},{{\"at\":2,\"x\":300}}]'></hf-mesh></div></div>"
        )
    };
    let partial = |shot: &Shot| shot.count(|p| p[0] > 20 && p[0] < 200);
    let sharp = shot_at(&html(""), 1.0);
    let blurred = shot_at(
        &html("data-motion-blur='on' data-motion-blur-shutter='360'"),
        1.0,
    );
    assert!(
        partial(&blurred) > partial(&sharp) * 4,
        "{} vs {}",
        partial(&sharp),
        partial(&blurred)
    );
}

#[test]
fn check_reports_scenes_and_rejects_mistakes() {
    let dir = scratch("scene3d-check");
    let ok = write(
        &dir,
        "ok.html",
        &scene(
            "data-hf-3d-antialias='3'",
            "<hf-light type='ambient' intensity='0.4'></hf-light>\
             <hf-camera fov='40' target='#hero'></hf-camera>\
             <hf-node id='hero'><hf-mesh shape='sphere' detail='2' color='#fa0'></hf-mesh></hf-node>\
             <hf-mesh shape='custom' vertices='0 0 0, 10 0 0, 0 10 0' indices='0 1 2' material='unlit' cull='none'></hf-mesh>",
        ),
    );
    let report = check(&ok);
    assert_eq!(report["ok"], true, "{report}");
    let report = &report["scenes3d"][0];
    assert_eq!(report["antialias"], 3);
    assert_eq!(report["triangles"], 321);
    assert_eq!(report["meshes"][0]["shape"], "sphere");
    assert_eq!(report["meshes"][1]["material"], "unlit");
    assert_eq!(report["camera"]["projection"]["fov"], 40.0);
    assert_eq!(report["camera"]["target"], true);
    assert_eq!(report["defaultLights"], false);
    assert_eq!(report["backend"], "cpu");

    for (nodes, needle) in [
        ("<hf-mesh shape='torus'></hf-mesh>", "unknown shape"),
        (
            "<hf-mesh shape='box' radius='3'></hf-mesh>",
            "does not apply",
        ),
        (
            "<hf-mesh shape='box' colour='#fff'></hf-mesh>",
            "unknown attribute colour",
        ),
        (
            "<hf-mesh shape='box' material='pbr'></hf-mesh>",
            "PBR is not supported",
        ),
        ("<hf-light type='point'></hf-light>", "unknown light type"),
        (
            "<hf-camera></hf-camera><hf-camera></hf-camera>",
            "at most one",
        ),
        ("<hf-camera target='#nope'></hf-camera>", "not a node"),
        ("<div></div>", "cannot be part of a 3D scene"),
        ("hello", "text inside"),
        (
            "<hf-mesh shape='sphere' detail='9'></hf-mesh>",
            "detail must be",
        ),
        (
            "<hf-mesh shape='custom' vertices='0 0 0' indices='0 1 2'></hf-mesh>",
            "does not exist",
        ),
        (
            "<hf-node data-hf-keyframes='[{\"at\":0,\"yaw\":3}]'></hf-node>",
            "unknown keyframe channel yaw",
        ),
        (
            "<hf-node data-hf-keyframes='[{\"at\":1},{\"at\":0.5}]'></hf-node>",
            "strictly increasing",
        ),
        (
            "<hf-camera near='10' far='5'></hf-camera>",
            "must be greater than near",
        ),
    ] {
        let bad = write(&dir, "bad.html", &scene("", nodes));
        let report = check(&bad);
        assert_eq!(report["ok"], false, "{nodes}");
        let error = report["error"].as_str().unwrap();
        assert!(error.contains(needle), "{nodes}: {error}");
        assert!(
            report["fix"].as_str().unwrap().contains("hf-mesh"),
            "{nodes}"
        );
    }
    let bad = write(&dir, "bad.html", &scene("data-hf-3d-antialias='8'", ""));
    assert!(check(&bad)["error"].as_str().unwrap().contains("antialias"));
    let too_many: String = (0..30)
        .map(|_| "<hf-mesh shape='sphere' detail='4'></hf-mesh>".to_string())
        .collect();
    let bad = write(&dir, "bad.html", &scene("", &too_many));
    assert!(check(&bad)["error"]
        .as_str()
        .unwrap()
        .contains("at most 100000 triangles"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn composition_variables_reach_scene_colors_and_toggles() {
    let html = "<div data-composition-id='v' data-width='320' data-height='180' data-duration='2' style='background:#000'\
         data-composition-variables='[{\"id\":\"accent\",\"type\":\"color\",\"label\":\"Accent\",\"default\":\"#ff0000\"},\
         {\"id\":\"extra\",\"type\":\"boolean\",\"label\":\"Extra\",\"default\":false}]'>\
         <div data-hf-3d style='position:absolute;left:0;top:0;width:320px;height:180px'>\
         <hf-mesh shape='box' size='60 60 4' position='-60 0 0' material='unlit' color='var(--accent)'></hf-mesh>\
         <hf-mesh shape='box' size='60 60 4' position='60 0 0' material='unlit' color='#00ff00' data-var-if='extra'></hf-mesh>\
         </div></div>";
    let shot = shot_at(html, 0.0);
    assert!(red(shot.at(100, 90)), "{:?}", shot.at(100, 90));
    assert_eq!(shot.count(green), 0, "the toggled mesh is removed");
}
