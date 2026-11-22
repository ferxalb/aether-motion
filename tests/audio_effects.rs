//! Audio effects end to end: `data-hf-volume-ramp`, `data-hf-eq`, and `data-hf-duck` parsed from
//! HTML, measured on the exact mix, reported by `check`, and carried into the encoded MP4.
mod common;

use aether_motion::audio::aac::PRIMING;
use aether_motion::audio::mix::{mix, Mix};
use aether_motion::encode::encode_native;
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::{scratch, write_wav};
use oxideav_aac::decode::StreamDecoder;
use std::f64::consts::PI;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

const RATE: usize = 48_000;

fn composition(dir: &Path, body: &str, duration: u32) -> PathBuf {
    let html = dir.join("fx.html");
    std::fs::write(
        &html,
        format!(
            "<div data-composition-id='x' data-width='64' data-height='36' data-duration='{duration}' style='background:#123'>{body}</div>"
        ),
    )
    .unwrap();
    html
}

fn mixed(html: &Path, seconds: usize) -> Mix {
    mix(&parse_file(html).unwrap(), seconds * RATE).unwrap()
}

fn constant(value: i16, seconds: f64) -> Vec<i16> {
    vec![value; (seconds * RATE as f64) as usize]
}

/// A sine of `freq` Hz at `amplitude` (full scale 1) between `from` and `to` seconds.
fn tone(freq: f64, amplitude: f64, seconds: f64, from: f64, to: f64) -> Vec<i16> {
    (0..(seconds * RATE as f64) as usize)
        .map(|i| {
            let t = i as f64 / RATE as f64;
            if t < from || t >= to {
                0
            } else {
                (amplitude * 32767.0 * (2.0 * PI * freq * t).sin()) as i16
            }
        })
        .collect()
}

