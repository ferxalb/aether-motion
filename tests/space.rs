//! 2.5D spaces (`data-hf-space`): projection (perspective and orthographic), hierarchical plane
//! transforms, depth order, visibility (near clipping, back faces, timing), camera motion, scene
//! instances, deterministic output across render order, threads, and segments, and the errors that
//! keep planes inside a space.

mod common;

use aether_motion::agent::check;
use aether_motion::encode::{
    assemble_parts, encode_native, render_part, segment_count, EncodeOptions,
};
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

/// A 400x200 composition (black) holding `body`, lasting 2 s.
fn doc(body: &str) -> String {
    format!(
        "<div data-composition-id='space' data-width='400' data-height='200' data-duration='2' style='background:#000'>{body}</div>"
    )
}

/// A full-frame space with `attrs`, holding `planes`.
fn space(attrs: &str, planes: &str) -> String {
    doc(&format!(
        "<div id='stage' style='left:0;top:0;width:400px;height:200px' {attrs}>{planes}</div>"
    ))
}

fn shot_at(html: &str, time: f64) -> Shot {
    let dir = scratch("space");
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
    p[0] > 160 && p[1] < 80 && p[2] < 80
}

fn blue(p: [u8; 3]) -> bool {
    p[2] > 160 && p[0] < 80 && p[1] < 80
}

/// `(width, height, center x, center y)` of the red pixels.
fn red_box(shot: &Shot) -> (f64, f64, f64, f64) {
    let (l, t, r, b) = shot.bbox(red).expect("red plane on screen");
    let (w, h) = ((r + 1 - l) as f64, (b + 1 - t) as f64);
    (w, h, l as f64 + w / 2.0, t as f64 + h / 2.0)
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

/// A 100x50 red plane centered at stage `(x, y)` px from the center, with extra attributes.
fn red_plane(x: i32, y: i32, attrs: &str) -> String {
    format!(
        "<div id='red' style='left:{}px;top:{}px;width:100px;height:50px;background:#f00' {attrs}></div>",
        150 + x,
        75 + y
    )
}

#[test]
fn depth_scales_planes_like_css_perspective() {
    let at = |z: i32| {
        red_box(&shot_at(
            &space(
                "data-hf-space",
                &red_plane(30, 0, &format!("data-hf-z='{z}'")),
            ),
            0.0,
        ))
    };
    let (w, h, cx, cy) = at(0);
    assert!(
        close(w, 100.0, 1.0)
            && close(h, 50.0, 1.0)
            && close(cx, 230.0, 0.5)
            && close(cy, 100.0, 0.5)
    );
    // Default perspective 1000 px: z = -1000 halves the plane and its offset from the center.
    let (w, h, cx, _) = at(-1000);
    assert!(
        close(w, 50.0, 1.0) && close(h, 25.0, 1.0) && close(cx, 215.0, 0.5),
        "{w} {h} {cx}"
    );
    let (w, _, cx, _) = at(500);
    assert!(close(w, 200.0, 1.0) && close(cx, 260.0, 0.5), "{w} {cx}");
    // A shorter perspective exaggerates depth.
    let html = space(
        "data-hf-space='perspective' data-hf-perspective='500'",
        &red_plane(0, 0, "data-hf-z='-500'"),
    );
    assert!(close(red_box(&shot_at(&html, 0.0)).0, 50.0, 1.0));
    // Orthographic: depth orders but never scales.
    let html = space(
        "data-hf-space='orthographic'",
        &red_plane(60, 0, "data-hf-z='-1000'"),
    );
    let (w, _, cx, _) = red_box(&shot_at(&html, 0.0));
    assert!(close(w, 100.0, 1.0) && close(cx, 260.0, 0.5));
}

#[test]
fn a_flat_space_matches_the_classic_camera() {
    let camera = "data-hf-camera='[{\"at\":0,\"zoom\":1},{\"at\":2,\"zoom\":1.6,\"x\":40,\"y\":-10,\"easing\":\"ease-in-out\"}]'";
    let planes = red_plane(-30, 20, "");
    let classic = doc(&format!(
        "<div style='left:0;top:0;width:400px;height:200px' {camera}>{planes}</div>"
    ));
    let flat = space(&format!("data-hf-space {camera}"), &planes);
    for t in [0.0, 0.7, 1.3, 1.9] {
        let (a, b) = (red_box(&shot_at(&classic, t)), red_box(&shot_at(&flat, t)));
        for (x, y) in [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)] {
            assert!(close(x, y, 1.0), "{t}: classic {a:?} space {b:?}");
        }
    }
}

