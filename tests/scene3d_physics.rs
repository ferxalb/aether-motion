//! Launch diagnostics and the native hinged lamp across contact, shadowing, and split exports.

mod common;

use aether_motion::agent::check;
use aether_motion::encode::{assemble_parts, encode_native, render_part, EncodeOptions};
use aether_motion::model::Content;
use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;
use common::{scratch, Shot};
use std::{fs, path::Path, process::Command};

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/hinged-lamp.html");
const FPS: u32 = 12;

#[test]
fn inspector_reports_requested_and_solved_launch_vectors() {
    let dir = scratch("launch-limits");
    let path = dir.join("launch.html");
    fs::write(&path, r#"
      <div data-composition-id="limits" data-width="160" data-height="96" data-duration="0.1">
        <div id="lab" data-hf-3d style="width:160px;height:96px">
          <hf-body id="fast" type="dynamic" shape="sphere" radius="8"
                   velocity="80000 0 0" angular-velocity="0 0 21600"></hf-body>
          <hf-body id="faster" type="dynamic" shape="sphere" radius="8"
                   position="0 0 -100" velocity="1000000 0 0" angular-velocity="0 0 100000"></hf-body>
          <hf-body id="normal" type="dynamic" shape="sphere" radius="8"
                   position="0 0 -200" velocity="100 0 0"></hf-body>
        </div>
      </div>"#).unwrap();
    let report = check(&path);
    assert_eq!(report["ok"], true, "{report}");
    let bodies = &report["scenes3d"][0]["physics"]["bodies"];
    for (index, (linear_requested, angular_requested)) in
        [(80_000.0, 21_600.0), (1_000_000.0, 100_000.0)]
            .into_iter()
            .enumerate()
    {
        let launch = &bodies[index]["launch"];
        for (value, requested, component, cap) in [
            (&launch["linear"], linear_requested, 0, 40_000.0),
            (&launch["angular"], angular_requested, 2, 5_400.0),
        ] {
            assert_eq!(value["requested"][component], requested);
            assert!(
                (value["actual"][component].as_f64().unwrap() - cap).abs() < 0.1,
                "{value}"
            );
            assert_eq!(value["firstTickAffected"], 1);
            assert_eq!(value["actualTick"], 1);
        }
    }
    assert!(bodies[2]["launch"]["linear"]["firstTickAffected"].is_null());
    let output = Command::new(env!("CARGO_BIN_EXE_aether-motion"))
        .arg("inspect")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let table = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Requested (x, y, z)",
        "Actual at tick 1",
        "First cap tick",
        "80000.000",
        "1000000.000",
        "40000.000",
        "5400.000",
    ] {
        assert!(table.contains(expected), "{table}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lamp_contact_turns_the_hinge_and_the_pivot_holds() {
    let composition = parse_file(Path::new(EXAMPLE)).unwrap();
    let Content::Scene3d(scene) = &composition.elements[composition.ids["bay"]].content else {
        panic!("expected the fixture's native viewport");
    };
    let lamp = scene
        .nodes
        .iter()
        .position(|node| node.id.as_deref() == Some("lamp"))
        .unwrap();
    let pivot = scene
        .nodes
        .iter()
        .position(|node| node.id.as_deref() == Some("pivot"))
        .unwrap();
    let pose = |t| scene.world(t)[lamp];
    let angle = |t| {
        let transform = pose(t);
        transform.m[1][0].atan2(transform.m[0][0]).to_degrees()
    };
    let first_motion = (1..150)
        .find(|tick| angle(f64::from(*tick) / 120.0).abs() > 0.1)
        .unwrap();
    assert!(
        (95..=98).contains(&first_motion),
        "first motion at tick {first_motion}"
    );
    assert!(angle(1.0) < -10.0, "the left hit must swing the lamp right");
    assert!(angle(1.32) < -23.0, "the beam must reach the column");
    assert!(
        angle(2.0) > 5.0,
        "the hinge must let the lamp return through center"
    );
    let pivot_world = scene.world(0.0)[pivot].t;
    for tick in 0..=scene.physics.as_ref().unwrap().ticks() {
        let time = tick as f64 / 120.0;
        let anchor = pose(time).apply([0.0, -59.0, 0.0]);
        assert!(
            anchor
                .iter()
                .zip(pivot_world)
                .all(|(a, b)| (a - b).abs() < 0.6),
            "hinge drift at tick {tick}: {anchor:?}"
        );
        assert!(angle(time).abs() < 72.5);
    }
}

#[test]
fn a_box_casts_a_shaft_shadow_beyond_its_camera_silhouette() {
    let dir = scratch("beam-shadow");
    let path = dir.join("shaft.html");
    let markup = |occluder| {
        format!(
            r##"
      <div data-composition-id="shaft" data-width="160" data-height="96" data-duration="1" style="background:#000">
        <div data-hf-3d style="width:160px;height:96px">
          <hf-camera projection="orthographic"></hf-camera>
          <hf-mesh id="column" shape="box" size="6 4 30" position="0 -11 0"
                   material="unlit" color="#123456"></hf-mesh>
          <hf-beam direction="0 1 0" position="0 -31 0" length="75" radius="20"
                   density="0.16" color="#f90" {occluder}></hf-beam>
        </div>
      </div>"##
        )
    };
    let still = |occluder| {
        fs::write(&path, markup(occluder)).unwrap();
        let composition = parse_file(&path).unwrap();
        let mut renderer = Renderer::new(&composition, 1.0).unwrap();
        Shot {
            pixels: renderer.render_at(0.0).unwrap().to_vec(),
            width: 160,
            height: 96,
        }
    };
    let shadowed = still("occluder='#column'");
    let unshadowed = still("");
    assert!(
        unshadowed.at(80, 65)[0] > 100,
        "the shaft is visible without opaque geometry"
    );
    assert!(
        shadowed.at(80, 65)[0] < 15,
        "light is blocked beyond the column"
    );
    assert!(
        shadowed.at(90, 65)[0] > 25,
        "part of the beam passes beside the column"
    );
    assert_eq!(
        shadowed.at(80, 37),
        [0x12, 0x34, 0x56],
        "the opaque column wins the depth test"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lamp_export_matches_two_parts_split_during_the_swing() {
    let dir = scratch("split-lamp");
    let path = Path::new(EXAMPLE);
    let whole = dir.join("whole.mp4");
    let assembled = dir.join("assembled.mp4");
    let composition = parse_file(path).unwrap();
    let mut renderer = Renderer::new(&composition, 0.25).unwrap();
    encode_native(&mut renderer, &whole, FPS, 1).unwrap();
    let parts = [dir.join("first.aseg"), dir.join("rest.aseg")];
    for (part, range) in parts.iter().zip([0..1, 1..3]) {
        // Compile independently, as a separate segment worker would.
        let composition = parse_file(path).unwrap();
        let mut renderer = Renderer::new(&composition, 0.25).unwrap();
        render_part(&mut renderer, part, &EncodeOptions::new(FPS, 1), range).unwrap();
    }
    assemble_parts(&composition, &parts, &assembled, 192_000).unwrap();
    assert!(
        fs::read(whole).unwrap() == fs::read(assembled).unwrap(),
        "the split changed the lamp export"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn beam_viewports_are_preflighted_before_the_first_visible_frame() {
    let dir = scratch("beam-backend");
    let path = dir.join("future-shaft.html");
    fs::write(&path, r#"
      <div data-composition-id="future-shaft" data-width="160" data-height="96" data-duration="2">
        <div id="future" data-start="1" data-duration="1" data-hf-3d style="width:160px;height:96px">
          <hf-beam></hf-beam>
        </div>
      </div>"#).unwrap();
    let binary = env!("CARGO_BIN_EXE_aether-motion");
    let check = Command::new(binary)
        .args(["check"])
        .arg(&path)
        .env("AETHER_3D_BACKEND", "gpu")
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(report["scenes3d"][0]["backend"], "cpu");
    assert!(report["scenes3d"][0]["gpuFallback"]
        .as_str()
        .unwrap()
        .contains("require the CPU"));
    let output = dir.join("frame.png");
    let strict = Command::new(binary)
        .arg("render")
        .arg(&path)
        .args(["--3d-backend", "gpu", "--require-gpu", "-o"])
        .arg(&output)
        .output()
        .unwrap();
    assert!(!strict.status.success());
    assert!(
        String::from_utf8_lossy(&strict.stderr).contains("<hf-beam> light shafts require the CPU")
    );
    assert!(!output.exists());
    fs::remove_dir_all(dir).unwrap();
}
