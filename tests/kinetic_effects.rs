//! 0.1.6 kinetic text and effects: spring easing, combined split start values, automatic wrapping
//! with line motion, `data-hf-bloom`, per-element `data-hf-motion-blur`, and their diagnostics.
mod common;

use aether_motion::model::{Content, Easing, Spring};
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::{scratch, Shot};
use std::path::PathBuf;

fn write(name: &str, body: &str, duration: f64) -> PathBuf {
    write_with_root(name, "", body, duration)
}

/// A 320x180 composition whose root also carries `root` attributes.
fn write_with_root(name: &str, root: &str, body: &str, duration: f64) -> PathBuf {
    let dir = scratch(name);
    let html = dir.join("index.html");
    std::fs::write(
        &html,
        format!(
            "<div id='root' data-composition-id='t' data-width='320' data-height='180' data-duration='{duration}' style='background:#000' {root}>{body}</div>"
        ),
    )
    .unwrap();
    html
}

fn shot(renderer: &mut Renderer<'_>, frame: usize, fps: u32) -> Shot {
    let (width, height) = renderer.dimensions();
    Shot {
        pixels: renderer.render_frame(frame, fps).unwrap().to_vec(),
        width: width as usize,
        height: height as usize,
    }
}

fn parse_error(name: &str, body: &str) -> String {
    let html = write(name, body, 1.0);
    format!("{:#}", parse_file(&html).unwrap_err())
}

#[test]
fn spring_easing_overshoots_exactly_and_settles() {
    for (overshoot, oscillations) in [(0.05, 1), (0.15, 2), (0.4, 3), (0.6, 6)] {
        let spring = Spring::new(overshoot, oscillations).unwrap();
        let easing = Easing::Spring(spring);
        assert_eq!(easing.apply(0.0), 0.0);
        assert!((easing.apply(1.0) - 1.0).abs() < 1e-6);
        let peak = (0..=4096)
            .map(|i| easing.apply(i as f32 / 4096.0))
            .fold(f32::MIN, f32::max);
        assert!((peak - 1.0 - overshoot).abs() < 2e-3, "{overshoot}: {peak}");
        // At rest at the end: the last step barely moves.
        assert!((easing.apply(0.999) - 1.0).abs() < 1e-3);
        // Played backwards it mirrors.
        let back = easing.reversed();
        assert!((back.apply(0.3) - (1.0 - easing.apply(0.7))).abs() < 1e-6);
    }
    let critical = Easing::Spring(Spring::new(0.0, 2).unwrap());
    let values: Vec<f32> = (0..=100)
        .map(|i| critical.apply(i as f32 / 100.0))
        .collect();
    assert!(values.windows(2).all(|w| w[1] >= w[0] - 1e-6) && values[100] <= 1.0 + 1e-6);
    assert!(Spring::new(0.7, 2).is_none() && Spring::new(0.1, 0).is_none());
}

#[test]
fn spring_keyframes_parse_and_reject_bad_arguments() {
    let html = write(
        "spring",
        "<div id='a' data-start='0' data-duration='1' style='width:10px;height:10px;background:#fff' \
         data-hf-keyframes='[{\"at\":0,\"x\":0},{\"at\":1,\"x\":100,\"easing\":\"spring(0.2, 2)\"}]'></div>",
        1.0,
    );
    let composition = parse_file(&html).unwrap();
    let motion = &composition.elements[1].motion;
    let peak = (0..=1000)
        .map(|i| motion.pose(i as f64 / 1000.0).x)
        .fold(f32::MIN, f32::max);
    assert!((peak - 120.0).abs() < 0.3, "{peak}");
    let error = parse_error(
        "spring-bad",
        "<div data-start='0' data-duration='1' data-hf-keyframes='[{\"at\":0,\"x\":0},{\"at\":1,\"x\":1,\"easing\":\"spring(2)\"}]'></div>",
    );
    assert!(error.contains("spring takes an overshoot"), "{error}");
}

const WRAP_TEXT: &str = "one two three four five six seven eight";

fn wrapped(extra: &str) -> PathBuf {
    write(
        "wrap",
        &format!(
            "<div id='t' data-start='0' data-duration='2' style='left:10px;top:10px;width:150px;font-size:20px;color:#fff' \
             data-hf-split='words' data-hf-stagger='0.1' {extra}>{WRAP_TEXT}</div>"
        ),
        2.0,
    )
}

/// Word pieces of the split element in document order, with the start of their entrance.
fn word_starts(composition: &aether_motion::model::Composition) -> Vec<(String, f64)> {
    composition
        .elements
        .iter()
        .filter_map(|e| match &e.content {
            Content::Text(t) if !t.is_empty() => Some((t.clone(), e.motion.y.0[0].at)),
            _ => None,
        })
        .collect()
}

