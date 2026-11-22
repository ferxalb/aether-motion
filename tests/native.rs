use aether_motion::encode::encode_native;
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

fn fixture() -> &'static Path {
    Path::new("examples/title-card.html")
}

#[test]
fn resolves_reference_timing_and_reuses_static_frames() {
    let composition = parse_file(fixture()).unwrap();
    assert_eq!(composition.width, 640);
    assert_eq!(composition.height, 360);
    assert_eq!(composition.duration, 4.0);
    let intro = &composition.elements[composition.ids["intro"]];
    let outro = &composition.elements[composition.ids["outro"]];
    assert_eq!((intro.start, intro.end), (0.0, 2.0));
    assert_eq!((outro.start, outro.end), (2.0, 4.0));
    let heading = &composition.elements[intro.children[0]];
    assert_eq!(heading.style.font_size, Some(56.0));

    let mut renderer = Renderer::new(&composition, 0.5).unwrap();
    assert_eq!(renderer.dimensions(), (320, 180));
    assert_eq!(renderer.frame_count(24), 96);
    let first = renderer.render_frame(24, 24).unwrap().to_vec();
    let second = renderer.render_frame(25, 24).unwrap().to_vec();
    assert_eq!(first, second);
    assert_eq!(renderer.stats().reused, 1);
    let later = renderer.render_frame(72, 24).unwrap().to_vec();
    assert_ne!(first, later);
}

