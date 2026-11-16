//! Audio end to end: `<audio>` parsing, sample-accurate placement through resampling and AAC, and
//! the MP4 structure (two tracks, interleaving, edit list, matching durations).
mod common;

use aether_motion::audio::{aac::PRIMING, mix::frames_for};
use aether_motion::encode::{encode_native, encode_native_with_audio};
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::scratch;
use oxideav_aac::decode::StreamDecoder;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// 16-bit PCM WAV, interleaved samples.
fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    bytes.extend_from_slice(&(channels * 2).to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

/// Mono 2 kHz burst under a Hann window, `length` samples, centered at source index `center`.
fn burst(rate: u32, seconds: f64, center: usize, length: usize) -> Vec<i16> {
    let mut samples = vec![0_i16; (f64::from(rate) * seconds) as usize];
    for i in 0..length {
        let hann =
            0.5 - 0.5 * (2.0 * std::f64::consts::PI * (i as f64 + 0.5) / length as f64).cos();
        let tone = (2.0 * std::f64::consts::PI * 2000.0 * i as f64 / f64::from(rate)).sin();
        samples[center - length / 2 + i] = (hann * tone * 20_000.0) as i16;
    }
    samples
}

fn composition(dir: &Path, body: &str, duration: u32) -> PathBuf {
    let html = dir.join("audio.html");
    std::fs::write(
        &html,
        format!(
            "<div data-composition-id='x' data-width='64' data-height='36' data-duration='{duration}' style='background:#123'>{body}</div>"
        ),
    )
    .unwrap();
    html
}

fn render(html: &Path, output: &Path, fps: u32, threads: u32) {
    let composition = parse_file(html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    encode_native(&mut renderer, output, fps, threads).unwrap();
}

/// Decoded left channel of the MP4's audio track with the priming samples dropped.
fn decode_audio(mp4_path: &Path) -> Vec<i16> {
    let file = File::open(mp4_path).unwrap();
    let size = file.metadata().unwrap().len();
    let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), size).unwrap();
    let count = mp4.sample_count(2).unwrap();
    let mut adts = Vec::new();
    for id in 1..=count {
        let unit = mp4.read_sample(2, id).unwrap().unwrap().bytes;
        let length = unit.len() + 7;
        adts.extend_from_slice(&[
            0xFF,
            0xF1,
            0x4C,
            0x80 | (length >> 11) as u8,
            (length >> 3) as u8,
            ((length & 7) << 5) as u8 | 0x1F,
            0xFC,
        ]);
        adts.extend_from_slice(&unit);
    }
    let frames = StreamDecoder::new().decode_all(&adts).unwrap();
    let pcm: Vec<i16> = frames.into_iter().flat_map(|f| f.pcm).collect();
    pcm.iter()
        .step_by(2)
        .skip(PRIMING as usize)
        .copied()
        .collect()
}

/// Energy-weighted center (in samples) of the decoded signal.
fn centroid(samples: &[i16]) -> f64 {
    let (mut weighted, mut total) = (0.0, 0.0);
    for (i, s) in samples.iter().enumerate() {
        let energy = f64::from(*s).powi(2);
        weighted += energy * i as f64;
        total += energy;
    }
    weighted / total
}

#[test]
fn parses_audio_attributes_and_inherited_start() {
    let dir = scratch("audio-parse");
    write_wav(&dir.join("a.wav"), 48_000, 1, &[0; 100]);
    let html = composition(
        &dir,
        "<div id='scene' data-start='1' data-duration='2'>\
           <audio id='nested' src='a.wav' data-volume='0.5' data-hf-fade-in='0.25' data-hf-trim-start='0.1'></audio>\
         </div>\
         <audio id='after' src='a.wav' data-start='scene+0.5' data-duration='0.5' data-hf-loop='true'></audio>",
        4,
    );
    let composition = parse_file(&html).unwrap();
    assert_eq!(composition.audio.len(), 2);
    let nested = &composition.audio[0];
    assert_eq!(nested.start, 1.0, "starts with the enclosing clip");
    assert_eq!(
        (nested.volume, nested.fade_in, nested.trim),
        (0.5, 0.25, 0.1)
    );
    assert!(!nested.looped && nested.duration.is_none());
    let after = &composition.audio[1];
    assert_eq!(after.start, 3.5);
    assert!(after.looped);
    // Audio is not a visual element: only the root and the scene exist.
    assert_eq!(composition.elements.len(), 2);
}

