//! Temporal regression tests for fractional motion, stable text zoom, and cache-independent frames.

mod common;
use aether_hyperframes::{parse::parse_file, render::Renderer};

fn fixture(name: &str, body: &str) -> std::path::PathBuf {
    let path = common::scratch(name).join("motion.html");
    std::fs::write(&path, format!(r#"<main data-composition-id="motion" data-width="320" data-height="180" data-duration="2" style="background:black">{body}</main>"#)).unwrap();
    path
}

fn mass_center(pixels: &[u8]) -> (f64, f64) {
    let (mut mass, mut moment) = (0.0, 0.0);
    for (i, pixel) in pixels.chunks_exact(4).enumerate() {
        mass += f64::from(pixel[0]);
        moment += f64::from(pixel[0]) * (i % 320) as f64;
    }
    (mass, moment / mass)
}

#[test]
fn motion_stability_fractional_box_moves_without_pulsing_or_holding() {
    let path = fixture(
        "fractional-box",
        r#"<div style="left:20px;top:40px;width:30px;height:20px;background:white" data-hf-keyframes='[{"at":0,"x":0},{"at":1,"x":15}]'></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let (mass, center) = mass_center(renderer.render_frame(0, 60).unwrap());
    for frame in 1..60 {
        let (actual_mass, actual_center) = mass_center(renderer.render_frame(frame, 60).unwrap());
        assert!(
            (actual_mass - mass).abs() <= 20.0,
            "frame {frame}: box area must stay constant"
        );
        assert!(
            (actual_center - center - frame as f64 * 0.25).abs() < 0.01,
            "frame {frame}: motion must retain its fractional position"
        );
    }
}

#[test]
fn motion_stability_text_moves_continuously_and_keeps_glyph_spacing() {
    let path = fixture(
        "fractional-text",
        r#"<div style="left:20px;top:40px;width:250px;height:40px;font-size:24px;color:white" data-hf-keyframes='[{"at":0,"x":0},{"at":1,"x":15}]'>Stable motion</div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let (mass, center) = mass_center(renderer.render_frame(0, 60).unwrap());
    for frame in 1..60 {
        let (actual_mass, actual_center) = mass_center(renderer.render_frame(frame, 60).unwrap());
        assert!(
            (actual_mass - mass).abs() / mass < 0.005,
            "frame {frame}: text must not pulse"
        );
        assert!(
            (actual_center - center - frame as f64 * 0.25).abs() < 0.03,
            "frame {frame}: text must move continuously"
        );
    }
}

#[test]
fn motion_stability_fractional_shadows_match_isolated_frames() {
    let path = fixture(
        "shadow-history",
        r#"<div style="left:40.25px;top:40.25px;width:80px;height:60px;box-shadow:0 4px 20px white" data-hf-keyframes='[{"at":0,"x":0},{"at":1,"x":15}]'></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut sequential = Renderer::new(&composition, 1.0).unwrap();
    for frame in 0..65 {
        let actual = sequential.render_frame(frame, 60).unwrap().to_vec();
        let mut isolated = sequential.fork().unwrap();
        assert_eq!(
            actual,
            isolated.render_frame(frame, 60).unwrap(),
            "frame {frame}: cache warmup and segment boundaries must not change pixels"
        );
    }
}

#[test]
fn motion_stability_camera_text_zoom_has_no_centroid_jumps() {
    let path = fixture(
        "text-zoom",
        r#"<div style="width:320px;height:180px" data-hf-camera='[{"at":0,"zoom":1},{"at":1,"zoom":1.08}]'><div style="left:20px;top:50px;width:280px;font-size:32px;color:white">AETHER</div></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let (_, center) = mass_center(renderer.render_frame(0, 60).unwrap());
    for frame in 1..60 {
        let (_, actual_center) = mass_center(renderer.render_frame(frame, 60).unwrap());
        let zoom = 1.0 + frame as f64 / 60.0 * 0.08;
        let expected = (center + 0.5 - 160.0) * zoom + 160.0 - 0.5;
        assert!(
            (actual_center - expected).abs() < 0.12,
            "frame {frame}: zoom centroid {actual_center} vs {expected}"
        );
    }
}

#[test]
fn motion_stability_mp4_has_120_ordered_samples_at_60_fps() {
    use aether_hyperframes::encode::encode_native;
    use std::{fs::File, io::BufReader};
    let path = fixture(
        "mp4-cadence",
        r#"<div style="left:20px;top:40px;width:30px;height:20px;background:white" data-hf-keyframes='[{"at":0,"x":0},{"at":2,"x":30}]'></div>"#,
    );
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    let output = path.with_extension("mp4");
    encode_native(&mut renderer, &output, 60, 2).unwrap();
    let file = File::open(output).unwrap();
    let length = file.metadata().unwrap().len();
    let mut reader = mp4::Mp4Reader::read_header(BufReader::new(file), length).unwrap();
    assert_eq!(reader.sample_count(1).unwrap(), 120);
    assert_eq!(reader.tracks()[&1].timescale(), 60);
    let mut decoder = openh264::decoder::Decoder::new().unwrap();
    let mut headers = Vec::new();
    for parameter in [
        reader.tracks()[&1].sequence_parameter_set().unwrap(),
        reader.tracks()[&1].picture_parameter_set().unwrap(),
    ] {
        headers.extend_from_slice(&[0, 0, 0, 1]);
        headers.extend_from_slice(parameter);
    }
    decoder.decode(&headers).unwrap();
    let mut rgb = vec![0u8; 320 * 180 * 3];
    let mut previous = None;
    for frame in 0..120 {
        let sample = reader.read_sample(1, frame + 1).unwrap().unwrap();
        assert_eq!(sample.start_time, frame as u64);
        assert_eq!(sample.duration, 1);
        assert_eq!(sample.rendering_offset, 0);
        let mut packet = Vec::new();
        let mut bytes = sample.bytes.as_ref();
        while !bytes.is_empty() {
            let len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            packet.extend_from_slice(&[0, 0, 0, 1]);
            packet.extend_from_slice(&bytes[4..4 + len]);
            bytes = &bytes[4 + len..];
        }
        let decoded = decoder
            .decode(&packet)
            .unwrap()
            .expect("every MP4 sample must decode a frame");
        decoded.write_rgb8(&mut rgb);
        let (mut mass, mut moment) = (0.0, 0.0);
        for (i, pixel) in rgb.chunks_exact(3).enumerate() {
            let value = f64::from(pixel[0].saturating_sub(10));
            mass += value;
            moment += value * (i % 320) as f64;
        }
        let center = moment / mass;
        assert!(
            (center - (34.5 + frame as f64 * 0.25)).abs() < 0.15,
            "decoded frame {frame}: center {center}"
        );
        if let Some(last) = previous {
            assert!(center > last, "decoded motion must not reverse or repeat");
        }
        previous = Some(center);
        if frame % 60 == 0 {
            assert!(sample.is_sync);
        }
    }
}
