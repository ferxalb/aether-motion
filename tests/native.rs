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