#[test]
fn rejects_invalid_audio_with_explicit_errors() {
    let dir = scratch("audio-errors");
    write_wav(&dir.join("a.wav"), 48_000, 1, &[0; 100]);
    let cases = [
        ("<audio data-start='0'></audio>", "requires src"),
        ("<audio src='https://x.test/a.wav'></audio>", "only local audio"),
        ("<audio src='a.wav' data-hf-loop='true'></audio>", "requires data-duration"),
        ("<audio src='a.wav' data-start='1.5' data-duration='1'></audio>", "exceeds composition"),
        ("<audio src='a.wav' data-start='2'></audio>", "starts after"),
        ("<audio src='a.wav' data-volume='20'></audio>", "data-volume"),
        (
            "<audio src='a.wav' data-duration='1' data-hf-fade-in='0.7' data-hf-fade-out='0.7'></audio>",
            "fade-in + fade-out",
        ),
        ("<audio src='a.wav'><source src='b.wav'></audio>", "child elements"),
    ];
    for (body, expected) in cases {
        let html = composition(&dir, body, 2);
        let error = format!("{:#}", parse_file(&html).unwrap_err());
        assert!(error.contains(expected), "{body}: {error}");
    }
}

#[test]
fn places_a_resampled_burst_at_the_right_sample_in_the_mp4() {
    let dir = scratch("audio-sync");
    // 44.1 kHz mono: a 10 ms burst centered 0.5 s into the file, played from t = 0.25 s.
    write_wav(
        &dir.join("burst.wav"),
        44_100,
        1,
        &burst(44_100, 1.0, 22_050, 441),
    );
    let html = composition(
        &dir,
        "<audio src='burst.wav' data-start='0.25' data-duration='0.9'></audio>",
        2,
    );
    let output = dir.join("out.mp4");
    render(&html, &output, 30, 1);

    let decoded = decode_audio(&output);
    let expected = frames_for(0.75) as f64;
    let found = centroid(&decoded[..frames_for(1.5)]);
    assert!(
        (found - expected).abs() < 8.0,
        "burst center at sample {found:.1}, expected {expected}"
    );
    // The clip ends at 1.15 s: nothing but silence (and codec ringing) afterwards.
    let tail = decoded[frames_for(1.3)..frames_for(1.9)]
        .iter()
        .map(|s| i32::from(*s).abs())
        .max()
        .unwrap();
    assert!(tail < 40, "tail peak {tail}");
}

#[test]
fn mp4_has_two_tracks_matching_durations_and_an_edit_list() {
    let dir = scratch("audio-mp4");
    write_wav(
        &dir.join("burst.wav"),
        48_000,
        1,
        &burst(48_000, 1.0, 24_000, 480),
    );
    let html = composition(&dir, "<audio src='burst.wav'></audio>", 2);
    let output = dir.join("out.mp4");
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    encode_native_with_audio(&mut renderer, &output, 25, 1, 128_000).unwrap();

    let bytes = std::fs::read(&output).unwrap();
    let file = File::open(&output).unwrap();
    let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(file), bytes.len() as u64).unwrap();
    assert_eq!(mp4.tracks().len(), 2);
    assert_eq!(mp4.sample_count(1).unwrap(), 50);
    // 2 s = 96000 samples = 93.75 units, rounded up, plus the flush frame.
    assert_eq!(mp4.sample_count(2).unwrap(), 94 + 1);
    assert_eq!(mp4.duration().as_secs_f64(), 2.0);
    let first = mp4.read_sample(2, 1).unwrap().unwrap();
    assert_eq!((first.start_time, first.duration), (0, 1024));

    let elst = bytes.windows(4).position(|w| w == b"elst").unwrap();
    let field = |at: usize| u32::from_be_bytes(bytes[elst + at..elst + at + 4].try_into().unwrap());
    assert_eq!(field(8), 1, "one edit");
    assert_eq!(field(12), 50, "edit spans the video, in movie ticks");
    assert_eq!(field(16), PRIMING, "edit skips the encoder delay");
    assert_eq!(bytes.windows(4).filter(|w| *w == b"elst").count(), 1);
}