#[test]
fn wrapping_splits_lines_at_the_box_width_and_keeps_piece_timing() {
    let plain = parse_file(&wrapped("")).unwrap();
    let wrap = parse_file(&wrapped("data-hf-split-wrap")).unwrap();
    // Same words with the same stagger, wrapped or not.
    assert_eq!(word_starts(&plain), word_starts(&wrap));
    let container = wrap.ids["t"];
    let rows = &wrap.elements[container].children;
    assert!(rows.len() >= 3, "{} rows", rows.len());
    assert!(wrap.elements[container].layout.split_wrap.is_none());
    // Every line fits the 150 px box: drawn, each band of text stays inside it.
    let mut renderer = Renderer::new(&wrap, 1.0).unwrap();
    let frame = shot(&mut renderer, 19, 10);
    let bands = frame.bands();
    assert_eq!(bands.len(), rows.len(), "{bands:?}");
    for band in bands {
        let (left, right) = frame.extent(band);
        assert!(left >= 10 && right < 160, "{band:?}: {left}..{right}");
    }
    // The unwrapped row overflows the box.
    let mut renderer = Renderer::new(&plain, 1.0).unwrap();
    let frame = shot(&mut renderer, 19, 10);
    assert_eq!(frame.bands().len(), 1);
    assert!(frame.extent(frame.bands()[0]).1 > 160);
}

#[test]
fn wrapped_lines_animate_and_render_the_same_at_any_scale() {
    let html = wrapped(
        "data-hf-split-wrap data-hf-split-line-y='30' data-hf-split-line-stagger='0.5' data-hf-split-line-duration='0.4'",
    );
    let composition = parse_file(&html).unwrap();
    let rows = &composition.elements[composition.ids["t"]].children;
    for (i, &row) in rows.iter().enumerate() {
        let motion = &composition.elements[row].motion;
        assert_eq!(motion.y.0[0].at, i as f64 * 0.5);
        assert_eq!(motion.pose(0.0).y, 30.0);
    }
    // Line breaks are decided at the unscaled size: at 2x the same lines are drawn.
    let mut one = Renderer::new(&composition, 1.0).unwrap();
    let mut two = Renderer::new(&composition, 2.0).unwrap();
    assert_eq!(
        shot(&mut one, 19, 10).bands().len(),
        shot(&mut two, 19, 10).bands().len()
    );
}

#[test]
fn split_start_values_combine_and_masks_clip() {
    let html = write(
        "split-from",
        "<div id='t' data-start='0' data-duration='2' style='left:10px;top:10px;width:300px;font-size:20px;color:#fff' \
         data-hf-split='chars' data-hf-split-y='12' data-hf-split-scale='0.5' data-hf-split-rotate='-20' data-hf-split-opacity='0.25' \
         data-hf-split-order='reverse' data-hf-split-mask='words' data-hf-stagger='0.1'>ab cd</div>",
        2.0,
    );
    let composition = parse_file(&html).unwrap();
    let letters: Vec<_> = composition
        .elements
        .iter()
        .filter(|e| matches!(&e.content, Content::Text(t) if !t.is_empty()))
        .collect();
    assert_eq!(letters.len(), 4);
    let first = letters[0].motion.pose(0.0);
    assert_eq!(
        (first.y, first.scale, first.rotate, first.opacity),
        (12.0, 0.5, -20.0, 0.25)
    );
    // Reverse order: the last letter starts first.
    assert_eq!(letters[3].motion.y.0[0].at, 0.0);
    assert!((letters[0].motion.y.0[0].at - 0.3).abs() < 1e-9);
    // Each word's letter group is a clip box.
    assert!(letters
        .iter()
        .all(|l| composition.elements[l.parent.unwrap()]
            .style
            .overflow_hidden));
}

#[test]
fn split_options_need_split_and_valid_values() {
    let error = parse_error(
        "no-split",
        "<div data-start='0' data-hf-split-wrap>text</div>",
    );
    assert!(error.contains("needs data-hf-split"), "{error}");
    let error = parse_error(
        "bad-order",
        "<div data-start='0' data-hf-split='words' data-hf-split-order='random'>a b</div>",
    );
    assert!(
        error.contains("forward, reverse, center, or edges"),
        "{error}"
    );
    let error = parse_error(
        "line-timing",
        "<div data-start='0' data-hf-split='words' data-hf-split-line-stagger='0.2'>a b</div>",
    );
    assert!(error.contains("needs a line start value"), "{error}");
}