fn rms(samples: &[f32]) -> f64 {
    (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
}

fn db(ratio: f64) -> f64 {
    20.0 * ratio.log10()
}

#[test]
fn ducks_music_ahead_of_the_voice_and_releases_after_it() {
    let dir = scratch("fx-duck");
    write_wav(&dir.join("music.wav"), 48_000, 1, &constant(8192, 3.0));
    write_wav(
        &dir.join("voice.wav"),
        48_000,
        1,
        &tone(1000.0, 0.5, 3.0, 1.0, 2.0),
    );
    let html = composition(
        &dir,
        "<audio id='voice' src='voice.wav' data-start='0'></audio>\
         <audio id='music' src='music.wav' data-start='0' data-duration='3' data-hf-duck='voice' \
                data-hf-duck-amount='12' data-hf-duck-attack='0.2' data-hf-duck-hold='0.1' data-hf-duck-release='0.4'></audio>",
        3,
    );
    let mix = mixed(&html, 3);
    let music = 0.25_f32;
    let floor = music * 10f32.powf(-12.0 / 20.0);
    let at = |t: f64| mix.left[(t * RATE as f64) as usize];
    assert_eq!(at(0.5), music, "untouched before the look-ahead");
    assert!(
        at(0.9) < music && at(0.9) > floor,
        "attack ramps ahead of the voice"
    );
    assert!(
        (at(0.9999) - floor).abs() < 1e-4,
        "fully down when the voice starts"
    );
    assert!((at(2.09) - floor).abs() < 1e-4, "held after the voice");
    assert!(at(2.3) > floor && at(2.3) < music, "releasing");
    assert_eq!(at(2.6), music, "released");
    let duck = &mix.clips[1].effects.as_ref().unwrap()["duck"];
    let regions = duck["regions"].as_array().unwrap();
    assert_eq!(regions.len(), 1);
    assert!((regions[0][0].as_f64().unwrap() - 0.9).abs() < 1e-3);
    assert!((regions[0][1].as_f64().unwrap() - 2.3).abs() < 1e-3);
    assert_eq!(duck["deepestDb"], -12.0);
}

#[test]
fn a_ducked_trigger_order_is_independent_of_document_order() {
    // The music is written before the voice that keys it; the voice must still be mixed first.
    let dir = scratch("fx-order");
    write_wav(&dir.join("music.wav"), 48_000, 1, &constant(8192, 2.0));
    write_wav(
        &dir.join("voice.wav"),
        48_000,
        1,
        &tone(500.0, 0.5, 2.0, 0.5, 1.5),
    );
    let html = composition(
        &dir,
        "<audio id='music' src='music.wav' data-start='0' data-duration='2' data-hf-duck='voice' data-hf-duck-amount='20'></audio>\
         <audio id='voice' src='voice.wav' data-start='0' data-volume='0'></audio>",
        2,
    );
    // A silent voice never crosses the threshold: nothing ducks.
    let mix = mixed(&html, 2);
    assert!(mix.left.iter().all(|s| *s == 0.25));
    assert_eq!(
        mix.clips[0].id.as_deref(),
        Some("music"),
        "reports keep document order"
    );
}

#[test]
fn volume_ramp_multiplies_volume_and_holds_its_edges() {
    let dir = scratch("fx-ramp");
    write_wav(&dir.join("dc.wav"), 48_000, 1, &constant(16384, 3.0));
    let html = composition(
        &dir,
        "<audio src='dc.wav' data-start='0' data-volume='0.5' \
                data-hf-volume-ramp='[{\"at\":1,\"volume\":0},{\"at\":2,\"db\":0}]'></audio>",
        3,
    );
    let mix = mixed(&html, 3);
    let at = |t: f64| mix.left[(t * RATE as f64) as usize];
    assert_eq!(at(0.5), 0.0, "first value holds before the first keyframe");
    assert!((at(1.5) - 0.125).abs() < 1e-4);
    assert_eq!(at(2.5), 0.25, "last value holds");
}

#[test]
fn a_neutral_effect_chain_is_byte_identical_to_no_effects() {
    let dir = scratch("fx-neutral");
    write_wav(
        &dir.join("t.wav"),
        44_100,
        1,
        &tone(440.0, 0.7, 1.0, 0.0, 1.0),
    );
    let plain = mixed(
        &composition(
            &dir,
            "<audio src='t.wav' data-volume='0.8' data-hf-fade-in='0.2'></audio>",
            1,
        ),
        1,
    );
    let neutral = mixed(
        &composition(
            &dir,
            "<audio src='t.wav' data-volume='0.8' data-hf-fade-in='0.2' data-hf-volume-ramp='[{\"at\":0,\"volume\":1}]'></audio>",
            1,
        ),
        1,
    );
    assert_eq!(plain.left, neutral.left);
}

#[test]
fn eq_bands_and_presets_shape_the_spectrum() {
    let dir = scratch("fx-eq");
    write_wav(
        &dir.join("low.wav"),
        48_000,
        1,
        &tone(100.0, 0.5, 1.0, 0.0, 1.0),
    );
    write_wav(
        &dir.join("mid.wav"),
        48_000,
        1,
        &tone(1000.0, 0.5, 1.0, 0.0, 1.0),
    );
    let level = |src: &str, eq: &str| {
        let body = format!("<audio src='{src}' data-hf-eq='{eq}'></audio>");
        let mix = mixed(&composition(&dir, &body, 1), 1);
        db(rms(&mix.left[RATE / 2..]) / (0.5 / 2f64.sqrt()))
    };
    let highpass = r#"[{"type":"highpass","freq":1000}]"#;
    assert!(level("low.wav", highpass) < -38.0);
    assert!(level("mid.wav", highpass).abs() < 3.1 && level("mid.wav", highpass) < -2.9);
    let peak = r#"[{"type":"peak","freq":1000,"gain":6,"q":2}]"#;
    assert!((level("mid.wav", peak) - 6.0).abs() < 0.05);
    assert!(level("low.wav", peak).abs() < 0.1);
    assert!(level("low.wav", "telephone") < -15.0);
    let report = mixed(
        &composition(&dir, "<audio src='mid.wav' data-hf-eq='voice'></audio>", 1),
        1,
    );
    let eq = &report.clips[0].effects.as_ref().unwrap()["eq"];
    assert_eq!(eq["preset"], "voice");
    assert_eq!(eq["bands"].as_array().unwrap().len(), 4);
}

#[test]
fn rejects_invalid_effects_with_explicit_errors() {
    let dir = scratch("fx-errors");
    write_wav(&dir.join("a.wav"), 48_000, 1, &[0; 100]);
    let cases = [
        ("<audio id='m' src='a.wav' data-hf-duck='nobody'></audio>", "references nobody"),
        ("<audio id='m' src='a.wav' data-hf-duck='m'></audio>", "the clip itself"),
        (
            "<audio id='a' src='a.wav' data-hf-duck='b'></audio><audio id='b' src='a.wav' data-hf-duck='a'></audio>",
            "cycle",
        ),
        ("<audio src='a.wav' data-hf-duck-amount='6'></audio>", "needs data-hf-duck"),
        ("<audio id='v' src='a.wav'></audio><audio src='a.wav' data-hf-duck='v' data-hf-duck-amount='90'></audio>", "data-hf-duck-amount"),
        ("<audio id='v' src='a.wav'></audio><audio src='a.wav' data-hf-duck='v' data-hf-duck-threshold='-120dB'></audio>", "data-hf-duck-threshold"),
        ("<audio src='a.wav' data-hf-eq='loud'></audio>", "unknown preset"),
        ("<audio src='a.wav' data-hf-eq='[{\"type\":\"highpass\",\"freq\":80,\"gain\":3}]'></audio>", "takes no gain"),
        ("<audio src='a.wav' data-hf-eq='[{\"type\":\"peak\",\"freq\":80}]'></audio>", "needs gain"),
        ("<audio src='a.wav' data-hf-eq='[{\"type\":\"peak\",\"freq\":30000,\"gain\":1}]'></audio>", "freq"),
        ("<audio src='a.wav' data-hf-volume-ramp='[{\"at\":0,\"volume\":1,\"db\":0}]'></audio>", "not both"),
        ("<audio src='a.wav' data-hf-volume-ramp='[{\"at\":1},{\"at\":0.5}]'></audio>", "strictly increasing"),
        ("<audio src='a.wav' data-hf-volume-ramp='[{\"at\":0,\"db\":24}]'></audio>", "+18"),
        ("<audio src='a.wav' data-hf-volume-ramp='[{\"at\":0,\"gain\":1}]'></audio>", "unknown field"),
        (
            "<audio id='v' src='a.wav'></audio><audio id='v' src='a.wav'></audio>",
            "duplicate id: v",
        ),
    ];
    for (body, expected) in cases {
        let html = composition(&dir, body, 1);
        let error = format!("{:#}", parse_file(&html).unwrap_err());
        assert!(error.contains(expected), "{body}: {error}");
    }
}

/// Decoded left channel of the MP4's audio track with the priming samples dropped.
fn decode_left(mp4_path: &Path) -> Vec<f32> {
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
    frames
        .into_iter()
        .flat_map(|f| f.pcm)
        .step_by(2)
        .skip(PRIMING as usize)
        .map(|s| f32::from(s) / 32768.0)
        .collect()
}

#[test]
fn the_mp4_carries_the_ducked_mix_and_is_deterministic() {
    let dir = scratch("fx-mp4");
    write_wav(&dir.join("music.wav"), 48_000, 2, &{
        let mono = tone(220.0, 0.4, 3.0, 0.0, 3.0);
        mono.iter().flat_map(|s| [*s, *s]).collect::<Vec<_>>()
    });
    write_wav(
        &dir.join("voice.wav"),
        48_000,
        1,
        &tone(2000.0, 0.5, 3.0, 1.0, 1.2),
    );
    let html = composition(
        &dir,
        "<audio id='voice' src='voice.wav' data-start='0'></audio>\
         <audio id='music' src='music.wav' data-start='0' data-duration='3' data-hf-eq='music-bed' \
                data-hf-duck='voice' data-hf-duck-hold='0.7' data-hf-duck-release='0.3'></audio>",
        3,
    );
    let render = |name: &str, threads: u32| {
        let output = dir.join(name);
        let composition = parse_file(&html).unwrap();
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        encode_native(&mut renderer, &output, 30, threads).unwrap();
        output
    };
    let (a, b) = (render("a.mp4", 1), render("b.mp4", 3));
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let left = decode_left(&a);
    let window =
        |from: f64, to: f64| rms(&left[(from * RATE as f64) as usize..(to * RATE as f64) as usize]);
    // Music alone at full level, then music alone while held down after the voice.
    let reduction = db(window(1.3, 1.8) / window(0.2, 0.7));
    assert!(
        (reduction + 12.0).abs() < 0.5,
        "ducked by {reduction:.2} dB"
    );
    assert!(
        (db(window(2.5, 2.9) / window(0.2, 0.7))).abs() < 0.3,
        "released"
    );
}

#[test]
fn master_limiter_dither_and_fade_options_parse_and_apply() {
    let dir = scratch("fx-master");
    write_wav(&dir.join("loud.wav"), 48_000, 1, &constant(30_000, 1.0));
    write_wav(
        &dir.join("tone.wav"),
        48_000,
        1,
        &tone(440.0, 0.5, 0.5, 0.0, 0.5),
    );
    // Two loud clips sum past full scale; the limiter keeps the mix at -1 dBFS.
    let body = "<audio src='loud.wav'></audio><audio src='loud.wav'></audio>";
    let html = dir.join("limit.html");
    std::fs::write(
        &html,
        format!("<div data-composition-id='x' data-width='64' data-height='36' data-duration='1' data-hf-audio-limiter='-1' data-hf-audio-dither='tpdf' style='background:#123'>{body}</div>"),
    )
    .unwrap();
    let composition_ = parse_file(&html).unwrap();
    assert!(composition_.audio_master.dither);
    let limited = mix(&composition_, RATE).unwrap();
    let ceiling = 10f32.powf(-1.0 / 20.0);
    assert!(limited.peak <= ceiling + 1e-6, "{}", limited.peak);
    assert_eq!(limited.clipped, 0);
    assert!(limited.limiter.unwrap().max_reduction_db > 5.0);
    let unlimited = mixed(&composition(&dir, body, 1), 1);
    assert!(unlimited.clipped > 0 && unlimited.limiter.is_none());
    // Dither is deterministic and changes only the 16-bit conversion, by at most one step.
    let (a, b) = (
        limited.to_i16_interleaved(true),
        limited.to_i16_interleaved(true),
    );
    assert_eq!(a, b);
    let plain = limited.to_i16_interleaved(false);
    assert!(a.iter().zip(&plain).all(|(x, y)| (x - y).abs() <= 1));
    assert_ne!(a, plain);
    // A looped clip with a crossfaded seam fills the duration.
    let looped = mixed(
        &composition(
            &dir,
            "<audio src='tone.wav' data-duration='1' data-hf-loop data-hf-loop-crossfade='0.05' data-hf-fade-in='0.1' data-hf-fade-curve='equal-power'></audio>",
            1,
        ),
        1,
    );
    assert!(rms(&looped.left[40_000..44_000]) > 0.3);
}

#[test]
fn rejects_invalid_master_and_fade_options() {
    let dir = scratch("fx-master-errors");
    write_wav(&dir.join("a.wav"), 48_000, 1, &[0; 48_000]);
    let cases = [
        ("<audio src='a.wav' data-hf-fade-curve='log'></audio>", "data-hf-fade-curve"),
        ("<audio src='a.wav' data-duration='1' data-hf-loop-crossfade='0.1'></audio>", "needs data-hf-loop"),
        ("<audio src='a.wav' data-duration='1' data-hf-loop data-hf-loop-crossfade='3'></audio>", "between 0 and 2"),
        ("<audio src='a.wav' data-duration='2' data-hf-loop data-hf-loop-crossfade='0.6'></audio>", "half the looped part"),
        ("<audio src='a.wav' data-duration='1' data-hf-loop data-hf-speed='2' data-hf-loop-crossfade='0.1'></audio>", "data-hf-speed"),
        ("<audio id='v' src='a.wav'></audio><audio src='a.wav' data-hf-duck='v' data-hf-duck-detector='lufs'></audio>", "peak or rms"),
        ("<audio id='v' src='a.wav'></audio><audio src='a.wav' data-hf-duck='v' data-hf-duck-window='0.1'></audio>", "needs data-hf-duck-detector"),
        ("<audio id='v' src='a.wav'></audio><audio src='a.wav' data-hf-duck='v' data-hf-duck-detector='rms' data-hf-duck-window='5'></audio>", "data-hf-duck-window"),
    ];
    for (body, expected) in cases {
        let html = composition(&dir, body, 2);
        let error = match parse_file(&html) {
            Err(e) => format!("{e:#}"),
            Ok(c) => format!("{:#}", mix(&c, 2 * RATE).err().expect(body)),
        };
        assert!(error.contains(expected), "{body}: {error}");
    }
    for (root, expected) in [
        ("data-hf-audio-limiter='3'", "data-hf-audio-limiter"),
        ("data-hf-audio-dither='rect'", "tpdf or off"),
    ] {
        let html = dir.join("root.html");
        std::fs::write(&html, format!("<div data-composition-id='x' data-width='64' data-height='36' data-duration='1' {root}></div>")).unwrap();
        let error = format!("{:#}", parse_file(&html).unwrap_err());
        assert!(error.contains(expected), "{root}: {error}");
    }
}
