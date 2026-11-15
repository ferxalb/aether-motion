use aether_hyperframes::encode::encode_native;
use aether_hyperframes::parse::parse_file;
use aether_hyperframes::render::Renderer;
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
    let ok = aether_hyperframes::agent::check(fixture());
    assert_eq!(ok["ok"], true);
    let dir = std::env::temp_dir().join(format!("aether-hf-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.html");
    std::fs::write(&bad, r#"<div data-composition-id="x" data-width="100" data-height="100" data-duration="1"><script>1</script></div>"#).unwrap();
    let report = aether_hyperframes::agent::check(&bad);
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
    let report = aether_hyperframes::agent::check(&path);
    assert_eq!(
        report["animations"][0]["keyframeTimes"],
        serde_json::json!([0.0, 1.0, 2.0])
    );

    let invalid = html.replace(r#"{"at":2}"#, r#"{"at":1}"#);
    std::fs::write(&path, invalid).unwrap();
    let report = aether_hyperframes::agent::check(&path);
    assert_eq!(report["ok"], false);
    assert!(report["error"].as_str().unwrap().contains("keyframe 3"));
    assert!(report["fix"].as_str().unwrap().contains("increasing"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn exports_full_hd_and_uhd_with_exact_cadence() {
    use aether_hyperframes::render::OutputSize;

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