fn bloom_scene(bloom: &str) -> PathBuf {
    write(
        "bloom",
        &format!(
            "<div data-start='0' data-duration='1' style='left:0;top:0;width:320px;height:180px;background:linear-gradient(to right, #101010, #202020)'></div>\
             <div id='dot' data-start='0' data-duration='1' style='left:140px;top:70px;width:40px;height:40px;background:#fff' {bloom}></div>\
             <div id='gray' data-start='0' data-duration='1' style='left:20px;top:70px;width:40px;height:40px;background:#555' {bloom}></div>"
        ),
        1.0,
    )
}

#[test]
fn bloom_glows_around_bright_pixels_only_and_keeps_them_sharp() {
    let plain = parse_file(&bloom_scene("")).unwrap();
    let bloom = parse_file(&bloom_scene(
        "data-hf-bloom='1.5' data-hf-bloom-radius='16' data-hf-bloom-threshold='0.5'",
    ))
    .unwrap();
    let base = shot(&mut Renderer::new(&plain, 1.0).unwrap(), 0, 30);
    let lit = shot(&mut Renderer::new(&bloom, 1.0).unwrap(), 0, 30);
    // The white square keeps its pixels and edges.
    for (x, y) in [(140, 70), (179, 109), (160, 90)] {
        assert_eq!(lit.at(x, y), base.at(x, y));
    }
    // Its halo brightens the background around it, fading with distance.
    let near = lit.at(186, 90)[0] as i32 - base.at(186, 90)[0] as i32;
    let far = lit.at(200, 90)[0] as i32 - base.at(200, 90)[0] as i32;
    assert!(near > 20 && near > far && far > 0, "{near} {far}");
    // The gray square is under the threshold: no halo at all.
    for x in 0..90 {
        assert_eq!(lit.at(x, 90), base.at(x, 90), "x {x}");
    }
    // Beyond the reach nothing changes.
    assert_eq!(lit.at(300, 90), base.at(300, 90));
    assert_eq!(lit.at(160, 10), base.at(160, 10));
    // Deterministic, and scaling keeps the halo proportional.
    let again = shot(&mut Renderer::new(&bloom, 1.0).unwrap(), 0, 30);
    assert_eq!(again.pixels, lit.pixels);
    let big = shot(&mut Renderer::new(&bloom, 2.0).unwrap(), 0, 30);
    let big_base = shot(&mut Renderer::new(&plain, 2.0).unwrap(), 0, 30);
    let big_near = big.at(372, 180)[0] as i32 - big_base.at(372, 180)[0] as i32;
    assert!((big_near - near).abs() <= 12, "{big_near} against {near}");
}

#[test]
fn bloom_intensity_animates_from_its_static_value() {
    let html = bloom_scene(
        "data-hf-bloom='1' data-hf-keyframes='[{\"at\":0,\"x\":0},{\"at\":0.5,\"bloom\":3},{\"at\":1,\"bloom\":0}]'",
    );
    let composition = parse_file(&html).unwrap();
    let track = &composition.elements[composition.ids["dot"]].motion.fx.bloom;
    assert_eq!(track.0[0].value, 1.0);
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let glow = |shot: &Shot| shot.at(190, 90)[0];
    let start = glow(&shot(&mut renderer, 0, 10));
    let peak = glow(&shot(&mut renderer, 5, 10));
    let end = glow(&shot(&mut renderer, 9, 10));
    assert!(peak > start && start > end, "{start} {peak} {end}");
}

#[test]
fn bloom_options_are_checked() {
    let error = parse_error(
        "bloom-opts",
        "<div data-start='0' data-hf-bloom-radius='10'>x</div>",
    );
    assert!(error.contains("need data-hf-bloom"), "{error}");
    let error = parse_error(
        "bloom-kf",
        "<div data-start='0' data-duration='1' data-hf-keyframes='[{\"at\":0,\"bloom\":1}]'>x</div>",
    );
    assert!(
        error.contains("keyframe bloom needs data-hf-bloom"),
        "{error}"
    );
    let error = parse_error(
        "bloom-range",
        "<div data-start='0' data-hf-bloom='9'>x</div>",
    );
    assert!(error.contains("from 0 to 8"), "{error}");
}

/// A static striped background and a white box moving 200 px per second.
fn blur_scene(root: &str, layer: &str) -> PathBuf {
    write_with_root(
        "layer-blur",
        root,
        &format!(
            "<div data-start='0' data-duration='2' style='left:0;top:0;width:320px;height:180px'>\
               <div style='position:absolute;left:0;top:120px;width:320px;height:4px;background:#888'></div>\
               <div style='position:absolute;left:150px;top:0;width:3px;height:180px;background:#888'></div></div>\
             <div id='box' data-start='0' data-duration='2' data-track-index='1' style='left:20px;top:40px;width:40px;height:40px;background:#fff' {layer} \
              data-hf-keyframes='[{{\"at\":0.5,\"x\":0}},{{\"at\":1.5,\"x\":200}}]'></div>"
        ),
        2.0,
    )
}