#[test]
fn camera_truck_gives_parallax_and_dolly_magnifies() {
    let planes = format!(
        "{}<div id='blue' data-hf-z='300' style='left:180px;top:120px;width:40px;height:20px;background:#00f'></div>",
        red_plane(0, -30, "data-hf-z='-600'")
    );
    let camera =
        "data-hf-space data-hf-camera='[{\"at\":0,\"x\":0,\"z\":0},{\"at\":2,\"x\":50,\"z\":300}]'";
    let html = space(camera, &planes);
    let (start, end) = (shot_at(&html, 0.0), shot_at(&html, 1.999));
    let shift = |shot: &Shot, test: fn([u8; 3]) -> bool| {
        let (l, _, r, _) = shot.bbox(test).unwrap();
        (l + r) as f64 / 2.0
    };
    // Trucking right moves the near (blue) plane left farther than the far (red) one.
    let far = shift(&start, red) - shift(&end, red);
    let near = shift(&start, blue) - shift(&end, blue);
    assert!(near > far * 3.0, "near {near} far {far}");
    // The 300 px dolly: the far plane grows from 1000/1600 to 1000/1300 of its size.
    let (w0, w1) = (red_box(&start).0, red_box(&end).0);
    assert!(
        close(w0, 62.5, 1.5) && close(w1, 1000.0 / 1300.0 * 100.0, 1.5),
        "{w0} {w1}"
    );
}

#[test]
fn nested_planes_inherit_transforms_and_draw_on_their_side() {
    // A child of a far plane is pushed back with it: its offset and size halve.
    let html = space(
        "data-hf-space",
        "<div data-hf-z='-1000' style='left:100px;top:50px;width:200px;height:100px'>\
           <div data-hf-z='0' style='left:150px;top:25px;width:40px;height:40px;background:#f00'></div>\
         </div>",
    );
    let (w, _, cx, cy) = red_box(&shot_at(&html, 0.0));
    assert!(
        close(w, 20.0, 1.0) && close(cx, 200.0 + 35.0, 0.6) && close(cy, 100.0 - 2.5, 0.6),
        "{w} {cx} {cy}"
    );
    // A tilted blue card with a red child plane in front of it (z = +20) and one behind (-20).
    let card = |z: i32| {
        space(
            "data-hf-space",
            &format!(
                "<div data-hf-rotate-y='35' style='left:100px;top:40px;width:200px;height:120px;background:#00f'>\
                   <div data-hf-z='{z}' style='left:70px;top:35px;width:60px;height:50px;background:#f00'></div>\
                 </div>"
            ),
        )
    };
    let front = shot_at(&card(20), 0.0);
    assert!(
        front.count(red) > 2000,
        "front child hidden: {}",
        front.count(red)
    );
    assert!(front.count(blue) > 10_000);
    let behind = shot_at(&card(-20), 0.0);
    assert!(
        behind.count(red) < 50,
        "child behind the card shows: {}",
        behind.count(red)
    );
    // The parent's turn carries the child: it is foreshortened like the card.
    let (w, h, _, _) = red_box(&front);
    assert!(w < 56.0 && h > 49.0, "{w} {h}");
}

