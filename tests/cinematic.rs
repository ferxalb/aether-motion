//! Regression coverage for frame sequences, camera-relative pan and optional filmic finishing.

mod common;

use aether_motion::encode::encode_native;
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;
use image::{Rgba, RgbaImage};
use std::{fs, path::Path};

fn write_scene(dir: &Path, body: &str, root: &str) -> std::path::PathBuf {
    let path = dir.join("scene.html");
    fs::write(
        &path,
        format!("<div data-composition-id='scene' data-width='64' data-height='36' data-duration='4.2' {root}>{body}</div>"),
    ).unwrap();
    path
}

fn pixel(frame: &[u8], x: usize, y: usize) -> [u8; 4] {
    frame[(y * 64 + x) * 4..][..4].try_into().unwrap()
}

fn cels(dir: &Path, extension: &str) {
    for (index, rgb) in [[255, 30, 0], [20, 255, 10], [10, 50, 255]]
        .into_iter()
        .enumerate()
    {
        let path = dir.join(format!("cel_{:04}.{extension}", index + 1));
        let image = RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([rgb[0], rgb[1], rgb[2], 255])
            } else {
                Rgba([255, 0, 255, 0])
            }
        });
        image.save(path).unwrap();
    }
}

#[test]
fn cinematic_sequence_seek_modes_formats_and_alpha() {
    let dir = scratch("cinematic-sequence");
    for extension in ["png", "webp"] {
        cels(&dir, extension);
        for (mode, expected) in [
            ("hold", [0, 1, 2, 2, 2]),
            ("loop", [0, 1, 2, 0, 1]),
            ("pingpong", [0, 1, 2, 1, 0]),
        ] {
            let path = write_scene(&dir, &format!(
                "<img data-start='0.5' data-duration='2' data-hf-sequence='cel_{{frame:04}}.{extension}' data-hf-sequence-first='1' data-hf-sequence-playback='{mode}' style='width:8px;height:4px'>",
            ), "");
            let composition = parse_file(&path).unwrap();
            let mut renderer = Renderer::new(&composition, 1.0).unwrap();
            for fps in [24_usize, 30, 60] {
                for step in [4, 0, 2, 1, 3, 0] {
                    let frame = fps / 2 + (step * fps).div_ceil(12);
                    let result = renderer.render_frame(frame, fps as u32).unwrap();
                    let [r, g, b, _] = pixel(result, 3, 1);
                    let color = [r, g, b];
                    assert_eq!(
                        color.iter().position(|&value| value > 100),
                        Some(expected[step])
                    );
                    assert_eq!(color.iter().filter(|&&value| value > 100).count(), 1);
                    assert_eq!(pixel(result, 7, 1), [0, 0, 0, 255]);
                }
            }
        }
    }
    fs::remove_file(dir.join("cel_0002.png")).unwrap();
    let path = write_scene(
        &dir,
        "<img data-hf-sequence='cel_{frame:04}.png' data-hf-sequence-first='1'>",
        "",
    );
    assert!(format!("{:#}", parse_file(&path).unwrap_err()).contains("missing sequence frame 2"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cinematic_parallax_scales_pan_and_keeps_zoom_and_masks() {
    let dir = scratch("cinematic-parallax");
    let path = write_scene(
        &dir,
        r#"
        <div style="width:64px;height:36px" data-hf-camera='[{"at":0,"zoom":2,"x":3}]'>
            <div data-hf-parallax="0" style="left:20px;top:12px;width:2px;height:2px;background:red"></div>
            <div data-hf-parallax="1" style="left:20px;top:16px;width:2px;height:2px;background:lime"></div>
            <div data-hf-parallax="2" style="left:25px;top:20px;width:2px;height:2px;background:blue; clip-path:inset(0 50% 0 0)"></div>
        </div>"#,
        "",
    );
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let frame = renderer.render_frame(9, 24).unwrap();
    assert_eq!(pixel(frame, 8, 7), [255, 0, 0, 255]);
    assert_eq!(pixel(frame, 11, 7), [255, 0, 0, 255]);
    assert_eq!(pixel(frame, 2, 15), [0, 255, 0, 255]);
    assert_eq!(pixel(frame, 6, 23), [0, 0, 255, 255]);
    assert_eq!(pixel(frame, 8, 23), [0, 0, 0, 255]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cinematic_filmic_identity_and_threaded_encoding() {
    let dir = scratch("cinematic-threads");
    cels(&dir, "png");
    let body = r#"<div data-hf-camera='[{"at":0},{"at":2,"x":8}]'>
        <img data-hf-parallax="0.25" data-hf-sequence="cel_{frame:04}.png" data-hf-sequence-first="1"
             data-hf-sequence-playback="pingpong" style="left:14px;top:7px;width:22px;height:14px"/>
        <div data-hf-bloom="1" style="left:20px;top:12px;width:3px;height:3px;background:white"></div>
        </div>"#;
    let path = write_scene(&dir, body, "style='background:#747474'");
    let legacy = parse_file(&path).unwrap();
    let path = write_scene(
        &dir,
        body,
        "style='background:#747474' data-hf-exposure='0' data-hf-shadow-toe='0'",
    );
    let disabled = parse_file(&path).unwrap();
    let path = write_scene(&dir, body, "style='background:#747474' data-hf-exposure='-0.75' data-hf-highlight-rolloff='0.5' data-hf-shadow-toe='0.25'");
    let filmic = parse_file(&path).unwrap();
    let mut a = Renderer::new(&legacy, 1.0).unwrap();
    let mut b = Renderer::new(&disabled, 1.0).unwrap();
    let mut c = Renderer::new(&filmic, 1.0).unwrap();
    assert_eq!(
        a.render_frame(24, 24).unwrap(),
        b.render_frame(24, 24).unwrap()
    );
    let frame = c.render_frame(24, 24).unwrap().to_vec();
    assert!(pixel(&frame, 63, 35)[0] < 116);
    assert_eq!(c.fork().unwrap().render_frame(24, 24).unwrap(), frame);
    let reference = dir.join("single.mp4");
    encode_native(&mut c, &reference, 24, 1).unwrap();
    let reference = fs::read(reference).unwrap();
    for threads in [2, 4] {
        let candidate = dir.join(format!("threads-{threads}.mp4"));
        encode_native(&mut c, &candidate, 24, threads).unwrap();
        assert_eq!(fs::read(candidate).unwrap(), reference, "threads {threads}");
    }
    fs::remove_dir_all(dir).unwrap();
}