#[test]
fn layer_blur_smears_the_moving_element_and_leaves_the_background_sharp() {
    let sharp = parse_file(&blur_scene("", "")).unwrap();
    let blurred = parse_file(&blur_scene(
        "",
        "data-hf-motion-blur='on' data-hf-motion-blur-shutter='360' data-hf-motion-blur-samples='16'",
    ))
    .unwrap();
    let fps = 10;
    let a = shot(&mut Renderer::new(&sharp, 1.0).unwrap(), 10, fps);
    let b = shot(&mut Renderer::new(&blurred, 1.0).unwrap(), 10, fps);
    // At frame 10 (t = 1 s) the box is at x = 120..160 and moves 20 px per frame.
    let gray = |s: &Shot, y: usize| {
        (100..200)
            .filter(|&x| (20..235).contains(&s.at(x, y)[0]))
            .count()
    };
    assert_eq!(gray(&a, 60), 0);
    assert!(gray(&b, 60) >= 30, "{}", gray(&b, 60));
    // Rows away from the box are byte-identical: the static background stays sharp.
    for y in (0..30).chain(90..180) {
        for x in 0..320 {
            assert_eq!(a.at(x, y), b.at(x, y), "({x}, {y})");
        }
    }
    // Before the motion starts the frame is drawn once, sharp, like the plain scene.
    let still_a = shot(&mut Renderer::new(&sharp, 1.0).unwrap(), 2, fps);
    let still_b = shot(&mut Renderer::new(&blurred, 1.0).unwrap(), 2, fps);
    assert_eq!(still_a.pixels, still_b.pixels);
}

#[test]
fn layer_blur_matches_the_root_shutter_when_only_that_layer_moves() {
    let shutter = "data-motion-blur-shutter='270' data-motion-blur-samples='12'";
    let root = parse_file(&blur_scene(&format!("data-motion-blur='on' {shutter}"), "")).unwrap();
    let layer = parse_file(&blur_scene(
        "",
        "data-hf-motion-blur='on' data-hf-motion-blur-shutter='270' data-hf-motion-blur-samples='12'",
    ))
    .unwrap();
    for frame in [6, 10, 14] {
        let a = shot(&mut Renderer::new(&root, 1.0).unwrap(), frame, 10);
        let b = shot(&mut Renderer::new(&layer, 1.0).unwrap(), frame, 10);
        assert_eq!(a.pixels, b.pixels, "frame {frame}");
    }
}

#[test]
fn layer_blur_stills_match_sequential_frames_at_the_same_fps() {
    let html = blur_scene("", "data-hf-motion-blur='on'");
    let composition = parse_file(&html).unwrap();
    let mut sequential = Renderer::new(&composition, 1.0).unwrap();
    let frames: Vec<Vec<u8>> = (0..20)
        .map(|i| sequential.render_frame(i, 10).unwrap().to_vec())
        .collect();
    for frame in [0, 7, 11, 19] {
        let mut fresh = Renderer::new(&composition, 1.0).unwrap();
        assert_eq!(fresh.render_frame(frame, 10).unwrap(), &frames[frame][..]);
    }
    // A different frame rate opens a different shutter window.
    let mut fast = Renderer::new(&composition, 1.0).unwrap();
    assert_ne!(fast.render_frame(20, 20).unwrap(), &frames[10][..]);
}

#[test]
fn layer_blur_combinations_are_diagnosed() {
    let error = format!(
        "{:#}",
        parse_file(&blur_scene(
            "data-motion-blur='on'",
            "data-hf-motion-blur='on'"
        ))
        .unwrap_err()
    );
    assert!(
        error.contains("cannot be combined with data-motion-blur on the root"),
        "{error}"
    );
    let error = parse_error(
        "nested",
        "<div id='outer' data-start='0' data-hf-motion-blur='on'><div id='inner' data-hf-motion-blur='on'>x</div></div>",
    );
    assert!(error.contains("#inner is inside #outer"), "{error}");
    let dir = scratch("root-blur");
    let html = dir.join("index.html");
    std::fs::write(
        &html,
        "<div data-composition-id='t' data-width='32' data-height='18' data-duration='1' data-hf-motion-blur='on'></div>",
    )
    .unwrap();
    let error = format!("{:#}", parse_file(&html).unwrap_err());
    assert!(error.contains("cannot be on the root"), "{error}");
    let error = parse_error(
        "blur-samples",
        "<div data-start='0' data-hf-motion-blur='on' data-hf-motion-blur-samples='100'>x</div>",
    );
    assert!(error.contains("from 2 to 64"), "{error}");
}
