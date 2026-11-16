//! `<video>`: H.264 MP4 sources decoded natively. The fixture is made by this crate's own encoder
//! (four solid colors, half a second each, at 10 fps), so no external tool is needed.
mod common;

use aether_motion::agent;
use aether_motion::encode::encode_native;
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;
use std::path::{Path, PathBuf};

const COLORS: [(&str, [u8; 3]); 4] = [
    ("#ff0000", [255, 0, 0]),
    ("#00ff00", [0, 255, 0]),
    ("#0000ff", [0, 0, 255]),
    ("#ffffff", [255, 255, 255]),
];

/// Writes `src.mp4` (20 frames at 10 fps, 32x32) into a fresh directory and returns the directory.
fn source() -> PathBuf {
    let dir = scratch("video");
    let clips: String = COLORS
        .iter()
        .enumerate()
        .map(|(i, (css, _))| {
            format!(
                r#"<div class="clip" data-start="{}" data-duration="0.5" data-track-index="0" style="left:0px;top:0px;width:32px;height:32px;background:{css}"></div>"#,
                i as f64 * 0.5
            )
        })
        .collect();
    let html = dir.join("source.html");
    std::fs::write(
        &html,
        format!(r#"<div data-composition-id="s" data-width="32" data-height="32" data-duration="2">{clips}</div>"#),
    )
    .unwrap();
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    encode_native(&mut renderer, &dir.join("src.mp4"), 10, 1).unwrap();
    dir
}

fn page(dir: &Path, attributes: &str, duration: f64) -> PathBuf {
    let html = dir.join("index.html");
    std::fs::write(
        &html,
        format!(
            r#"<div data-composition-id="v" data-width="32" data-height="32" data-duration="{duration}" style="background:#000000">
<video src="src.mp4" muted class="clip" data-start="0" data-track-index="0" {attributes} style="left:0px;top:0px;width:32px;height:32px"></video></div>"#
        ),
    )
    .unwrap();
    html
}

/// The color at the center of the frame shown at `time`.
fn center(renderer: &mut Renderer<'_>, time: f64) -> [u8; 3] {
    let frame = renderer.render_at(time).unwrap();
    let i = (16 * 32 + 16) * 4;
    [frame[i], frame[i + 1], frame[i + 2]]
}

fn near(actual: [u8; 3], wanted: [u8; 3]) -> bool {
    actual
        .iter()
        .zip(wanted)
        .all(|(&a, w)| (i32::from(a) - i32::from(w)).abs() <= 40)
}

#[test]
fn frames_follow_source_time_in_any_order() {
    let dir = source();
    let composition = parse_file(&page(&dir, r#"data-duration="2""#, 2.0)).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    // Forward, then backward, then a jump: each must show the source frame at that time.
    for (time, wanted) in [
        (0.1, 0),
        (0.6, 1),
        (1.1, 2),
        (1.7, 3),
        (0.1, 0),
        (1.6, 3),
        (0.7, 1),
    ] {
        let color = center(&mut renderer, time);
        assert!(near(color, COLORS[wanted].1), "t={time}: {color:?}");
    }
}

#[test]
fn media_start_trims_and_the_last_frame_holds() {
    let dir = source();
    let composition = parse_file(&page(
        &dir,
        r#"data-media-start="1" data-duration="2""#,
        2.0,
    ))
    .unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    for (time, wanted) in [(0.1, 2), (0.6, 3), (1.5, 3)] {
        let color = center(&mut renderer, time);
        assert!(near(color, COLORS[wanted].1), "t={time}: {color:?}");
    }
}

#[test]
fn looping_repeats_the_source() {
    let dir = source();
    let composition = parse_file(&page(&dir, r#"loop data-duration="4""#, 4.0)).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    for (time, wanted) in [(2.1, 0), (2.6, 1), (3.7, 3)] {
        let color = center(&mut renderer, time);
        assert!(near(color, COLORS[wanted].1), "t={time}: {color:?}");
    }
}

#[test]
fn clip_length_defaults_to_the_source() {
    let dir = source();
    let composition = parse_file(&page(&dir, r#"data-media-start="0.5""#, 3.0)).unwrap();
    let video = &composition.elements[1];
    assert!(
        (video.end - video.start - 1.5).abs() < 1e-6,
        "{} .. {}",
        video.start,
        video.end
    );
}

#[test]
fn check_reports_the_video_and_rejects_bad_sources() {
    let dir = source();
    let report = agent::check(&page(&dir, r#"data-duration="2""#, 2.0));
    assert_eq!(report["ok"], true, "{report}");
    let video = &report["videos"][0];
    assert_eq!(video["frames"], 20);
    assert_eq!(video["width"], 32);
    assert_eq!(video["hasAudioTrack"], false);

    let no_length = agent::check(&page(&dir, "loop", 2.0));
    assert_eq!(no_length["ok"], false);
    assert!(
        no_length["error"]
            .as_str()
            .unwrap()
            .contains("data-duration"),
        "{no_length}"
    );

    std::fs::write(dir.join("src.mp4"), b"not a video").unwrap();
    let junk = agent::check(&page(&dir, r#"data-duration="2""#, 2.0));
    assert_eq!(junk["ok"], false);
    assert!(junk["error"].as_str().unwrap().contains("video"), "{junk}");
}

/// A 16-bit mono 48 kHz WAV holding a 440 Hz tone, `seconds` long.
fn tone(path: &Path, seconds: f64) {
    let samples = (48_000.0 * seconds) as usize;
    let mut bytes = Vec::new();
    let data = (samples * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&96_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for n in 0..samples {
        let value =
            (f64::sin(n as f64 * 440.0 * std::f64::consts::TAU / 48_000.0) * 12_000.0) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn the_audio_track_is_mixed_unless_muted() {
    let dir = scratch("video-audio");
    tone(&dir.join("tone.wav"), 2.0);
    let html = dir.join("source.html");
    std::fs::write(
        &html,
        r#"<div data-composition-id="s" data-width="32" data-height="32" data-duration="2" style="background:#204060">
<audio src="tone.wav" data-start="0"></audio></div>"#,
    )
    .unwrap();
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    encode_native(&mut renderer, &dir.join("src.mp4"), 10, 1).unwrap();

    let with_sound = agent::check(&page_without_muted(&dir, ""));
    assert_eq!(with_sound["ok"], true, "{with_sound}");
    assert_eq!(with_sound["videos"][0]["audioMixed"], true);
    let clips = with_sound["audio"]["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1, "{with_sound}");
    assert!(
        (clips[0]["end"].as_f64().unwrap() - 2.0).abs() < 0.05,
        "{with_sound}"
    );
    assert!(
        with_sound["audio"]["peak"].as_f64().unwrap() > 0.1,
        "{with_sound}"
    );

    let muted = agent::check(&page_without_muted(&dir, "muted"));
    assert_eq!(muted["videos"][0]["audioMixed"], false);
    assert!(muted["audio"]["clips"].as_array().unwrap().is_empty());
}

fn page_without_muted(dir: &Path, extra: &str) -> PathBuf {
    let html = dir.join("index.html");
    std::fs::write(
        &html,
        format!(
            r#"<div data-composition-id="v" data-width="32" data-height="32" data-duration="2" style="background:#000000">
<video src="src.mp4" {extra} class="clip" data-start="0" data-duration="2" data-track-index="0" style="left:0px;top:0px;width:32px;height:32px"></video></div>"#
        ),
    )
    .unwrap();
    html
}

/// Overwrites the `tkhd` display matrix of an MP4 written by this crate (version 0 header).
fn set_matrix(path: &Path, matrix: [i32; 9]) {
    let mut bytes = std::fs::read(path).unwrap();
    let at = bytes.windows(4).position(|w| w == b"tkhd").unwrap() + 44;
    for (i, value) in matrix.iter().enumerate() {
        bytes[at + i * 4..at + i * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn rotation_metadata_turns_the_picture_upright() {
    // Source: left half red, right half blue.
    let dir = scratch("video-rotation");
    let html = dir.join("source.html");
    std::fs::write(
        &html,
        r#"<div data-composition-id="s" data-width="32" data-height="32" data-duration="1" style="background:#000000">
<div class="clip" data-start="0" data-duration="1" data-track-index="0" style="left:0px;top:0px;width:16px;height:32px;background:#ff0000"></div>
<div class="clip" data-start="0" data-duration="1" data-track-index="0" style="left:16px;top:0px;width:16px;height:32px;background:#0000ff"></div></div>"#,
    )
    .unwrap();
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    encode_native(&mut renderer, &dir.join("src.mp4"), 10, 1).unwrap();
    let one = 0x1_0000;
    // 90 degrees clockwise: the left half ends up on top.
    set_matrix(
        &dir.join("src.mp4"),
        [0, one, 0, -one, 0, 0, 0, 0, 0x4000_0000],
    );
    let html = dir.join("index.html");
    std::fs::write(
        &html,
        r#"<div data-composition-id="v" data-width="32" data-height="32" data-duration="1" style="background:#000000">
<video src="src.mp4" muted class="clip" data-start="0" data-duration="1" data-track-index="0" style="left:0px;top:0px;width:32px;height:32px"></video></div>"#,
    )
    .unwrap();
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let frame = renderer.render_at(0.3).unwrap();
    let at = |x: usize, y: usize| {
        [
            frame[(y * 32 + x) * 4],
            frame[(y * 32 + x) * 4 + 1],
            frame[(y * 32 + x) * 4 + 2],
        ]
    };
    assert!(near(at(16, 5), [255, 0, 0]), "top: {:?}", at(16, 5));
    assert!(near(at(16, 27), [0, 0, 255]), "bottom: {:?}", at(16, 27));
}

#[test]
fn colour_tags_are_read_from_the_stream() {
    // This crate's encoder writes limited-range BT.601 and tags it in the SPS.
    let dir = source();
    let report = agent::check(&page(&dir, r#"data-duration="2""#, 2.0));
    let colour = &report["videos"][0]["colour"];
    assert_eq!(colour["matrix"], "bt601", "{report}");
    assert_eq!(colour["range"], "limited");
    assert_eq!(colour["tagged"], true, "{report}");
}

#[test]
fn source_children_and_poster_are_accepted() {
    let dir = source();
    let html = dir.join("index.html");
    std::fs::write(
        &html,
        r#"<div data-composition-id="v" data-width="32" data-height="32" data-duration="2" style="background:#000000">
<video muted poster="x.png" class="clip" data-start="0" data-track-index="0" style="left:0px;top:0px;width:32px;height:32px">
<source src="missing.webm" type="video/webm"><source src="src.mp4" type="video/mp4"></video></div>"#,
    )
    .unwrap();
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    assert!(near(center(&mut renderer, 0.6), COLORS[1].1));
}