#[test]
fn tilted_planes_foreshorten_and_planes_sort_by_depth() {
    // Orthographic: a 60 degree turn halves the width; a 60 degree tilt halves the height.
    let turned = space(
        "data-hf-space='orthographic'",
        &red_plane(0, 0, "data-hf-rotate-y='60'"),
    );
    let (w, h, cx, cy) = red_box(&shot_at(&turned, 0.0));
    assert!(
        close(w, 50.0, 1.5)
            && close(h, 50.0, 1.0)
            && close(cx, 200.0, 0.6)
            && close(cy, 100.0, 0.6)
    );
    let tilted = space(
        "data-hf-space='orthographic'",
        &red_plane(0, 0, "data-hf-rotate-x='60'"),
    );
    let (w, h, _, _) = red_box(&shot_at(&tilted, 0.0));
    assert!(close(w, 100.0, 1.5) && close(h, 25.0, 1.5), "{w} {h}");
    // Perspective: the edge turned away from the camera is shorter than the near one.
    let perspective = space(
        "data-hf-space data-hf-perspective='300'",
        &red_plane(0, 0, "data-hf-rotate-y='70'"),
    );
    let shot = shot_at(&perspective, 0.0);
    let (l, _, r, _) = shot.bbox(red).unwrap();
    let column = |x: usize| (0..shot.height).filter(|&y| red(shot.at(x, y))).count();
    assert!(
        column(l + 2) > column(r - 2) + 8,
        "{} {}",
        column(l + 2),
        column(r - 2)
    );
    // Depth, not document order, decides what is in front.
    let html = space(
        "data-hf-space",
        &format!(
            "{}<div data-hf-z='-200' style='left:150px;top:75px;width:100px;height:50px;background:#00f'></div>",
            red_plane(0, 0, "data-hf-z='100'")
        ),
    );
    let shot = shot_at(&html, 0.0);
    assert!(red(shot.at(200, 100)));
}

#[test]
fn planes_behind_the_camera_and_back_faces_are_not_drawn() {
    // Perspective 1000: the near distance is 100 px, so z = 950 is clipped.
    let gone = space("data-hf-space", &red_plane(0, 0, "data-hf-z='950'"));
    assert_eq!(shot_at(&gone, 0.0).count(red), 0);
    let back =
        |attrs: &str| shot_at(&space("data-hf-space", &red_plane(0, 0, attrs)), 0.0).count(red);
    assert!(back("data-hf-rotate-y='180'") > 4000);
    assert_eq!(back("data-hf-rotate-y='180' data-hf-backface='hidden'"), 0);
    assert!(back("data-hf-rotate-y='20' data-hf-backface='hidden'") > 3000);
    // A floor reaching past the camera is cut at the near distance and still covers the bottom.
    let floor = space(
        "data-hf-space",
        "<div data-hf-rotate-x='90' data-hf-z='0' style='left:-800px;top:-1840px;width:2000px;height:4000px;background:#f00'></div>",
    );
    let shot = shot_at(&floor, 0.0);
    assert!(red(shot.at(200, 199)) && red(shot.at(5, 199)) && !red(shot.at(200, 90)));
    // Clip timing and fades apply to planes like any element.
    let timed = space(
        "data-hf-space",
        &red_plane(0, 0, "class='clip' data-start='1' data-duration='1'"),
    );
    assert_eq!(shot_at(&timed, 0.5).count(red), 0);
    assert!(shot_at(&timed, 1.5).count(red) > 4000);
}

#[test]
fn keyframed_depth_and_tilt_animate() {
    let html = space(
        "data-hf-space",
        &red_plane(
            0,
            0,
            "data-hf-keyframes='[{\"at\":0,\"z\":-1000,\"rotateY\":0},{\"at\":2,\"z\":0,\"rotateY\":0,\"easing\":\"ease-out\"}]'",
        ),
    );
    assert!(close(red_box(&shot_at(&html, 0.0)).0, 50.0, 1.0));
    let mid = red_box(&shot_at(&html, 1.0)).0;
    assert!(mid > 60.0 && mid < 100.0, "{mid}");
    let card = space(
        "data-hf-space='orthographic'",
        &red_plane(
            0,
            0,
            "data-hf-keyframes='[{\"at\":0,\"rotateX\":0},{\"at\":2,\"rotateX\":60}]'",
        ),
    );
    assert!(close(red_box(&shot_at(&card, 1.999)).1, 25.0, 1.5));
}

