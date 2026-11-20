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
    // This crate's encoder writes limited-range BT.709 and tags it in the SPS.
    let dir = source();
    let report = agent::check(&page(&dir, r#"data-duration="2""#, 2.0));
    let colour = &report["videos"][0]["colour"];
    assert_eq!(colour["matrix"], "bt709", "{report}");
    assert_eq!(colour["range"], "limited");
    assert_eq!(colour["tagged"], true, "{report}");
}

#[test]
fn native_exports_round_trip_reference_colors_through_bt709() {
    // Saturated primaries and secondaries separate the matrices: decoding BT.709 codes with the
    // BT.601 matrix (or the reverse) moves red, green, and blue by 20 or more levels. Each color is
    // a flat 64x64 frame, so H.264 keeps it within a few levels.
    const REFERENCE: [[u8; 3]; 10] = [
        [0, 0, 0],
        [255, 255, 255],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [255, 255, 0],
        [0, 255, 255],
        [255, 0, 255],
        [200, 120, 40],
    ];
    let dir = scratch("video-bt709");
    let clips: String = REFERENCE
        .iter()
        .enumerate()
        .map(|(i, [r, g, b])| {
            format!(
                r#"<div class="clip" data-start="{i}" data-duration="1" data-track-index="0" style="left:0px;top:0px;width:64px;height:64px;background:rgb({r},{g},{b})"></div>"#
            )
        })
        .collect();
    let html = dir.join("source.html");
    std::fs::write(
        &html,
        format!(r#"<div data-composition-id="s" data-width="64" data-height="64" data-duration="{}">{clips}</div>"#, REFERENCE.len()),
    )
    .unwrap();
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    encode_native(&mut renderer, &dir.join("src.mp4"), 2, 1).unwrap();
    let html = dir.join("index.html");
    std::fs::write(
        &html,
        format!(
            r#"<div data-composition-id="v" data-width="64" data-height="64" data-duration="{}" style="background:#000000">
<video src="src.mp4" muted class="clip" data-start="0" data-track-index="0" style="left:0px;top:0px;width:64px;height:64px"></video></div>"#,
            REFERENCE.len()
        ),
    )
    .unwrap();
    let report = agent::check(&html);
    assert_eq!(report["videos"][0]["colour"]["matrix"], "bt709", "{report}");
    assert_eq!(
        report["videos"][0]["colour"]["range"], "limited",
        "{report}"
    );
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    for (i, wanted) in REFERENCE.iter().enumerate() {
        let frame = renderer.render_at(i as f64 + 0.75).unwrap();
        let at = (32 * 64 + 32) * 4;
        let got = [frame[at], frame[at + 1], frame[at + 2]];
        let error = got
            .iter()
            .zip(wanted)
            .map(|(&a, &w)| a.abs_diff(w))
            .max()
            .unwrap();
        assert!(error <= 4, "rgb {wanted:?} decoded as {got:?}");
    }
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

#[test]
fn constant_speed_slows_the_picture_and_stretches_the_default_length() {
    let dir = source();
    let composition = parse_file(&page(&dir, r#"data-hf-speed="0.5""#, 4.0)).unwrap();
    let video = &composition.elements[1];
    assert!(
        (video.end - video.start - 4.0).abs() < 1e-9,
        "{}",
        video.end
    );
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    // Media time is half the clip time: each color lasts 1 s instead of 0.5 s.
    for (time, wanted) in [(0.1, 0), (0.9, 0), (1.1, 1), (2.1, 2), (3.9, 3)] {
        let actual = center(&mut renderer, time);
        assert!(near(actual, COLORS[wanted].1), "t={time}: {actual:?}");
    }
}

#[test]
fn a_speed_ramp_integrates_the_rate_into_source_time() {
    let dir = source();
    // 2x for 0.5 s (media 1.0), linear 2x -> 0.5x until 1 s (media 1.625), then 0.5x.
    let ramp =
        r#"data-hf-speed-ramp='[{"at":0,"speed":2},{"at":0.5,"speed":2},{"at":1,"speed":0.5}]'"#;
    let html = page(&dir, ramp, 1.75);
    let composition = parse_file(&html).unwrap();
    let video = &composition.elements[1];
    assert!(
        (video.end - 1.75).abs() < 1e-6,
        "default length {}",
        video.end
    );
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    for (time, wanted) in [(0.2, 0), (0.4, 1), (0.9, 3), (1.6, 3)] {
        let actual = center(&mut renderer, time);
        assert!(near(actual, COLORS[wanted].1), "t={time}: {actual:?}");
    }
    let report = agent::check(&html);
    assert_eq!(report["ok"], true, "{report}");
    let reported = &report["videos"][0];
    assert_eq!(reported["speed"]["ramp"][2]["speed"], 0.5, "{report}");
    assert!(
        (reported["mediaEnd"].as_f64().unwrap() - 2.0).abs() < 1e-6,
        "{report}"
    );
}

#[test]
fn rejects_invalid_speed_settings() {
    let dir = source();
    for (attributes, needle) in [
        (r#"data-hf-speed="0""#, "data-hf-speed must be between"),
        (r#"data-hf-speed="20""#, "data-hf-speed must be between"),
        (
            r#"data-hf-speed="2" data-hf-speed-ramp='[{"at":0,"speed":1}]'"#,
            "not both",
        ),
        (
            r#"data-hf-speed-ramp='[{"at":1,"speed":1},{"at":0.5,"speed":2}]'"#,
            "strictly increasing",
        ),
        (
            r#"data-hf-speed-ramp='[{"at":0,"rate":1}]'"#,
            "unknown field",
        ),
        (
            r#"data-hf-speed-ramp='[{"at":0,"speed":1},{"at":0.5,"speed":0}]'"#,
            "set data-duration",
        ),
    ] {
        let report = agent::check(&page(&dir, attributes, 2.0));
        assert_eq!(report["ok"], false, "{attributes}");
        let error = report["error"].as_str().unwrap();
        assert!(error.contains(needle), "{attributes}: {error}");
    }
    // With a duration, a ramp to 0 freezes on the last frame reached.
    let frozen =
        r#"data-duration="2" data-hf-speed-ramp='[{"at":0,"speed":1},{"at":0.5,"speed":0}]'"#;
    let report = agent::check(&page(&dir, frozen, 2.0));
    assert_eq!(report["ok"], true, "{report}");
    assert!((report["videos"][0]["mediaEnd"].as_f64().unwrap() - 0.25).abs() < 1e-9);
}

#[test]
fn the_video_sound_follows_the_picture_speed() {
    let dir = scratch("video-audio-speed");
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

    let report = agent::check(&page_without_muted(&dir, r#"data-hf-speed="0.5""#));
    assert_eq!(report["ok"], true, "{report}");
    let clip = &report["audio"]["clips"][0];
    assert_eq!(clip["speed"]["speed"], 0.5, "{report}");
    // The 2 s clip plays 1 s of the source's sound, like its picture.
    assert!(
        (clip["end"].as_f64().unwrap() - 2.0).abs() < 0.05,
        "{report}"
    );
    assert!(
        (clip["mediaSeconds"].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "{report}"
    );
    assert!(
        (report["videos"][0]["mediaEnd"].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "{report}"
    );
}
