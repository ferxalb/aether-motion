//! Camera Rig v1 and the `data-hf-shots` composer: rendered subject framing, smooth and continuous
//! camera curves on `examples/cinematic-camera-rig-6s.html`, identical frames in any render order,
//! thread count, and split, unchanged classic cameras, and diagnostics for invalid rigs.

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

fn write(dir: &Path, html: &str) -> PathBuf {
    let path = dir.join("index.html");
    std::fs::write(&path, html).unwrap();
    path
}

/// A 160x90 street: two parallax planes of stripes, and a red subject on a 1.5x plane that walks
/// with stepped and eased keyframes. The camera follows it through three shots.
fn street(camera: &str) -> String {
    format!(
        "<div data-composition-id='rig' data-width='160' data-height='90' data-duration='3' style='background:#000'>\
         <div id='cam' style='left:0;top:0;width:160px;height:90px' {camera}>\
           <div data-hf-parallax='0.4' style='left:-200px;top:0;width:560px;height:90px'>\
             <div style='left:0;top:0;width:40px;height:90px;background:#1a1a1a'></div>\
             <div style='left:120px;top:0;width:40px;height:90px;background:#1a1a1a'></div>\
             <div style='left:240px;top:0;width:40px;height:90px;background:#1a1a1a'></div>\
             <div style='left:360px;top:0;width:40px;height:90px;background:#1a1a1a'></div>\
           </div>\
           <div data-hf-parallax='1.5' style='left:-300px;top:0;width:760px;height:90px'>\
             <div style='left:300px;top:60px;width:760px;height:4px;background:#444'></div>\
             <div id='hero' style='left:330px;top:40px;width:6px;height:6px;background:#f00'\
               data-hf-keyframes='[{{\"at\":0,\"x\":0}},{{\"at\":1.4,\"x\":40,\"easing\":\"ease-in-out\"}},{{\"at\":3,\"x\":90,\"y\":-8,\"easing\":\"ease-in\"}}]'></div>\
           </div>\
         </div><div id='outside' style='left:0;top:0;width:1px;height:1px'></div></div>"
    )
}

fn red_center(shot: &Shot) -> (f64, f64) {
    let (l, t, r, b) = shot
        .bbox(|[r, g, b]| r > 160 && g < 80 && b < 80)
        .expect("subject on screen");
    ((l + r + 1) as f64 / 2.0, (t + b + 1) as f64 / 2.0)
}

fn frame(renderer: &mut Renderer, n: usize) -> Shot {
    let pixels = renderer.render_frame(n, FPS).unwrap().to_vec();
    let (width, height) = renderer.dimensions();
    Shot {
        pixels,
        width: width as usize,
        height: height as usize,
    }
}