#[test]
fn audio_output_is_identical_for_any_thread_count() {
    let dir = scratch("audio-determinism");
    write_wav(
        &dir.join("burst.wav"),
        44_100,
        1,
        &burst(44_100, 1.0, 22_050, 441),
    );
    let html = composition(
        &dir,
        "<audio src='burst.wav' data-hf-fade-out='0.2' data-duration='1.5' data-hf-loop='true'></audio>",
        3,
    );
    let (one, two) = (dir.join("one.mp4"), dir.join("two.mp4"));
    render(&html, &one, 10, 1);
    render(&html, &two, 10, 3);
    assert_eq!(std::fs::read(one).unwrap(), std::fs::read(two).unwrap());
}

#[test]
fn compositions_without_audio_keep_a_single_track() {
    let dir = scratch("audio-none");
    let html = composition(&dir, "", 1);
    let output = dir.join("out.mp4");
    render(&html, &output, 10, 1);
    let bytes = std::fs::read(&output).unwrap();
    let mp4 = mp4::Mp4Reader::read_header(
        BufReader::new(File::open(&output).unwrap()),
        bytes.len() as u64,
    )
    .unwrap();
    assert_eq!(mp4.tracks().len(), 1);
    assert!(!bytes.windows(4).any(|w| w == b"elst"));
}

#[test]
fn a_missing_audio_file_is_an_explicit_render_error() {
    let dir = scratch("audio-missing");
    let html = composition(&dir, "<audio src='nope.wav' data-duration='1'></audio>", 1);
    let composition = parse_file(&html).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let error = encode_native(&mut renderer, &dir.join("out.mp4"), 10, 1).unwrap_err();
    assert!(format!("{error:#}").contains("nope.wav"), "{error:#}");
    assert!(!dir.join("out.mp4").exists());
}

#[test]
fn half_speed_moves_a_burst_to_twice_its_time_in_the_mp4() {
    let dir = scratch("audio-speed");
    // A 10 ms burst 0.4 s into the file plays 0.8 s in at half speed.
    write_wav(
        &dir.join("burst.wav"),
        48_000,
        1,
        &burst(48_000, 1.0, 19_200, 480),
    );
    let html = composition(
        &dir,
        "<audio src='burst.wav' data-start='0' data-hf-speed='0.5'></audio>",
        2,
    );
    let report = aether_motion::agent::check(&html);
    assert_eq!(report["ok"], true, "{report}");
    let clip = &report["audio"]["clips"][0];
    assert!(
        (clip["end"].as_f64().unwrap() - 2.0).abs() < 1e-6,
        "{report}"
    );
    assert!(
        (clip["mediaSeconds"].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "{report}"
    );

    let output = dir.join("out.mp4");
    render(&html, &output, 30, 1);
    let decoded = decode_audio(&output);
    let expected = frames_for(0.8) as f64;
    let found = centroid(&decoded[..frames_for(1.5)]);
    assert!(
        (found - expected).abs() < 8.0,
        "burst center at sample {found:.1}, expected {expected}"
    );
    // The 2 kHz tone drops an octave: count zero crossings across the stretched burst.
    let window = &decoded[frames_for(0.79)..frames_for(0.81)];
    let crossings = window
        .windows(2)
        .filter(|w| (w[0] < 0) != (w[1] < 0))
        .count();
    // 20 ms at 1 kHz is 40 crossings (at 2 kHz it would be 80).
    assert!((30..=50).contains(&crossings), "{crossings} zero crossings");
}

#[test]
fn speed_ramped_audio_is_identical_for_any_thread_count() {
    let dir = scratch("audio-speed-determinism");
    write_wav(
        &dir.join("burst.wav"),
        48_000,
        1,
        &burst(48_000, 1.0, 24_000, 4800),
    );
    let html = composition(
        &dir,
        r#"<audio src='burst.wav' data-duration='2' data-hf-loop='true' data-hf-speed-ramp='[{"at":0,"speed":3},{"at":1,"speed":0.3,"easing":"ease-in-out-cubic"}]'></audio>"#,
        2,
    );
    let (one, four) = (dir.join("one.mp4"), dir.join("four.mp4"));
    render(&html, &one, 25, 1);
    render(&html, &four, 25, 4);
    assert_eq!(std::fs::read(&one).unwrap(), std::fs::read(&four).unwrap());
}
