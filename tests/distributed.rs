//! Distributed rendering: part files for segment ranges assemble into the exact bytes of a
//! single-process render, with audio, and incomplete or mismatched part sets are refused.
mod common;

use aether_motion::encode::{
    assemble_parts, encode_native, render_part, segment_count, EncodeOptions, Quality,
};
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::{scratch, write_wav};
use std::path::{Path, PathBuf};

const FPS: u32 = 10;

/// A 3.5 s animated composition with a sound, so frames change and the MP4 has two tracks.
fn composition(dir: &Path) -> PathBuf {
    let tone: Vec<i16> = (0..48_000 * 2)
        .map(|i| ((i as f64 * 0.05).sin() * 8_000.0) as i16)
        .collect();
    write_wav(&dir.join("tone.wav"), 48_000, 1, &tone);
    let html = dir.join("index.html");
    std::fs::write(
        &html,
        "<div data-composition-id='x' data-width='64' data-height='36' data-duration='3.5' style='background:#123'>\
         <div id='box' data-start='0' data-duration='3.5' data-track-index='1' style='left:0;top:8px;width:16px;height:16px;background:#f80'\
          data-hf-keyframes='[{\"at\":0,\"x\":0},{\"at\":3.4,\"x\":44}]'></div>\
         <audio src='tone.wav' data-start='0.5'></audio></div>",
    )
    .unwrap();
    html
}

fn part(html: &Path, output: &Path, segments: std::ops::Range<usize>, quality: Quality) {
    let composition = parse_file(html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let options = EncodeOptions {
        quality,
        ..EncodeOptions::new(FPS, 2)
    };
    render_part(&mut renderer, output, &options, segments).unwrap();
}

#[test]
fn assembled_parts_match_a_single_render_byte_for_byte() {
    let dir = scratch("distributed");
    let html = composition(&dir);
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    assert_eq!(segment_count(renderer.frame_count(FPS), FPS), Some(4));
    let single = dir.join("single.mp4");
    encode_native(&mut renderer, &single, FPS, 3).unwrap();

    // Uneven ranges, assembled in shuffled order.
    let parts = [dir.join("c.aseg"), dir.join("a.aseg"), dir.join("b.aseg")];
    part(&html, &parts[1], 0..1, Quality::Standard);
    part(&html, &parts[2], 1..3, Quality::Standard);
    part(&html, &parts[0], 3..4, Quality::Standard);
    let assembled = dir.join("assembled.mp4");
    assemble_parts(&composition, &parts, &assembled, 192_000).unwrap();
    assert_eq!(
        std::fs::read(&single).unwrap(),
        std::fs::read(&assembled).unwrap()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn incomplete_or_mismatched_parts_are_refused() {
    let dir = scratch("distributed-bad");
    let html = composition(&dir);
    let composition = parse_file(&html).unwrap();
    let (a, b, c) = (dir.join("a.aseg"), dir.join("b.aseg"), dir.join("c.aseg"));
    part(&html, &a, 0..2, Quality::Standard);
    part(&html, &b, 2..4, Quality::High);
    part(&html, &c, 1..4, Quality::Standard);
    let out = dir.join("out.mp4");
    let error = |parts: &[PathBuf]| {
        assemble_parts(&composition, parts, &out, 192_000)
            .unwrap_err()
            .to_string()
    };
    assert!(error(std::slice::from_ref(&a)).contains("cover"));
    assert!(error(&[a.clone(), b]).contains("different render"));
    assert!(error(&[a.clone(), c]).contains("exactly once"));
    assert!(!out.exists());

    // Ranges outside the render, and renders too short to split.
    let composition_ok = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition_ok, 1.0).unwrap();
    let options = EncodeOptions::new(FPS, 1);
    assert!(render_part(&mut renderer, &dir.join("x.aseg"), &options, 3..5).is_err());
    let short = dir.join("short.html");
    std::fs::write(
        &short,
        "<div data-composition-id='s' data-width='16' data-height='16' data-duration='1'></div>",
    )
    .unwrap();
    let short = parse_file(&short).unwrap();
    let mut renderer = Renderer::new(&short, 1.0).unwrap();
    let message = render_part(&mut renderer, &dir.join("y.aseg"), &options, 0..1)
        .unwrap_err()
        .to_string();
    assert!(message.contains("one piece"), "{message}");
    std::fs::remove_dir_all(dir).unwrap();
}