#[test]
fn locked_follow_holds_the_subject_on_its_anchor_through_zoom_and_parallax() {
    let dir = scratch("rig-anchor");
    let html = write(
        &dir,
        &street(
            "data-hf-camera-subject='hero' data-hf-camera-lag='0' data-hf-camera-anchor='0.3 0.55' \
             data-hf-shots='[{\"preset\":\"follow\",\"duration\":1},{\"preset\":\"dolly-in\",\"duration\":1,\"zoom\":1.6,\"anchor\":[0.35,0.5]},{\"preset\":\"follow\"}]'",
        ),
    );
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    for n in 0..36 {
        let (x, y) = red_center(&frame(&mut renderer, n));
        let t = n as f64 / FPS as f64;
        // The anchor moves from (0.3, 0.55) to (0.35, 0.5) over the dolly, smoothly and monotonically.
        let (ax, ay) = if t <= 1.0 {
            (0.3, 0.55)
        } else if t >= 2.0 {
            (0.35, 0.5)
        } else {
            (f64::NAN, f64::NAN)
        };
        if ax.is_nan() {
            assert!(
                (0.3 * 160.0 - 1.0..=0.35 * 160.0 + 1.0).contains(&x),
                "x {x} at {t}"
            );
            assert!(
                (0.5 * 90.0 - 1.0..=0.55 * 90.0 + 1.0).contains(&y),
                "y {y} at {t}"
            );
        } else {
            assert!((x - ax * 160.0).abs() <= 1.0, "x {x} at {t}");
            assert!((y - ay * 90.0).abs() <= 1.0, "y {y} at {t}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn smoothed(dir: &Path) -> PathBuf {
    write(
        dir,
        &street(
            "data-hf-camera-subject='hero' data-hf-camera-lag='0.4' data-hf-camera-lookahead='0.3' \
             data-hf-camera-deadzone='3 2' \
             data-hf-shots='[{\"preset\":\"follow\",\"duration\":1,\"from\":{\"zoom\":1.2}},{\"preset\":\"dolly-in\",\"duration\":1},{\"preset\":\"reveal\",\"to\":[10,0]}]'",
        ),
    )
}

#[test]
fn tracked_frames_do_not_depend_on_render_order_threads_or_split() {
    let dir = scratch("rig-order");
    let html = smoothed(&dir);
    let composition = parse_file(&html).unwrap();
    let frames = Renderer::new(&composition, 1.0).unwrap().frame_count(FPS);
    let mut forward = Renderer::new(&composition, 1.0).unwrap();
    let reference: Vec<Vec<u8>> = (0..frames)
        .map(|n| forward.render_frame(n, FPS).unwrap().to_vec())
        .collect();
    let mut backward = Renderer::new(&composition, 1.0).unwrap();
    for n in (0..frames).rev() {
        assert_eq!(
            backward.render_frame(n, FPS).unwrap(),
            &reference[n][..],
            "reverse {n}"
        );
    }
    // Random access, each from a fresh parse and renderer, as a separate process would.
    for n in [17, 3, 35, 0, 22, 9] {
        let fresh = parse_file(&html).unwrap();
        let mut single = Renderer::new(&fresh, 1.0).unwrap();
        assert_eq!(
            single.render_frame(n, FPS).unwrap(),
            &reference[n][..],
            "random {n}"
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
    assert!(segments >= 3);
    let parts: Vec<PathBuf> = (0..segments)
        .map(|i| dir.join(format!("{i}.aseg")))
        .collect();
    for (i, part) in parts.iter().enumerate().rev() {
        let fresh = parse_file(&html).unwrap();
        let mut renderer = Renderer::new(&fresh, 1.0).unwrap();
        let options = EncodeOptions::new(FPS, 2);
        render_part(&mut renderer, part, &options, i..i + 1).unwrap();
    }
    let assembled = dir.join("assembled.mp4");
    assemble_parts(&composition, &parts, &assembled, 192_000).unwrap();
    assert_eq!(one, std::fs::read(assembled).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

/// Camera translation and scale of the demo's rig, sampled every millisecond.
fn demo_samples() -> (Vec<[f64; 3]>, aether_motion::model::Composition) {
    let composition = parse_file(Path::new("examples/cinematic-camera-rig-6s.html")).unwrap();
    let rig = &composition.elements[composition.ids["rig"]];
    let samples = (0..=6000)
        .map(|ms| {
            let pose = rig.motion.pose(ms as f64 / 1000.0);
            [f64::from(pose.x), f64::from(pose.y), f64::from(pose.scale)]
        })
        .collect();
    (samples, composition)
}

#[test]
fn demo_camera_is_continuous_unsnapping_and_lands_on_its_end_poses() {
    let (samples, composition) = demo_samples();
    let speed = |i: usize, c: usize| (samples[i + 1][c] - samples[i][c]) * 1000.0;
    for c in 0..3 {
        // Velocity changes by a bounded amount per millisecond: no jumps, snaps, or spikes.
        let limit = if c == 2 { 0.01 } else { 1.5 };
        let mut worst: f64 = 0.0;
        for i in 0..5998 {
            worst = worst.max((speed(i + 1, c) - speed(i, c)).abs());
        }
        assert!(worst < limit, "channel {c}: velocity jump {worst}");
    }
    // Zoom (scale = 1.2 x zoom): held, then a dolly-in, then a pull-back, each one-directional.
    let scale = |ms: usize| samples[ms][2];
    assert!((scale(0) - 1.2 * 1.35).abs() < 1e-5);
    assert!((scale(2000) - 1.2 * 1.35).abs() < 1e-5);
    assert!((scale(4000) - 1.2 * 1.6).abs() < 1e-5);
    assert!((scale(6000) - 1.2).abs() < 1e-5);
    for ms in 0..2000 {
        assert!((scale(ms + 1) - scale(ms)).abs() < 1e-6);
    }
    for ms in 2000..4000 {
        assert!(scale(ms + 1) >= scale(ms) - 1e-7);
    }
    for ms in 4000..6000 {
        assert!(scale(ms + 1) <= scale(ms) + 1e-7);
    }
    // The pull-back ends at rest on its focus (0, 40): translation -1.2 * focus.
    // Velocity decays to zero at the last key (cubic ease into rest), never reversing.
    for (c, end) in samples[6000].into_iter().enumerate() {
        assert!(speed(5999, c).abs() < 0.3, "end speed {}", speed(5999, c));
        // Any reversal in the follow-to-fixed blend stays far below a pixel.
        let direction = (end - samples[4000][c]).signum();
        let backwards: f64 = (4000..6000)
            .map(|i| ((samples[i + 1][c] - samples[i][c]) * -direction).max(0.0))
            .sum();
        eprintln!("channel {c}: reversal travel {backwards:.6} px");
        assert!(backwards < 0.05, "channel {c} reverses {backwards} px");
    }
    assert!(samples[6000][0].abs() < 1e-3);
    assert!((samples[6000][1] + 48.0).abs() < 1e-3);
    // While tracking (0-4 s), the walker stays on his anchor: 40% -> 42% across, 60% -> 62% down.
    let rig = &composition.elements[composition.ids["rig"]];
    let worker = &composition.elements[composition.ids["worker"]];
    let mut follow_range = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
    for ms in (0..=4000).step_by(10) {
        let t = ms as f64 / 1000.0;
        let camera = rig.motion.pose(t);
        let walk = worker.motion.pose(t);
        let zoom = f64::from(camera.scale);
        // Worker center relative to the camera center (1600x900 box), and his plane's pan.
        let (sx, sy) = (465.625 - 800.0 + f64::from(walk.x), 691.0 - 450.0);
        let screen_x = zoom * sx + 1.14 * f64::from(camera.x);
        let screen_y = zoom * sy + 1.14 * f64::from(camera.y);
        // Anchor offsets from the frame center, in output px (the box is 1920x1080 on screen).
        if ms <= 2000 {
            follow_range[0] = follow_range[0].min(screen_x);
            follow_range[1] = follow_range[1].max(screen_x);
            follow_range[2] = follow_range[2].min(screen_y);
            follow_range[3] = follow_range[3].max(screen_y);
        }
        assert!(
            (-0.1 * 1920.0 - 8.0..=-0.08 * 1920.0 + 8.0).contains(&screen_x),
            "x {screen_x} at {t}"
        );
        assert!(
            (0.1 * 1080.0 - 4.0..=0.12 * 1080.0 + 4.0).contains(&screen_y),
            "y {screen_y} at {t}"
        );
    }
    // Shot 1 holds the anchor (-192, +108 px from the center) within the walk's 12 Hz steps.
    eprintln!(
        "follow framing x {:.2}..{:.2}, y {:.2}..{:.2}",
        follow_range[0], follow_range[1], follow_range[2], follow_range[3]
    );
    assert!(follow_range[0] > -192.0 - 6.0 && follow_range[1] < -192.0 + 6.0);
    assert!((follow_range[2] - 108.0).abs() < 0.5 && (follow_range[3] - 108.0).abs() < 0.5);
}

#[test]
fn classic_camera_keyframes_keep_their_tracks() {
    let dir = scratch("rig-classic");
    let html = write(
        &dir,
        "<div data-composition-id='c' data-width='64' data-height='36' data-duration='2'>\
         <div id='cam' style='left:0;top:0;width:64px;height:36px' \
           data-hf-camera='[{\"at\":0,\"zoom\":1},{\"at\":2,\"zoom\":2,\"x\":10,\"easing\":\"ease-out\"}]'></div></div>",
    );
    let composition = parse_file(&html).unwrap();
    let motion = &composition.elements[1].motion;
    assert!(motion.rig.is_none());
    assert_eq!(motion.x.0.len(), 2);
    let end = motion.pose(2.0);
    assert_eq!((end.x, end.scale), (-20.0, 2.0));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_rigs_are_explained() {
    let dir = scratch("rig-errors");
    let cases = [
        ("data-hf-shots='[{\"preset\":\"follow\"}]'", "data-hf-camera-subject"),
        ("data-hf-camera-subject='nobody'", "unknown id: nobody"),
        ("data-hf-camera-subject='outside'", "must be inside the camera"),
        ("data-hf-camera-subject='hero' data-hf-shots='[{\"preset\":\"static\",\"duration\":5}]'", "longer than the camera clip"),
        ("data-hf-camera='[{\"at\":0}]' data-hf-shots='[{\"preset\":\"static\"}]'", "not both"),
        ("data-hf-camera-subject='hero' data-hf-camera-lagg='1'", "unknown camera option"),
        ("data-hf-camera-subject='hero' data-hf-shots='[{\"preset\":\"pan\"}]'", "needs to"),
        ("data-hf-camera='[{\"at\":0,\"follow\":1}]'", "needs a subject"),
    ];
    for (camera, expected) in cases {
        let report = check(&write(&dir, &street(camera)));
        assert_eq!(report["ok"], false, "{camera}");
        let error = report["error"].as_str().unwrap();
        assert!(error.contains(expected), "{camera}: {error}");
        assert!(
            report["fix"]
                .as_str()
                .unwrap()
                .contains("data-hf-camera-subject"),
            "{camera}: {}",
            report["fix"]
        );
    }
    let report = check(&smoothed(&dir));
    assert_eq!(report["ok"], true);
    let camera = &report["cameras"][0];
    assert_eq!(camera["subject"]["id"], "hero");
    assert_eq!(camera["subject"]["depth"], 1.5);
    assert_eq!(camera["keys"].as_array().unwrap().len(), 4);
    std::fs::remove_dir_all(dir).unwrap();
}
