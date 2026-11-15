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
    assert!(report["fix"].as_str().unwrap().contains("elimina"));
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
    assert!(report["error"]
        .as_str()
        .unwrap()
        .contains("fotograma clave 3"));
    assert!(report["fix"].as_str().unwrap().contains("ascendente"));
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