#[test]
fn a_scene_instance_keeps_its_space() {
    let dir = scratch("space-scene");
    let scene = space(
        "data-hf-space data-hf-camera='[{\"at\":0,\"z\":0},{\"at\":2,\"z\":400,\"x\":30}]'",
        &format!(
            "{}<div data-hf-rotate-y='40' data-hf-z='-300' style='left:20px;top:20px;width:120px;height:80px;background:#00f'></div>",
            red_plane(40, 10, "data-hf-z='-500'")
        ),
    );
    let scene_path = write(&dir, "scene.html", &scene);
    let host = write(
        &dir,
        "host.html",
        "<div data-composition-id='host' data-width='400' data-height='200' data-duration='2' style='background:#000'>\
           <div id='shot' data-composition-src='scene.html' data-start='0' data-track-index='1'></div>\
         </div>",
    );
    let (scene, host) = (parse_file(&scene_path).unwrap(), parse_file(&host).unwrap());
    let mut a = Renderer::new(&scene, 1.0).unwrap();
    let mut b = Renderer::new(&host, 1.0).unwrap();
    for n in [0, 9, 23] {
        assert_eq!(
            a.render_frame(n, FPS).unwrap(),
            b.render_frame(n, FPS).unwrap(),
            "frame {n}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn frames_do_not_depend_on_order_threads_or_split() {
    let dir = scratch("space-order");
    let html = write(
        &dir,
        "index.html",
        &space(
            "data-hf-space data-hf-camera='[{\"at\":0,\"x\":-40,\"z\":0},{\"at\":2,\"x\":40,\"zoom\":1.2,\"z\":350,\"easing\":\"ease-in-out-cubic\"}]'",
            &format!(
                "<div data-hf-z='-1500' style='left:-600px;top:-300px;width:1600px;height:800px;background:linear-gradient(to bottom,#123,#456)'></div>\
                 <div data-hf-rotate-x='90' style='left:-500px;top:-300px;width:1400px;height:1200px;background:linear-gradient(to bottom,#330,#cc6)'></div>\
                 {}\
                 <div data-hf-keyframes='[{{\"at\":0,\"rotateY\":-70}},{{\"at\":2,\"rotateY\":20}}]' style='left:120px;top:30px;width:160px;height:100px;background:#00f;border-radius:12px;box-shadow:0px 8px 20px rgba(0,0,0,0.5)'>\
                   <p style='position:absolute;left:10px;top:10px;font-size:24px;color:#fff'>Depth</p>\
                   <div data-hf-z='25' style='left:100px;top:60px;width:40px;height:24px;background:#fa0'></div>\
                 </div>",
                red_plane(-120, -40, "data-hf-z='-400' data-hf-bloom='1'")
            ),
        ),
    );
    let composition = parse_file(&html).unwrap();
    let frames = Renderer::new(&composition, 1.0).unwrap().frame_count(FPS);
    let mut forward = Renderer::new(&composition, 1.0).unwrap();
    let reference: Vec<Vec<u8>> = (0..frames)
        .map(|n| forward.render_frame(n, FPS).unwrap().to_vec())
        .collect();
    assert_ne!(reference[0], reference[frames - 1]);
    let mut backward = Renderer::new(&composition, 1.0).unwrap();
    for n in (0..frames).rev() {
        assert_eq!(
            backward.render_frame(n, FPS).unwrap(),
            &reference[n][..],
            "reverse {n}"
        );
    }
    let mp4 = |threads: u32| {
        let out = dir.join(format!("t{threads}.mp4"));
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        encode_native(&mut renderer, &out, FPS, threads).unwrap();
        std::fs::read(out).unwrap()
    };
    let one = mp4(1);
    assert_eq!(one, mp4(4));
    let segments = segment_count(frames, FPS).unwrap();
    let parts: Vec<PathBuf> = (0..segments)
        .map(|i| dir.join(format!("{i}.aseg")))
        .collect();
    for (i, part) in parts.iter().enumerate().rev() {
        let fresh = parse_file(&html).unwrap();
        let mut renderer = Renderer::new(&fresh, 1.0).unwrap();
        render_part(&mut renderer, part, &EncodeOptions::new(FPS, 2), i..i + 1).unwrap();
    }
    let assembled = dir.join("assembled.mp4");
    assemble_parts(&composition, &parts, &assembled, 192_000).unwrap();
    assert_eq!(one, std::fs::read(assembled).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_example_checks_and_moves() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/depth-lantern-valley.html");
    let report = check(&path);
    assert_eq!(report["ok"], true, "{report}");
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 0.25).unwrap();
    let first = renderer.render_frame(0, 30).unwrap().to_vec();
    let last = renderer.render_frame(179, 30).unwrap().to_vec();
    assert_ne!(first, last);
}

#[test]
fn invalid_spaces_are_explained() {
    let dir = scratch("space-errors");
    let cases = [
        (doc(&red_plane(0, 0, "data-hf-z='-100'")), "no data-hf-space ancestor"),
        (space("data-hf-z='10'", ""), "no data-hf-space ancestor"),
        (
            doc("<div style='left:0;top:0;width:400px;height:200px' data-hf-camera='[{\"at\":0,\"z\":10}]'></div>"),
            "need data-hf-space on the camera",
        ),
        (
            doc("<div style='left:0;top:0;width:400px;height:200px' data-hf-shots='[{\"preset\":\"static\",\"rotateY\":10}]'></div>"),
            "need data-hf-space on the camera",
        ),
        (
            space(
                "data-hf-space data-hf-camera-subject='red' data-hf-camera='[{\"at\":0,\"rotateY\":5}]'",
                &red_plane(0, 0, ""),
            ),
            "cannot be framed by a camera that also turns",
        ),
        (space("data-hf-perspective='800'", ""), "needs data-hf-space"),
        (space("data-hf-space='isometric'", ""), "perspective or orthographic"),
        (space("data-hf-space='orthographic' data-hf-perspective='900'", ""), "only applies"),
        (space("data-hf-space", &red_plane(0, 0, "data-hf-backface='maybe'")), "visible or hidden"),
    ];
    for (html, expected) in cases {
        let path = write(&dir, "index.html", &html);
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(
            error.contains(expected),
            "expected {expected:?} in: {error}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn intersecting_planes_are_split_where_they_cross() {
    // An X seen from the front: left of center the red plane is nearer, right of it the blue one.
    let html = space(
        "data-hf-space",
        "<div data-hf-rotate-y='45' style='left:120px;top:50px;width:160px;height:100px;background:#f00'></div>\
         <div data-hf-rotate-y='-45' style='left:120px;top:50px;width:160px;height:100px;background:#00f'></div>",
    );
    let shot = shot_at(&html, 0.0);
    assert!(red(shot.at(170, 100)), "{:?}", shot.at(170, 100));
    assert!(blue(shot.at(230, 100)), "{:?}", shot.at(230, 100));
}

#[test]
fn a_far_card_standing_on_a_long_floor_stays_in_front_of_the_floor_behind_it() {
    // Sorting by centers would draw the floor (center nearer) over the card's foot.
    let html = space(
        "data-hf-space",
        &format!(
            "{}<div data-hf-rotate-x='90' data-hf-z='-400' style='left:-800px;top:-1850px;width:2000px;height:4000px;background:#00f'></div>",
            "<div data-hf-z='-1500' style='left:150px;top:50px;width:100px;height:100px;background:#f00'></div>"
        ),
    );
    let shot = shot_at(&html, 0.0);
    assert!(red(shot.at(200, 117)), "{:?}", shot.at(200, 117));
    assert!(blue(shot.at(200, 150)), "{:?}", shot.at(200, 150));
}

#[test]
fn nested_spaces_and_flattening_elements_render_their_planes() {
    // A space turned inside another space: its own plane draws inside it, foreshortened.
    let flat = space(
        "data-hf-space",
        "<div data-hf-space style='left:100px;top:40px;width:200px;height:120px'>\
           <div data-hf-z='50' style='left:50px;top:30px;width:100px;height:60px;background:#f00'></div>\
         </div>",
    );
    let turned = flat.replace(
        "<div data-hf-space style=",
        "<div data-hf-space data-hf-rotate-y='50' style=",
    );
    let (w_flat, _, _, _) = red_box(&shot_at(&flat, 0.0));
    let (w_turned, _, _, _) = red_box(&shot_at(&turned, 0.0));
    assert!(w_turned < w_flat * 0.8, "{w_turned} {w_flat}");
    // A plane inside overflow: hidden is flattened into it and clipped by its box.
    let clipped = space(
        "data-hf-space",
        "<div data-hf-rotate-y='20' style='left:100px;top:50px;width:200px;height:100px;overflow:hidden;background:#00f'>\
           <div data-hf-z='40' style='left:150px;top:10px;width:120px;height:80px;background:#f00'></div>\
         </div>",
    );
    let shot = shot_at(&clipped, 0.0);
    let (_, _, right, _) = shot.bbox(red).expect("clipped plane");
    let (_, _, blue_right, _) = shot.bbox(blue).expect("card");
    assert!(right <= blue_right + 1, "{right} {blue_right}");
}

#[test]
fn the_camera_pans_tilts_and_rolls() {
    let plane = red_plane(0, 0, "data-hf-z='-500'");
    let at = |turn: &str| {
        red_box(&shot_at(
            &space(
                &format!("data-hf-space data-hf-camera='[{{\"at\":0,{turn}}}]'"),
                &plane,
            ),
            0.0,
        ))
    };
    let (_, _, cx, _) = at("\"rotateY\":3");
    assert!(cx < 180.0, "pan right moves the scene left: {cx}");
    let (_, _, _, cy) = at("\"rotateX\":3");
    assert!(cy > 120.0, "tilt up moves the scene down: {cy}");
    let (w, h, cx, cy) = at("\"rotate\":90");
    assert!(
        close(w, 33.3, 1.5)
            && close(h, 66.7, 1.5)
            && close(cx, 200.0, 1.0)
            && close(cy, 100.0, 1.0),
        "{w} {h}"
    );
}

#[test]
fn smooth_rig_keyframes_and_shots_dolly_the_camera() {
    let plane = red_plane(0, 0, "");
    for camera in [
        "data-hf-camera='[{\"at\":0,\"z\":0},{\"at\":2,\"z\":500,\"easing\":\"smooth\"}]'",
        "data-hf-shots='[{\"preset\":\"static\",\"duration\":1},{\"preset\":\"static\",\"z\":500}]'",
    ] {
        let html = space(&format!("data-hf-space {camera}"), &plane);
        assert!(close(red_box(&shot_at(&html, 0.0)).0, 100.0, 1.0), "{camera}");
        assert!(close(red_box(&shot_at(&html, 1.999)).0, 200.0, 1.5), "{camera}");
    }
}

#[test]
fn parallax_children_of_a_space_camera_pan_by_their_factor() {
    let html = space(
        "data-hf-space data-hf-camera='[{\"at\":0,\"x\":0},{\"at\":2,\"x\":40}]'",
        &format!(
            "{}<div data-hf-parallax='0.5' style='left:180px;top:150px;width:40px;height:20px;background:#00f'></div>",
            red_plane(0, -40, "")
        ),
    );
    let (a, b) = (shot_at(&html, 0.0), shot_at(&html, 1.999));
    let center = |shot: &Shot, test: fn([u8; 3]) -> bool| {
        let (l, _, r, _) = shot.bbox(test).unwrap();
        (l + r) as f64 / 2.0
    };
    let full = center(&a, red) - center(&b, red);
    let half = center(&a, blue) - center(&b, blue);
    assert!(
        close(full, 40.0, 1.0) && close(half, 20.0, 1.0),
        "{full} {half}"
    );
}

#[test]
fn a_space_camera_follows_a_subject_moving_in_depth() {
    let html = space(
        "data-hf-space data-hf-camera-subject='red' data-hf-camera-lag='0' data-hf-camera-anchor='0.25 0.5' \
         data-hf-camera='[{\"at\":0,\"z\":0,\"follow\":1},{\"at\":2,\"z\":300,\"follow\":1}]'",
        &red_plane(
            0,
            0,
            "data-hf-keyframes='[{\"at\":0,\"x\":0,\"z\":-600},{\"at\":2,\"x\":300,\"z\":-200}]'",
        ),
    );
    for t in [0.0, 0.7, 1.4, 1.999] {
        let (_, _, cx, cy) = red_box(&shot_at(&html, t));
        assert!(
            close(cx, 100.0, 1.5) && close(cy, 100.0, 1.0),
            "{t}: {cx} {cy}"
        );
    }
}

#[test]
fn a_plane_with_motion_blur_blurs_its_own_depth_motion() {
    let moving =
        "data-hf-keyframes='[{\"at\":0,\"x\":-150,\"z\":-900},{\"at\":2,\"x\":150,\"z\":400}]'";
    let sharp = space("data-hf-space", &red_plane(0, 0, moving));
    let blurred = space(
        "data-hf-space",
        &red_plane(0, 0, &format!("{moving} data-hf-motion-blur='on'")),
    );
    let (a, b) = (shot_at(&sharp, 1.0), shot_at(&blurred, 1.0));
    assert_ne!(a.pixels, b.pixels);
    // Partly covered pixels appear around the growing edge.
    let soft = |shot: &Shot| shot.count(|p| p[0] > 30 && p[0] < 220 && p[1] < 40);
    assert!(soft(&b) > soft(&a) + 20, "{} {}", soft(&b), soft(&a));
    assert_eq!(shot_at(&blurred, 1.0).pixels, b.pixels);
}

/// The probed box of the text element `label` and the bounds of the bright (text) pixels.
fn probe_and_ink(html: &str) -> ([f32; 4], (usize, usize, usize, usize)) {
    let dir = scratch("space-probe");
    let path = write(&dir, "index.html", html);
    let composition = parse_file(&path).unwrap();
    let label = composition
        .elements
        .iter()
        .position(|e| e.id.as_deref() == Some("label"))
        .unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let boxes = renderer.probe_text_at(0.0).unwrap();
    let found = boxes
        .iter()
        .find(|b| b.element == label)
        .expect("probed text")
        .rect;
    let (w, h) = renderer.dimensions();
    let shot = Shot {
        pixels: renderer.render_at(0.0).unwrap().to_vec(),
        width: w as usize,
        height: h as usize,
    };
    std::fs::remove_dir_all(dir).unwrap();
    let ink = shot
        .bbox(|p| p.iter().all(|&c| c > 150))
        .expect("text pixels");
    (found, ink)
}

#[test]
fn text_on_tilted_planes_is_probed_through_the_projection() {
    use aether_motion::space::math::{Affine, View};
    use aether_motion::space::Projection;
    let card = |turn: &str| {
        space(
            "data-hf-space",
            &format!(
                "<div {turn} style='left:100px;top:50px;width:200px;height:100px;background:#123'>\
                   <p id='label' style='position:absolute;left:10px;top:30px;font-size:30px;color:#fff'>Label</p>\
                 </div>"
            ),
        )
    };
    // The facing card gives the text box in the plane's own px (its center is the stage center).
    let (flat, _) = probe_and_ink(&card(""));
    let (turned, ink) = probe_and_ink(&card("data-hf-rotate-y='30'"));
    let view = View::new(
        Projection::Perspective { distance: 1000.0 },
        [200.0, 100.0],
        1.0,
        [0.0; 3],
        [0.0; 3],
        100.0,
    );
    let plane = Affine::pose([0.0; 3], 0.0, 30.0, 0.0, 1.0);
    let mut expected = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in [(0, 1), (2, 1), (2, 3), (0, 3)] {
        let local = [f64::from(flat[x]) - 200.0, f64::from(flat[y]) - 100.0, 0.0];
        let p = view.project(plane.apply(local));
        expected = [
            expected[0].min(p[0]),
            expected[1].min(p[1]),
            expected[2].max(p[0]),
            expected[3].max(p[1]),
        ];
    }
    for (got, want) in turned.iter().zip(expected) {
        assert!(close(f64::from(*got), want, 0.5), "{turned:?} {expected:?}");
    }
    // The ink lies inside the reported box.
    let (l, t, r, b) = ink;
    assert!(l as f32 >= turned[0] - 1.0 && r as f32 <= turned[2] + 1.0);
    assert!(t as f32 >= turned[1] - 1.0 && b as f32 <= turned[3] + 1.0);
    // Through two projections (a nested space turned inside a tilted space), the box stays close
    // to the ink instead of growing at every map.
    let nested = space(
        "data-hf-space",
        "<div data-hf-space data-hf-rotate-y='35' style='left:60px;top:30px;width:280px;height:140px;background:#123'>\
           <div data-hf-rotate-x='25' style='left:40px;top:20px;width:200px;height:100px'>\
             <p id='label' style='position:absolute;left:10px;top:30px;font-size:30px;color:#fff'>Label</p>\
           </div>\
         </div>",
    );
    let (boxed, (l, t, r, b)) = probe_and_ink(&nested);
    let ink_w = (r + 1 - l) as f32;
    let ink_h = (b + 1 - t) as f32;
    assert!(
        l as f32 >= boxed[0] - 1.0 && r as f32 <= boxed[2] + 1.0,
        "{boxed:?} {:?}",
        (l, t, r, b)
    );
    assert!(boxed[2] - boxed[0] < ink_w + 14.0, "{boxed:?} {ink_w}");
    assert!(boxed[3] - boxed[1] < ink_h + 18.0, "{boxed:?} {ink_h}");
}

#[test]
fn split_planes_share_a_hard_edge_without_showing_the_backdrop() {
    let html = space(
        "data-hf-space",
        "<div data-hf-rotate-y='45' style='left:120px;top:50px;width:160px;height:100px;background:#f00'></div>\
         <div data-hf-rotate-y='-45' style='left:120px;top:50px;width:160px;height:100px;background:#00f'></div>",
    );
    let shot = shot_at(&html, 0.0);
    let dark = (70..130)
        .flat_map(|y| (196..205).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = shot.at(x, y);
            u16::from(p[0]) + u16::from(p[2]) < 230
        })
        .count();
    assert_eq!(dark, 0, "the backdrop shows through the cut");
}

#[test]
fn projected_blends_and_nested_subjects_are_explained() {
    let dir = scratch("space-errors-2");
    let cases = [
        (
            space(
                "data-hf-space",
                "<div data-hf-rotate-y='20' style='left:100px;top:50px;width:200px;height:100px;background:#00f'>\
                   <div style='position:absolute;left:0px;top:0px;width:50px;height:50px;background:#f00;mix-blend-mode:screen'></div>\
                 </div>",
            ),
            "cannot be drawn on a projected plane",
        ),
        (
            space(
                "data-hf-space data-hf-camera-subject='red' data-hf-camera='[{\"at\":0,\"follow\":1}]'",
                &format!(
                    "<div data-hf-space style='left:0;top:0;width:400px;height:200px'>{}</div>",
                    red_plane(0, 0, "data-hf-z='-100'")
                ),
            ),
            "a nested space of the camera's space",
        ),
    ];
    for (html, expected) in cases {
        let path = write(&dir, "index.html", &html);
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(
            error.contains(expected),
            "expected {expected:?} in: {error}"
        );
    }
    // A blend on a plane that faces the camera is drawn as in 2D.
    let facing = space(
        "data-hf-space",
        "<div style='left:100px;top:50px;width:200px;height:100px;background:#00f'>\
           <div style='position:absolute;left:0px;top:0px;width:50px;height:50px;background:#f00;mix-blend-mode:screen'></div>\
         </div>",
    );
    assert!(shot_at(&facing, 0.0).count(|p| p[0] > 200 && p[2] > 200) > 2000);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn tilted_planes_keep_overflowing_children_and_filter_minified_detail() {
    let html = space(
        "data-hf-space",
        "<div data-hf-rotate-y='20' style='left:100px;top:50px;width:100px;height:50px;background:#00f'>\
           <div style='position:absolute;left:130px;top:0px;width:40px;height:40px;background:#f00'></div>\
         </div>",
    );
    assert!(shot_at(&html, 0.0).count(red) > 600);
    // Fine stripes on a floor: far rows average to grey instead of aliasing to black and white.
    let stripes = "<svg width='2000' height='4000' viewBox='0 0 2000 4000'><defs><pattern id='s' width='4' height='4' patternUnits='userSpaceOnUse'>\
                   <rect width='2' height='4' fill='#fff'/></pattern></defs><rect width='2000' height='4000' fill='url(#s)'/></svg>";
    let floor = space(
        "data-hf-space",
        &format!(
            "<div data-hf-rotate-x='90' data-hf-z='-1500' style='left:-800px;top:-1840px;width:2000px;height:4000px;background:#000'>\
               <div style='position:absolute;left:0px;top:0px;width:2000px;height:4000px'>{stripes}</div></div>"
        ),
    );
    let shot = shot_at(&floor, 0.0);
    let row = (101..140)
        .find(|&y| (150..250).all(|x| shot.at(x, y)[0] > 5))
        .expect("a far floor row");
    let jumps = (150..250)
        .filter(|&x| shot.at(x, row)[0].abs_diff(shot.at(x + 1, row)[0]) > 90)
        .count();
    assert!(jumps < 5, "row {row}: {jumps} jumps");
}