#[test]
fn decodes_local_images_and_orders_tracks() {
    let dir = std::env::temp_dir().join(format!("aether-hf-image-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("red.png");
    let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
    pixels.save(&png).unwrap();
    let html = dir.join("image.html");
    std::fs::write(&html, "<div data-composition-id='x' data-width='8' data-height='8' data-duration='1'>\
        <img src='red.png' data-start='0' data-duration='1' data-track-index='1' style='left:2px;top:2px;width:4px;height:4px'>\
        <div data-start='0' data-duration='1' data-track-index='0' style='background:green'></div></div>").unwrap();
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let frame = renderer.render_frame(0, 1).unwrap();
    let at = |x: usize, y: usize| &frame[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
    assert_eq!(at(0, 0), &[0, 128, 0, 255]);
    assert_eq!(at(3, 3), &[255, 0, 0, 255]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn writes_native_mp4_with_expected_timing() {
    let dir = std::env::temp_dir().join(format!("aether-hf-mp4-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let html = dir.join("video.html");
    std::fs::write(&html, "<div data-composition-id='x' data-width='32' data-height='32' data-duration='1' style='background:red'></div>").unwrap();
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let output = dir.join("out.mp4");
    encode_native(&mut renderer, &output, 4, 1).unwrap();
    let file = File::open(&output).unwrap();
    let size = file.metadata().unwrap().len();
    let mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), size).unwrap();
    assert_eq!(mp4.sample_count(1).unwrap(), 4);
    assert_eq!(mp4.duration().as_secs_f64(), 1.0);
    assert_eq!(mp4.tracks()[&1].width(), 32);
    assert_eq!(mp4.tracks()[&1].height(), 32);
    drop(mp4);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_browser_only_script() {
    let dir = std::env::temp_dir().join(format!("aether-hf-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("script.html");
    std::fs::write(&path, "<div data-composition-id='x' data-width='16' data-height='16' data-duration='1'><script>run()</script></div>").unwrap();
    let error = parse_file(&path).unwrap_err().to_string();
    assert!(error.contains("<script>"), "{error}");
    std::fs::write(&path, "<div data-composition-id='x' data-width='16' data-height='16' data-duration='1'></div><script>run()</script>").unwrap();
    let error = parse_file(&path).unwrap_err().to_string();
    assert!(error.contains("<script>"), "{error}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_reports_ok_and_actionable_errors() {
    let ok = aether_motion::agent::check(fixture());
    assert_eq!(ok["ok"], true);
    let dir = std::env::temp_dir().join(format!("aether-hf-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.html");
    std::fs::write(&bad, r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="1"><script>1</script></div>"#).unwrap();
    let report = aether_motion::agent::check(&bad);
    assert_eq!(report["ok"], false);
    assert!(report["fix"].as_str().unwrap().contains("remove"));
}

#[test]
fn animates_nested_clips_deterministically_at_arbitrary_times() {
    let html = r#"<main data-composition-id="test" data-width="16" data-height="16" data-duration="4">
        <div id="motion" class="clip" data-start="1" data-duration="3" data-track-index="0"
             style="left:1px;top:1px;width:2px;height:2px"
             data-hf-keyframes='[{"at":0,"x":0,"opacity":0},{"at":1,"x":8,"y":4,"opacity":1,"easing":"ease-in"},{"at":2}]'>
            <div style="background:red"></div>
        </div>
    </main>"#;
    let dir = std::env::temp_dir().join(format!("aether-hf-motion-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("motion.html");
    std::fs::write(&path, html).unwrap();
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let pixel = |x: usize, y: usize| (y * 16 + x) * 4;
    let mid = renderer.render_at(1.5).unwrap().to_vec();
    assert_eq!(mid[pixel(3, 2)], 64);
    assert_eq!(mid[pixel(1, 1)], 0);
    let held = renderer.render_at(2.0).unwrap().to_vec();
    assert_eq!(held[pixel(9, 5)], 255);
    assert_eq!(renderer.render_at(3.5).unwrap(), held);
    assert_eq!(renderer.stats().reused, 1);
    assert_eq!(renderer.render_frame(3, 2).unwrap(), mid);
    let report = aether_motion::agent::check(&path);
    assert_eq!(
        report["animations"][0]["keyframeTimes"],
        serde_json::json!([0.0, 1.0, 2.0])
    );

    let invalid = html.replace(r#"{"at":2}"#, r#"{"at":1}"#);
    std::fs::write(&path, invalid).unwrap();
    let report = aether_motion::agent::check(&path);
    assert_eq!(report["ok"], false);
    assert!(report["error"].as_str().unwrap().contains("keyframe 3"));
    assert!(report["fix"].as_str().unwrap().contains("increasing"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn exports_full_hd_and_uhd_with_exact_cadence() {
    use aether_motion::render::OutputSize;

    let dir = std::env::temp_dir().join(format!("aether-hf-uhd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let html = dir.join("uhd.html");
    std::fs::write(&html, r#"<main data-composition-id="test" data-width="640" data-height="360" data-duration="0.1" style="background:#102030">
        <div style="width:80px;height:12px;background:white" data-hf-keyframes='[{"at":0},{"at":0.04,"x":40}]'></div>
    </main>"#).unwrap();
    let composition = parse_file(&html).unwrap();
    for (size, width, height) in [
        (OutputSize::FullHd, 1920, 1080),
        (OutputSize::Uhd, 3840, 2160),
    ] {
        for fps in [30, 60] {
            let output = dir.join(format!("{width}-{fps}.mp4"));
            let mut renderer = Renderer::with_output_size(&composition, size).unwrap();
            encode_native(&mut renderer, &output, fps, 2).unwrap();
            let file = File::open(&output).unwrap();
            let bytes = file.metadata().unwrap().len();
            let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), bytes).unwrap();
            assert_eq!(mp4.timescale(), fps);
            assert_eq!(mp4.sample_count(1).unwrap(), fps / 10);
            assert_eq!(mp4.duration().as_secs_f64(), 0.1);
            assert_eq!(
                (mp4.tracks()[&1].width(), mp4.tracks()[&1].height()),
                (width, height)
            );
            for tick in 0..fps / 10 {
                let sample = mp4.read_sample(1, tick + 1).unwrap().unwrap();
                assert_eq!((sample.start_time, sample.duration), (u64::from(tick), 1));
                assert!(!sample.bytes.is_empty());
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rounds_gradient_backgrounds_and_clears_pixels_left_by_motion() {
    let dir = std::env::temp_dir().join(format!("aether-hf-paint-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("paint.html");
    std::fs::write(&path, r#"<main data-composition-id="test" data-width="32" data-height="18" data-duration="2">
        <div style="width:12px;height:12px;border-radius:6px;background:linear-gradient(to right, red, blue)"
          data-hf-keyframes='[{"at":0},{"at":1,"x":16,"opacity":0.5}]'></div>
    </main>"#).unwrap();
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let offset = |x: usize, y: usize| (y * 32 + x) * 4;
    let first = renderer.render_at(0.0).unwrap().to_vec();
    assert_eq!(first[offset(0, 0)], 0);
    assert!(first[offset(0, 3)] > 0 && first[offset(0, 3)] < 244);
    assert!(first[offset(0, 5)] > 230);
    assert!(first[offset(11, 5) + 2] > 230);
    let moved = renderer.render_at(1.0).unwrap();
    assert_eq!(moved[offset(0, 5)], 0);
    assert_eq!(
        moved[offset(17, 5)],
        (f32::from(first[offset(1, 5)]) * 0.5).round() as u8
    );
    assert_eq!(renderer.render_at(0.0).unwrap(), first);
    std::fs::remove_dir_all(dir).unwrap();
}

fn motion_composition(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("aether-hf-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let html = dir.join("motion.html");
    std::fs::write(
        &html,
        r#"<div data-composition-id="m" data-width="64" data-height="48" data-duration="3" style="background:#102030">
<div id="a" class="clip" data-start="0" data-duration="3" data-track-index="0"
 style="left:4px;top:4px;width:20px;height:20px;background:#ff8800"
 data-hf-keyframes='[{"at":0,"x":0,"opacity":0},{"at":2,"x":30,"opacity":1,"easing":"ease-in-out"}]'></div></div>"#,
    )
    .unwrap();
    (dir, html)
}

#[test]
fn parallel_segments_are_byte_identical_for_any_thread_count() {
    let (dir, html) = motion_composition("segments");
    let composition = parse_file(&html).unwrap();
    let mut outputs = Vec::new();
    for threads in [1, 3] {
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        let output = dir.join(format!("out-{threads}.mp4"));
        // 4 fps x 3 s = 12 frames in three 1-second segments.
        encode_native(&mut renderer, &output, 4, threads).unwrap();
        assert_eq!(renderer.stats().rasterized + renderer.stats().reused, 12);
        outputs.push(output);
    }
    let (a, b) = (
        std::fs::read(&outputs[0]).unwrap(),
        std::fs::read(&outputs[1]).unwrap(),
    );
    assert_eq!(a, b, "MP4 output must not depend on the thread count");
    let size = a.len() as u64;
    let mut mp4 =
        mp4::Mp4Reader::read_header(BufReader::new(File::open(&outputs[0]).unwrap()), size)
            .unwrap();
    assert_eq!(mp4.sample_count(1).unwrap(), 12);
    let sync: Vec<u32> = (1..=12)
        .filter(|&i| mp4.read_sample(1, i).unwrap().unwrap().is_sync)
        .collect();
    assert_eq!(sync, [1, 5, 9], "an IDR at the start of each segment");
    drop(mp4);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn forked_renderer_matches_fresh_render_in_any_order() {
    let (dir, html) = motion_composition("fork");
    let composition = parse_file(&html).unwrap();
    let mut main = Renderer::new(&composition, 1.0).unwrap();
    let mut fork = main.fork().unwrap();
    for frame in [29, 2, 11, 0, 23, 24, 5] {
        let fresh = Renderer::new(&composition, 1.0)
            .unwrap()
            .render_frame(frame, 10)
            .unwrap()
            .to_vec();
        assert_eq!(
            fork.render_frame(frame, 10).unwrap(),
            fresh.as_slice(),
            "frame {frame}"
        );
    }
    assert!(main.render_frame(0, 10).is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

fn css_composition(name: &str, css: &str, body: &str) -> std::path::PathBuf {
    let html = format!(
        r#"<html><head><style>{css}</style></head><body>
        <main data-composition-id="css" data-width="16" data-height="16" data-duration="6">{body}</main>
        </body></html>"#
    );
    let dir = std::env::temp_dir().join(format!("aether-hf-css-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("css.html");
    std::fs::write(&path, html).unwrap();
    path
}

/// Red channel of the pixel at (x, y) in a 16x16 render at scale 1.
fn red_at(renderer: &mut Renderer, time: f64, x: usize, y: usize) -> u8 {
    renderer.render_at(time).unwrap()[(y * 16 + x) * 4]
}

#[test]
fn css_keyframes_animate_with_fill_modes() {
    let css = "@keyframes slide { from { transform: translate(0, 0) } to { transform: translate(8px, 0) } }
        .box { left: 0; top: 0; width: 4px; height: 4px; background: #f00 }";
    let hold = css_composition(
        "hold",
        css,
        r#"<div class="clip box" data-start="0" data-duration="6" style="animation: slide 2s linear forwards"></div>"#,
    );
    let composition = parse_file(&hold).unwrap();
    let mut r = Renderer::new(&composition, 1.0).unwrap();
    assert_eq!(red_at(&mut r, 0.0, 1, 1), 255);
    assert_eq!(red_at(&mut r, 1.0, 1, 1), 0, "moved 4px at the midpoint");
    assert_eq!(red_at(&mut r, 1.0, 5, 1), 255);
    assert_eq!(
        red_at(&mut r, 4.0, 9, 1),
        255,
        "forwards holds the end pose"
    );

    let reset = css_composition(
        "reset",
        css,
        r#"<div class="clip box" data-start="0" data-duration="6" style="animation: slide 2s linear"></div>"#,
    );
    let composition = parse_file(&reset).unwrap();
    let mut r = Renderer::new(&composition, 1.0).unwrap();
    assert_eq!(red_at(&mut r, 4.0, 1, 1), 255, "fill none returns to rest");
    assert_eq!(red_at(&mut r, 4.0, 9, 1), 0);
}

#[test]
fn css_delay_direction_and_iterations_are_a_pure_function_of_time() {
    let css =
        "@keyframes slide { from { transform: translateX(0) } to { transform: translateX(8px) } }
        .box { left: 0; top: 0; width: 4px; height: 4px; background: #f00 }";
    let path = css_composition(
        "loop",
        css,
        r#"<div class="clip box" data-start="0" data-duration="6"
            style="animation: slide 2s linear 1s infinite alternate both"></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut r = Renderer::new(&composition, 1.0).unwrap();
    assert_eq!(
        red_at(&mut r, 0.5, 1, 1),
        255,
        "backwards fill holds 0% during the delay"
    );
    // Iteration 0 forwards: at 2s elapsed 1s => +4px. Iteration 1 backwards: at 4s => +4px again.
    let forward = r.render_at(2.0).unwrap().to_vec();
    let backward = r.render_at(4.0).unwrap().to_vec();
    assert_eq!(forward, backward);
    // Same time reached in a different order renders the same bytes.
    let again = r.render_at(2.0).unwrap().to_vec();
    assert_eq!(forward, again);
    assert_eq!(red_at(&mut r, 3.0, 9, 1), 255, "end of the first leg");
}

#[test]
fn css_variables_color_functions_and_scale() {
    let css = ":root { --size: 4px; --accent: rgb(255 0 0) }
        .box { left: 6px; top: 6px; width: var(--size); height: var(--size, 9px); background: var(--accent);
               transform: scale(2) }";
    let path = css_composition(
        "vars",
        css,
        r#"<div class="clip box" data-start="0" data-duration="6"></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut r = Renderer::new(&composition, 1.0).unwrap();
    // 4px box centered at (8, 8), doubled: covers 4..12.
    assert_eq!(red_at(&mut r, 0.0, 4, 4), 255);
    assert_eq!(red_at(&mut r, 0.0, 11, 11), 255);
    assert_eq!(red_at(&mut r, 0.0, 3, 8), 0);
    assert_eq!(red_at(&mut r, 0.0, 12, 8), 0);
}

#[test]
fn css_animation_errors_are_explicit() {
    let cases = [
        (".a { animation: nope 1s }", "@keyframes nope"),
        (
            "@keyframes k { to { left: 5px } } .a { animation: k 1s }",
            "unsupported property: left",
        ),
        (".a { width: var(--missing) }", "CSS variable not defined"),
        (
            "@keyframes k { to { opacity: 0 } } .a { animation: k }",
            "animation-duration is missing",
        ),
        ("@media print { .a { width: 1px } }", "@"),
    ];
    for (i, (css, expected)) in cases.iter().enumerate() {
        let path = css_composition(
            &format!("err{i}"),
            css,
            r#"<div class="clip a" data-start="0" data-duration="2"></div>"#,
        );
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(error.contains(expected), "{css}: {error}");
    }
}

/// Parses `body` in a `width`x`height` composition and returns an RGB sampler at scale 1.
fn sized_render(
    name: &str,
    (width, height): (usize, usize),
    css: &str,
    body: &str,
) -> impl FnMut(usize, usize) -> [u8; 3] {
    let html = format!(
        r#"<html><head><style>{css}</style></head><body>
        <main data-composition-id="flex" data-width="{width}" data-height="{height}" data-duration="1">{body}</main>
        </body></html>"#
    );
    let dir = std::env::temp_dir().join(format!("aether-hf-flex-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("flex.html");
    std::fs::write(&path, html).unwrap();
    let composition = Box::leak(Box::new(parse_file(&path).unwrap()));
    let mut renderer = Renderer::new(composition, 1.0).unwrap();
    let pixels = renderer.render_at(0.0).unwrap().to_vec();
    move |x, y| {
        let i = (y * width + x) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    }
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLACK: [u8; 3] = [0, 0, 0];

#[test]
fn flex_row_centers_items_with_gap() {
    let mut at = sized_render(
        "row",
        (30, 10),
        ".row { left: 0; top: 0; width: 30px; height: 10px; display: flex; justify-content: center;
                align-items: center; gap: 2px }
         .a, .b { width: 4px; height: 4px } .a { background: #f00 } .b { background: #0f0 }",
        r#"<div class="clip row" data-start="0" data-duration="1">
             <div class="clip a" data-start="0" data-duration="1"></div>
             <div class="clip b" data-start="0" data-duration="1"></div></div>"#,
    );
    // Items span 10..14 and 16..20 horizontally, 3..7 vertically.
    assert_eq!(at(10, 3), RED);
    assert_eq!(at(13, 6), RED);
    assert_eq!(at(9, 3), BLACK);
    assert_eq!(at(14, 3), BLACK, "gap");
    assert_eq!(at(16, 3), GREEN);
    assert_eq!(at(19, 6), GREEN);
    assert_eq!(at(10, 2), BLACK);
    assert_eq!(at(10, 7), BLACK);
}

#[test]
fn flex_column_space_between_respects_padding() {
    let mut at = sized_render(
        "column",
        (10, 30),
        ".col { left: 0; top: 0; width: 10px; height: 30px; display: flex; flex-direction: column;
                justify-content: space-between; padding: 2px }
         .a { width: 6px; height: 4px; background: #f00 }",
        r#"<div class="clip col" data-start="0" data-duration="1">
             <div class="clip a" data-start="0" data-duration="1"></div>
             <div class="clip a" data-start="0" data-duration="1"></div></div>"#,
    );
    assert_eq!(at(2, 2), RED, "first item at the padding edge");
    assert_eq!(at(7, 5), RED);
    assert_eq!(at(8, 2), BLACK, "item width is 6px");
    assert_eq!(at(2, 1), BLACK, "padding");
    assert_eq!(at(2, 12), BLACK, "space between");
    assert_eq!(at(2, 24), RED, "last item at the bottom padding edge");
    assert_eq!(at(2, 27), RED);
    assert_eq!(at(2, 28), BLACK);
}

#[test]
fn flex_grow_shares_free_space_and_stretches_the_cross_axis() {
    let mut at = sized_render(
        "grow",
        (20, 4),
        ".row { left: 0; top: 0; width: 20px; height: 4px; display: flex }
         .a { flex: 1; background: #f00 } .b { flex: 3; background: #0f0 }",
        r#"<div class="clip row" data-start="0" data-duration="1">
             <div class="clip a" data-start="0" data-duration="1"></div>
             <div class="clip b" data-start="0" data-duration="1"></div></div>"#,
    );
    assert_eq!(at(4, 0), RED);
    assert_eq!(at(4, 3), RED, "stretched to the container height");
    assert_eq!(at(5, 0), GREEN);
    assert_eq!(at(19, 3), GREEN);
}

#[test]
fn box_shadow_falls_off_outside_and_stays_under_the_element() {
    let mut at = sized_render(
        "shadow",
        (24, 24),
        ".box { left: 8px; top: 8px; width: 8px; height: 8px; background: #00f;
                box-shadow: 0 0 6px 0 #f00 }",
        r#"<div class="clip box" data-start="0" data-duration="1"></div>"#,
    );
    let near = at(7, 12);
    let far = at(2, 12);
    assert!(
        near[0] > far[0] && near[0] > 40,
        "shadow is strongest at the edge: {near:?} {far:?}"
    );
    assert_eq!(near[2], 0);
    assert!(at(0, 12)[0] < 8, "only the Gaussian tail remains far away");
    assert_eq!(at(12, 12), [0, 0, 255], "the element covers its own shadow");
}

#[test]
fn flex_and_shadow_errors_are_explicit() {
    let cases = [
        (
            ".a { box-shadow: inset inset 0 0 4px #000 }",
            "inset given twice",
        ),
        (".a { flex-direction: row-reverse }", "flex-direction"),
        (".a { padding: -1px }", "non-negative"),
        (".a { gap: 10% }", "non-negative px"),
        (".a { border: 1px dashed red }", "unsupported CSS color"),
        (".a { justify-content: baseline }", "justify-content"),
    ];
    for (i, (css, expected)) in cases.iter().enumerate() {
        let path = css_composition(
            &format!("flexerr{i}"),
            css,
            r#"<div class="clip a" data-start="0" data-duration="2"></div>"#,
        );
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(error.contains(expected), "{css}: {error}");
    }
}

#[test]
fn border_is_drawn_inside_the_box_and_insets_flex_children() {
    let mut at = sized_render(
        "border",
        (24, 14),
        ".box { left: 2px; top: 2px; width: 10px; height: 10px; background: #00f; border: 2px solid #f00 }
         .row { left: 14px; top: 2px; width: 8px; height: 10px; display: flex; border: 2px solid #f00 }
         .kid { width: 2px; height: 2px; background: #0f0 }",
        r#"<div class="clip box" data-start="0" data-duration="1"></div>
           <div class="clip row" data-start="0" data-duration="1">
             <div class="clip kid" data-start="0" data-duration="1"></div></div>"#,
    );
    assert_eq!(at(2, 2), RED);
    assert_eq!(at(3, 7), RED);
    assert_eq!(at(4, 4), [0, 0, 255], "background shows inside the border");
    assert_eq!(at(11, 11), RED);
    assert_eq!(at(12, 11), BLACK, "border-box: nothing outside the box");
    assert_eq!(at(14, 2), RED);
    assert_eq!(at(16, 4), GREEN, "flex child starts inside the border");
    assert_eq!(at(15, 4), RED);
}

#[test]
fn rotate_turns_the_box_around_its_center_and_carries_children() {
    // A 4x2 box centered at (8, 8) rotated 90 degrees covers x 7..9, y 6..10.
    for transform in [
        "rotate(90deg)",
        "rotate(0.25turn)",
        "translate(0, 0) rotate(90deg)",
    ] {
        let mut at = sized_render(
            "rotate",
            (16, 16),
            &format!(
                ".box {{ left: 6px; top: 7px; width: 4px; height: 2px; background: #f00;
                        transform: {transform} }}
                 .kid {{ left: 0; top: 0; width: 2px; height: 2px; background: #0f0 }}"
            ),
            r#"<div class="clip box" data-start="0" data-duration="1">
                 <div class="clip kid" data-start="0" data-duration="1"></div></div>"#,
        );
        // The child (top-left square of the box) rotates with it to the top-right of the result.
        assert_eq!(at(8, 6), GREEN, "{transform}");
        assert_eq!(at(7, 6), GREEN, "{transform}");
        assert_eq!(at(7, 9), RED, "{transform}");
        assert_eq!(at(8, 9), RED, "{transform}");
        assert_eq!(at(6, 8), BLACK, "{transform}");
        assert_eq!(at(9, 8), BLACK, "{transform}");
        assert_eq!(at(7, 5), BLACK, "{transform}");
    }
}

#[test]
fn rotate_animates_and_stays_deterministic() {
    let css = "@keyframes spin { from { transform: rotate(0deg) } to { transform: rotate(90deg) } }
        .box { left: 6px; top: 7px; width: 4px; height: 2px; background: #f00;
               animation: spin 1s linear both }";
    let path = css_composition(
        "spin",
        css,
        r#"<div class="clip box" data-start="0" data-duration="2"></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut r = Renderer::new(&composition, 1.0).unwrap();
    assert_eq!(red_at(&mut r, 0.0, 6, 7), 255, "starts unrotated");
    assert_eq!(red_at(&mut r, 0.0, 7, 6), 0);
    let mid = r.render_at(0.5).unwrap().to_vec();
    assert_ne!(mid, r.render_at(0.0).unwrap().to_vec());
    assert_eq!(red_at(&mut r, 1.5, 7, 6), 255, "forwards holds 90 degrees");
    assert_eq!(r.render_at(0.5).unwrap(), mid, "same time, same bytes");
}

#[test]
fn quality_presets_trade_file_size_for_fidelity() {
    use aether_motion::encode::{encode_native_with_options, EncodeOptions, Quality};
    let dir = std::env::temp_dir().join(format!("aether-hf-quality-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let composition = parse_file(fixture()).unwrap();
    let mut sizes = Vec::new();
    for quality in [
        Quality::Draft,
        Quality::Standard,
        Quality::High,
        Quality::Max,
    ] {
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        let output = dir.join(format!("{quality:?}.mp4"));
        let options = EncodeOptions {
            quality,
            ..EncodeOptions::new(12, 1)
        };
        encode_native_with_options(&mut renderer, &output, &options).unwrap();
        let file = File::open(&output).unwrap();
        let size = file.metadata().unwrap().len();
        let mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), size).unwrap();
        assert_eq!(mp4.sample_count(1).unwrap(), 48, "{quality:?}");
        sizes.push(size);
    }
    // A lower quantizer keeps more detail, so each preset is larger than the one before.
    assert!(sizes.windows(2).all(|pair| pair[0] < pair[1]), "{sizes:?}");
    assert_eq!(Quality::default(), Quality::Standard);
    assert_eq!("standard".parse::<Quality>().unwrap().base_qp(), 26);
    assert!("ultra".parse::<Quality>().is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn percent_translate_matches_the_px_translation_of_the_own_box() {
    // A 6x4 box: -50% -50% is (-3px, -2px); after rotate(90deg) it is turned with the box.
    let frame = |name: &str, transform: &str| {
        let css = format!(
            ".box {{ left: 8px; top: 8px; width: 6px; height: 4px; background: #f00; transform: {transform} }}"
        );
        let path = css_composition(
            name,
            &css,
            r#"<div class="clip box" data-start="0" data-duration="1"></div>"#,
        );
        let composition = parse_file(&path).unwrap();
        let mut r = Renderer::new(&composition, 1.0).unwrap();
        r.render_at(0.25).unwrap().to_vec()
    };
    for (percent, px) in [
        ("translate(-50%, -50%)", "translate(-3px, -2px)"),
        ("translateX(50%) translateY(-25%)", "translate(3px, -1px)"),
        ("scale(2) translate(-50%, 0)", "scale(2) translate(-3px, 0)"),
        (
            "rotate(90deg) translate(-50%, 50%)",
            "rotate(90deg) translate(-3px, 2px)",
        ),
    ] {
        assert_eq!(frame("pct", percent), frame("px", px), "{percent}");
    }
}

#[test]
fn percent_translate_is_rejected_where_it_cannot_be_resolved() {
    let cases = [(
        "main { transform: translate(-50%, 0) }",
        "not supported on the root",
    )];
    for (css, expected) in cases {
        let css =
            format!(".box {{ left: 0; top: 0; width: 4px; height: 4px; background: #f00 }} {css}");
        let path = css_composition(
            "pct-err",
            &css,
            r#"<div class="clip box" data-start="0" data-duration="1"></div>"#,
        );
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(error.contains(expected), "{css}: {error}");
    }
}

#[test]
fn css_animation_of_transform_replaces_a_static_percent_translation() {
    // A 6x4 box: the static translate(-50%, 25%) is (-3px, 1px). The animation replaces it while
    // it applies, and the implied `from` keyframe and the time outside the animation keep it.
    let frames = |name: &str, rest: &str, keys: &str, fill: &str| {
        let css = format!(
            "@keyframes k {{ {keys} }}
             .box {{ left: 4px; top: 4px; width: 6px; height: 4px; background: #f00;
                     transform: {rest}; animation: k 1s linear 0.25s {fill} }}"
        );
        let path = css_composition(
            name,
            &css,
            r#"<div class="clip box" data-start="0" data-duration="2"></div>"#,
        );
        let composition = parse_file(&path).unwrap();
        let mut r = Renderer::new(&composition, 1.0).unwrap();
        [0.0, 0.25, 0.75, 1.0, 1.5]
            .map(|t| r.render_at(t).unwrap().to_vec())
            .to_vec()
    };
    for fill in ["none", "both", "forwards", "backwards"] {
        assert_eq!(
            frames(
                "rest-pct",
                "translate(-50%, 25%)",
                "to { transform: translate(4px, 0) }",
                fill
            ),
            frames(
                "rest-px",
                "translate(-3px, 1px)",
                "to { transform: translate(4px, 0) }",
                fill
            ),
            "{fill}"
        );
        assert_eq!(
            frames(
                "both-pct",
                "translate(-50%, 25%)",
                "50% { transform: translateX(100%) } to { transform: scale(1.5) }",
                fill
            ),
            frames(
                "both-px",
                "translate(-3px, 1px)",
                "50% { transform: translateX(6px) } to { transform: scale(1.5) }",
                fill
            ),
            "{fill}"
        );
    }
}

#[test]
fn percent_translate_in_keyframes_matches_the_px_animation() {
    // A 6x4 box: 100% is 6px horizontally and 4px vertically.
    let frames = |name: &str, from: &str, to: &str| {
        let css = format!(
            "@keyframes slide {{ from {{ transform: {from} }} to {{ transform: {to} }} }}
             .box {{ left: 4px; top: 4px; width: 6px; height: 4px; background: #f00;
                     animation: slide 1s ease-in-out both }}"
        );
        let path = css_composition(
            name,
            &css,
            r#"<div class="clip box" data-start="0" data-duration="2"></div>"#,
        );
        let composition = parse_file(&path).unwrap();
        let mut r = Renderer::new(&composition, 1.0).unwrap();
        [0.0, 0.3, 0.5, 0.8, 1.5]
            .map(|t| r.render_at(t).unwrap().to_vec())
            .to_vec()
    };
    assert_eq!(
        frames("kf-pct", "translateX(-100%)", "translate(50%, 50%)"),
        frames("kf-px", "translateX(-6px)", "translate(3px, 2px)"),
    );
    assert_eq!(
        frames(
            "kf-pct-rot",
            "rotate(90deg) translateX(50%)",
            "rotate(90deg) translateX(0)"
        ),
        frames(
            "kf-px-rot",
            "rotate(90deg) translateX(3px)",
            "rotate(90deg) translateX(0)"
        ),
    );
}

/// Renders a 16x16 composition with a red 6x4 `.box` at (4, 4) styled by `extra` at each time.
fn origin_frames(name: &str, extra: &str, times: &[f64]) -> Vec<Vec<u8>> {
    let css = format!(
        ".box {{ left: 4px; top: 4px; width: 6px; height: 4px; background: #f00 }} {extra}"
    );
    let path = css_composition(
        name,
        &css,
        r#"<div class="clip box" data-start="0" data-duration="2"></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut r = Renderer::new(&composition, 1.0).unwrap();
    times
        .iter()
        .map(|&t| r.render_at(t).unwrap().to_vec())
        .collect()
}

/// Inclusive `(left, top, right, bottom)` of pixels whose red channel exceeds 127.
fn red_bbox(frame: &[u8]) -> Option<(usize, usize, usize, usize)> {
    let mut found: Option<(usize, usize, usize, usize)> = None;
    for y in 0..16 {
        for x in 0..16 {
            if frame[(y * 16 + x) * 4] > 127 {
                found = Some(found.map_or((x, y, x, y), |(l, t, r, b)| {
                    (l.min(x), t.min(y), r.max(x), b.max(y))
                }));
            }
        }
    }
    found
}

#[test]
fn transform_origin_matches_the_equivalent_center_translation() {
    // Around the center c, pivoting at c + d adds the translation d - s·R·d. For the 6x4 box:
    // `0 0` is d = (-3, -2), `right bottom` is d = (3, 2).
    for (origin, center) in [
        (
            "transform-origin: 0 0; transform: scale(2)",
            "transform: translate(3px, 2px) scale(2)",
        ),
        (
            "transform-origin: right bottom; transform: scale(0.5)",
            "transform: translate(1.5px, 1px) scale(0.5)",
        ),
        (
            "transform-origin: 100% 0; transform: translate(1px, 1px) scale(1.5)",
            "transform: translate(-0.5px, 2px) scale(1.5)",
        ),
        (
            "transform-origin: 3px 2px; transform: scale(3)",
            "transform: scale(3)",
        ),
    ] {
        assert_eq!(
            origin_frames("origin", &format!(".box {{ {origin} }}"), &[0.5]),
            origin_frames("center", &format!(".box {{ {center} }}"), &[0.5]),
            "{origin}"
        );
    }
}

#[test]
fn transform_origin_pivots_rotation() {
    // rotate(90deg) around the top-left corner: d = (-3, -2), R·d = (2, -3), so the box moves by
    // (-5, 1) relative to a turn around its center. sin and cos of 90 degrees are not exact in
    // floating point, so allow one level per channel.
    let origin = origin_frames(
        "rot-origin",
        ".box { transform-origin: left top; transform: rotate(90deg) }",
        &[0.5],
    );
    let center = origin_frames(
        "rot-center",
        ".box { transform: translate(-5px, 1px) rotate(90deg) }",
        &[0.5],
    );
    let worst = origin[0]
        .iter()
        .zip(&center[0])
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 1, "largest channel difference {worst}");
    // The turned 4x6 box hangs to the left of the pivot at (4, 4): x 0..=3, y 4..=9.
    assert_eq!(red_bbox(&origin[0]), Some((0, 4, 3, 9)));
}

#[test]
fn transform_origin_holds_the_pivot_while_scale_animates() {
    // Scale animates 0.5 -> 2 over 1 s around the top-left corner, which never moves.
    let frames = origin_frames(
        "origin-anim",
        "@keyframes grow { from { transform: scale(0.5) } to { transform: scale(2) } }
         .box { transform-origin: 0 0; animation: grow 1s linear both }",
        &[0.0, 2.0 / 3.0, 1.5],
    );
    assert_eq!(red_bbox(&frames[0]), Some((4, 4, 6, 5)));
    assert_eq!(red_bbox(&frames[1]), Some((4, 4, 12, 9)));
    assert_eq!(red_bbox(&frames[2]), Some((4, 4, 15, 11)));
    // The default center origin grows symmetrically instead.
    let centered = origin_frames(
        "center-anim",
        "@keyframes grow { from { transform: scale(0.5) } to { transform: scale(2) } }
         .box { animation: grow 1s linear both }",
        &[1.5],
    );
    assert_eq!(red_bbox(&centered[0]), Some((1, 2, 12, 9)));
}

#[test]
fn transform_origin_is_rejected_where_it_has_no_pivot() {
    let cases = [
        (
            "main { transform-origin: 0 0 }",
            r#"<div class="clip box" data-start="0" data-duration="1"></div>"#,
            "not supported on the root",
        ),
        (
            ".box { transform-origin: 0 0 }",
            r#"<div class="clip box" data-start="0" data-duration="1" data-hf-camera='[{"at":0,"zoom":1},{"at":1,"zoom":2}]'></div>"#,
            "a camera zooms and turns around its center",
        ),
        (
            ".box { transform-origin: 0 0 0 4px }",
            r#"<div class="clip box" data-start="0" data-duration="1"></div>"#,
            "one to three values",
        ),
        (
            ".box { transform-origin: 0 0 10px }",
            r#"<div class="clip box" data-start="0" data-duration="1"></div>"#,
            "3D origin",
        ),
        (
            ".box { transform-origin: top 4px }",
            r#"<div class="clip box" data-start="0" data-duration="1"></div>"#,
            "horizontal value first",
        ),
        (
            "span { transform-origin: 0 0 }",
            r#"<div class="clip box" data-start="0" data-duration="1">Hi <span>there</span></div>"#,
            "transform",
        ),
    ];
    for (css, body, expected) in cases {
        let css =
            format!(".box {{ left: 0; top: 0; width: 4px; height: 4px; background: #f00 }} {css}");
        let path = css_composition("origin-err", &css, body);
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(error.contains(expected), "{css}: {error}");
    }
}

/// Red-weighted x centroid of each row `rows` of a 16x16 frame.
fn row_centroids(frame: &[u8], rows: std::ops::Range<usize>) -> Vec<f32> {
    rows.map(|y| {
        let (mut mass, mut moment) = (0.0_f32, 0.0_f32);
        for x in 0..16 {
            let red = f32::from(frame[(y * 16 + x) * 4]);
            mass += red;
            moment += red * (x as f32 + 0.5);
        }
        moment / mass
    })
    .collect()
}

/// Total red coverage of a 16x16 frame, in pixels.
fn red_area(frame: &[u8]) -> f32 {
    frame.chunks(4).map(|p| f32::from(p[0]) / 255.0).sum()
}

fn skew_frame(name: &str, css: &str) -> Vec<u8> {
    let css =
        format!(".box {{ left: 6px; top: 6px; width: 4px; height: 4px; background: #f00 }} {css}");
    let path = css_composition(
        name,
        &css,
        r#"<div class="clip box" data-start="0" data-duration="1"></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut r = Renderer::new(&composition, 1.0).unwrap();
    r.render_at(0.5).unwrap().to_vec()
}

#[test]
fn skew_shears_the_box_around_its_center_and_keeps_its_area() {
    // skewX(45deg) moves each row by its height from the center (8, 8): rows 6..=9 have their
    // centers at y - 8 = -1.5, -0.5, 0.5, 1.5.
    let frame = skew_frame("skewx", ".box { transform: skewX(45deg) }");
    for (got, want) in row_centroids(&frame, 6..10)
        .iter()
        .zip([6.5, 7.5, 8.5, 9.5])
    {
        assert!((got - want).abs() < 0.1, "row centroid {got} vs {want}");
    }
    assert!(
        (red_area(&frame) - 16.0).abs() < 0.3,
        "area {}",
        red_area(&frame)
    );
    // skewY shears columns instead: the frame is skewX's transposed (to one level, since skewY
    // is drawn as a rotation times a shear).
    let transposed = skew_frame("skewy", ".box { transform: skewY(45deg) }");
    for y in 0..16 {
        for x in 0..16 {
            let (a, b) = (frame[(y * 16 + x) * 4], transposed[(x * 16 + y) * 4]);
            assert!(a.abs_diff(b) <= 1, "({x}, {y}): {a} vs {b}");
        }
    }
    // A non-uniform scale doubles the area and keeps the center. The box is drawn at the uniform
    // scale sqrt(2) and resampled by (sqrt(2), 1/sqrt(2)), so its soft edges lose a little
    // coverage (bilinear minification), as a rotated layer's do.
    let wide = skew_frame("scale-xy", ".box { transform: scale(2, 1) }");
    assert!(
        (red_area(&wide) - 32.0).abs() < 1.6,
        "area {}",
        red_area(&wide)
    );
    assert_eq!(red_bbox(&wide), Some((4, 6, 11, 9)));
    assert_eq!(wide, skew_frame("scale-x", ".box { transform: scaleX(2) }"));
}

#[test]
fn skew_follows_transform_origin_and_effect_layers() {
    // Around the top-left corner, d = (-2, -2) and A·d = (-4, -2): the box moves 2 px right.
    let frame = skew_frame(
        "skew-origin",
        ".box { transform-origin: 0 0; transform: skewX(45deg) }",
    );
    for (got, want) in row_centroids(&frame, 6..10)
        .iter()
        .zip([8.5, 9.5, 10.5, 11.5])
    {
        assert!((got - want).abs() < 0.1, "row centroid {got} vs {want}");
    }
    // The filter layer holds the whole skewed box, so nothing is cut.
    let plain = skew_frame("skew-plain", ".box { transform: skewX(45deg) }");
    let filtered = skew_frame(
        "skew-filter",
        ".box { transform: skewX(45deg); filter: saturate(1.5) }",
    );
    assert!(
        (red_area(&plain) - red_area(&filtered)).abs() < 0.2,
        "{} vs {}",
        red_area(&plain),
        red_area(&filtered)
    );
}

#[test]
fn transforms_without_skew_render_exactly_as_before() {
    for (shaped, plain) in [
        ("rotate(30deg) skewX(0deg)", "rotate(30deg)"),
        ("skew(0, 0) rotate(90deg) scaleX(1)", "rotate(90deg)"),
        (
            "scale(1.5, 1.5) rotate(-15deg)",
            "scale(1.5) rotate(-15deg)",
        ),
    ] {
        assert_eq!(
            skew_frame("shaped", &format!(".box {{ transform: {shaped} }}")),
            skew_frame("plain", &format!(".box {{ transform: {plain} }}")),
            "{shaped}"
        );
    }
}

#[test]
fn skew_is_rejected_where_it_cannot_be_drawn() {
    let clip = r#"<div class="clip box" data-start="0" data-duration="1"></div>"#;
    let cases = [
        ("main { transform: skewX(10deg) }", clip.to_string(), "cannot be on the root"),
        (
            ".box { transform: skewX(10deg) }",
            r#"<div class="clip box" data-start="0" data-duration="1" data-hf-camera='[{"at":0,"zoom":1},{"at":1,"zoom":2}]'></div>"#.to_string(),
            "on camera",
        ),
        (
            "@keyframes k { to { transform: translate(1px, 0) } } .box { transform: skewX(10deg); animation: k 1s both }",
            clip.to_string(),
            "replaces the static skew()",
        ),
        (
            "@keyframes k { to { transform: skewX(10deg) } } .box { animation: k 1s both }",
            clip.to_string(),
            "cannot be animated",
        ),
        (
            ".box { transform: skewX(10deg) }",
            r#"<div class="clip box" data-start="0" data-duration="1" data-hf-reveal="from-left"></div>"#.to_string(),
            "combined with rotate or skew",
        ),
        (".box { transform: skewX(90deg) }", clip.to_string(), "flattens"),
        (
            "span { transform: skewX(10deg) }",
            r#"<div class="clip box" data-start="0" data-duration="1">Hi <span>there</span></div>"#.to_string(),
            "transform",
        ),
        (
            ".plane { width: 4px; height: 4px; transform: skewX(10deg) }",
            r#"<div class="clip box" data-start="0" data-duration="1" data-hf-space><div class="plane" data-hf-z="10"></div></div>"#.to_string(),
            "data-hf-space stage or plane",
        ),
    ];
    for (css, body, expected) in cases {
        let css =
            format!(".box {{ left: 0; top: 0; width: 4px; height: 4px; background: #f00 }} {css}");
        let path = css_composition("skew-err", &css, &body);
        let error = format!("{:#}", parse_file(&path).unwrap_err());
        assert!(error.contains(expected), "{css}: {error}");
    }
}
