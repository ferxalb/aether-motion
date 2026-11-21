//! CLI contract, run through the real binary. `render`, `check`, and `preview` with a file run
//! standalone and never read `aether.json`; without one they use the project. `-o` and `--output`
//! are one option; PNGs are overwritten while named MP4s need `--force`; a failed render never
//! destroys the previous output; `check` keeps its JSON report; and every mode rejects the same
//! mistakes with the same message and exit code (2 for usage errors, 1 for failures).
mod common;

use common::scratch;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_aether-motion");

/// A 160x90, 0.2 s composition (6 frames at 30 fps) with a moving box.
const HTML: &str = "<div data-composition-id='c' data-width='160' data-height='90' data-duration='0.2' style='background:#123'>\
    <div class='clip' data-start='0' data-duration='0.2' data-track-index='0' style='left:10px;top:20px;width:30px;height:30px;background:#fc0'\
     data-hf-keyframes='[{\"at\":0,\"x\":0},{\"at\":0.2,\"x\":90}]'></div></div>";

/// A fresh folder holding `index.html` and, optionally, an `aether.json`.
fn project(name: &str, config: Option<&str>) -> PathBuf {
    let dir = scratch(&format!("cli-{name}"));
    fs::write(dir.join("index.html"), HTML).unwrap();
    if let Some(config) = config {
        fs::write(dir.join("aether.json"), config).unwrap();
    }
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .env_remove("CLICOLOR_FORCE")
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Width and height from a PNG's IHDR chunk.
fn png_size(path: &Path) -> (u32, u32) {
    let bytes = fs::read(path).unwrap();
    let be = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
    (be(16), be(20))
}

#[test]
fn add_list_prints_catalog_without_editing_the_composition() {
    let dir = project("add-list", None);
    let default = run(&dir, &["add"]);
    let explicit = run(&dir, &["add", "list"]);
    let extra = run(&dir, &["add", "list", "extra"]);

    assert_eq!((code(&default), code(&explicit), code(&extra)), (0, 0, 2));
    assert_eq!(explicit.stdout, default.stdout);
    assert!(String::from_utf8_lossy(&explicit.stdout).contains("Blocks"));
    assert_eq!(fs::read_to_string(dir.join("index.html")).unwrap(), HTML);
}

#[test]
fn short_and_long_output_flags_are_one_option() {
    let dir = project("alias", None);
    for (mode, file) in [("direct", Some("index.html")), ("project", None)] {
        let with = |flag: &str, name: &str| {
            let mut args = vec!["render"];
            args.extend(file);
            args.extend([flag, name, "--frame", "3"]);
            run(&dir, &args)
        };
        let short = with("-o", &format!("{mode}-short.png"));
        let long = with("--output", &format!("{mode}-long.png"));
        assert_eq!(
            (code(&short), code(&long)),
            (0, 0),
            "{mode}: {}",
            stderr(&short)
        );
        assert_eq!(
            fs::read(dir.join(format!("{mode}-short.png"))).unwrap(),
            fs::read(dir.join(format!("{mode}-long.png"))).unwrap(),
            "{mode}: -o and --output drew different frames"
        );
        // The same mistake under either spelling: same message, same exit code.
        let mut base = vec!["render"];
        base.extend(file);
        let short = run(
            &dir,
            &[&base[..], &["-o", "x.mp4", "--frame", "1"]].concat(),
        );
        let long = run(
            &dir,
            &[&base[..], &["--output", "x.mp4", "--frame", "1"]].concat(),
        );
        assert_eq!((code(&short), code(&long)), (2, 2));
        assert_eq!(stderr(&short), stderr(&long));
    }
}

#[test]
fn direct_mode_never_reads_aether_json() {
    let dir = project("isolation", Some(r#"{"size": "1080p", "out": "dist"}"#));
    assert_eq!(code(&run(&dir, &["render", "-o", "project.png"])), 0);
    assert_eq!(png_size(&dir.join("project.png")), (1920, 1080));
    assert_eq!(
        code(&run(&dir, &["render", "index.html", "-o", "direct.png"])),
        0
    );
    assert_eq!(png_size(&dir.join("direct.png")), (160, 90));
    // Defaults: the project's "out" folder in project mode, ./out in direct mode.
    assert_eq!(code(&run(&dir, &["render", "--scale", "1"])), 0);
    assert!(dir.join("dist/index.mp4").is_file());
    assert_eq!(
        code(&run(&dir, &["render", "index.html", "--scale", "1"])),
        0
    );
    assert!(dir.join("out/index.mp4").is_file());
    // A broken aether.json fails project mode only.
    fs::write(dir.join("aether.json"), "{ not json").unwrap();
    let project = run(&dir, &["render", "-o", "p.png"]);
    assert_eq!(code(&project), 1);
    assert!(
        stderr(&project).contains("aether.json"),
        "{}",
        stderr(&project)
    );
    assert_eq!(
        code(&run(&dir, &["render", "index.html", "-o", "d.png"])),
        0
    );
}

#[test]
fn pngs_overwrite_and_named_mp4s_need_force() {
    let dir = project("overwrite", None);
    for args in [
        &[
            "render",
            "index.html",
            "--output",
            "still.png",
            "--frame",
            "0",
        ][..],
        &[
            "render",
            "index.html",
            "--output",
            "still.png",
            "--frame",
            "5",
        ],
        &["render", "-o", "still.png", "--frame", "2"],
    ] {
        assert_eq!(code(&run(&dir, args)), 0, "{args:?}");
    }
    for base in [&["render", "index.html"][..], &["render"]] {
        let named = |extra: &[&str]| {
            run(
                &dir,
                &[base, &["-o", "clip.mp4", "--scale", "1"], extra].concat(),
            )
        };
        let _ = fs::remove_file(dir.join("clip.mp4"));
        assert_eq!(code(&named(&[])), 0);
        let before = fs::read(dir.join("clip.mp4")).unwrap();
        let refused = named(&[]);
        assert_eq!(code(&refused), 2, "{base:?}");
        assert!(stderr(&refused).contains("clip.mp4 already exists"));
        assert!(stderr(&refused).contains("--force"));
        assert_eq!(
            fs::read(dir.join("clip.mp4")).unwrap(),
            before,
            "refusal touched the file"
        );
        assert_eq!(code(&named(&["-f"])), 0);
        assert_eq!(code(&named(&["--force"])), 0);
    }
    // The default project export is the project's own file and is replaced on every render.
    assert_eq!(code(&run(&dir, &["render", "--scale", "1"])), 0);
    assert_eq!(code(&run(&dir, &["render", "--scale", "1"])), 0);
}

#[test]
fn both_modes_reject_the_same_mistakes_the_same_way() {
    let dir = project("usage", None);
    for mistake in [
        &["--threads", "0"][..],
        &["--audio-bitrate", "32"],
        &["--size", "4k", "--scale", "2"],
        &["--qualty", "high"],
        &["--quality", "ultra"],
        &["--fps"],
        &["--fps", "0"],
        &["-o", "clip.gif"],
        &["-o", "a.png", "--frames", "0,1"],
        &["-o", "a.png", "--frame", "99"],
        &["-o", "a.png", "--segments", "0..1"],
        &["-o", "a.png", "--encoder", "ffmpeg"],
    ] {
        let project = run(&dir, &[&["render"][..], mistake].concat());
        let direct = run(&dir, &[&["render", "index.html"][..], mistake].concat());
        assert_eq!(
            (code(&project), code(&direct)),
            (2, 2),
            "{mistake:?}: {}",
            stderr(&direct)
        );
        assert!(stderr(&direct).starts_with("error: "), "{mistake:?}");
        assert!(
            stderr(&direct).contains("aether-motion render --help"),
            "{mistake:?}"
        );
        assert_eq!(stderr(&project), stderr(&direct), "{mistake:?}");
    }
    let two = run(&dir, &["render", "index.html", "other.html"]);
    assert_eq!(code(&two), 2);
    let unknown = run(&dir, &["rendr"]);
    assert_eq!(code(&unknown), 2);
    assert!(stderr(&unknown).contains("did you mean `aether-motion render`?"));
}

#[test]
fn failures_exit_1_and_leave_outputs_alone() {
    let dir = project("failure", None);
    fs::write(
        dir.join("bad.html"),
        HTML.replace("data-duration='0.2' style", "data-duration='x' style"),
    )
    .unwrap();
    fs::write(dir.join("keep.mp4"), b"previous").unwrap();
    let out = run(&dir, &["render", "bad.html", "-o", "keep.mp4", "--force"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).starts_with("error: "));
    // The composition failed before anything was replaced.
    assert_eq!(fs::read(dir.join("keep.mp4")).unwrap(), b"previous");
    assert_eq!(
        code(&run(&dir, &["render", "missing.html", "-o", "a.png"])),
        1
    );
}

/// A composition whose background is the color variable `tint` (default red).
const TINTED: &str = r##"<div data-composition-id='v' data-width='160' data-height='90' data-duration='0.2' data-composition-variables='[{"id":"tint","type":"color","label":"Tint","default":"#ff0000"}]' style='background:var(--tint)'></div>"##;

/// The project sets `tint` to blue and 24 fps.
const TINTED_PROJECT: &str = r##"{"variables": {"tint": "#0000ff"}, "fps": 24, "size": "1080p"}"##;

/// The RGB of a PNG's center pixel, and its size.
fn center(path: &Path) -> ([u8; 3], (u32, u32)) {
    let image = image::open(path).unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    (image.get_pixel(w / 2, h / 2).0, (w, h))
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", stderr(out)))
}

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn check_and_preview_with_a_file_never_read_aether_json() {
    let dir = scratch("cli-modes");
    fs::write(dir.join("index.html"), TINTED).unwrap();
    fs::write(dir.join("aether.json"), TINTED_PROJECT).unwrap();
    let tint = |out: &Output| json(out)["variables"]["declared"][0].clone();

    // Project mode: values, fps, and size from aether.json.
    let project = run(&dir, &["check", "--json"]);
    assert_eq!(code(&project), 0);
    assert_eq!(tint(&project)["value"], "#0000ff");
    assert_eq!(tint(&project)["overridden"], true);
    assert_eq!(code(&run(&dir, &["preview", "--no-open"])), 0);
    assert_eq!(center(&dir.join(".aether/preview.png")).0, BLUE);
    assert_eq!(code(&run(&dir, &["render", "-o", "p.png"])), 0);
    assert_eq!(center(&dir.join("p.png")), (BLUE, (1920, 1080)));

    // Standalone: defaults only.
    let alone = run(&dir, &["check", "index.html", "--json"]);
    assert_eq!(code(&alone), 0);
    assert_eq!(tint(&alone)["value"], "#ff0000");
    assert_eq!(tint(&alone)["overridden"], false);
    assert_eq!(code(&run(&dir, &["preview", "index.html", "--no-open"])), 0);
    assert_eq!(center(&dir.join(".aether/preview.png")).0, RED);
    assert_eq!(
        code(&run(&dir, &["render", "index.html", "-o", "d.png"])),
        0
    );
    assert_eq!(center(&dir.join("d.png")), (RED, (160, 90)));
    // Flags still apply standalone.
    let flagged = run(
        &dir,
        &[
            "check",
            "index.html",
            "--json",
            "--variables",
            r##"{"tint":"#00ff00"}"##,
        ],
    );
    assert_eq!(tint(&flagged)["value"], "#00ff00");

    // A broken aether.json changes nothing standalone and fails project mode with no report.
    fs::write(dir.join("aether.json"), "{ nope").unwrap();
    let again = run(&dir, &["check", "index.html", "--json"]);
    assert_eq!(
        (code(&again), &again.stdout),
        (0, &alone.stdout),
        "standalone check changed"
    );
    assert_eq!(code(&run(&dir, &["preview", "index.html", "--no-open"])), 0);
    for args in [
        &["check", "--json"][..],
        &["preview", "--no-open"],
        &["render", "-o", "x.png"],
    ] {
        let out = run(&dir, args);
        assert_eq!(code(&out), 1, "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?} printed a partial report");
        assert!(
            stderr(&out).contains("aether.json is not valid JSON"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    // An invalid project value is also a project-mode failure only.
    fs::write(dir.join("aether.json"), r#"{"fps": 500}"#).unwrap();
    assert_eq!(code(&run(&dir, &["preview", "--no-open"])), 1);
    assert_eq!(code(&run(&dir, &["preview", "index.html", "--no-open"])), 0);
}

#[test]
fn check_keeps_its_json_report() {
    let dir = project("check-json", None);
    let piped = run(&dir, &["check", "index.html"]);
    let flagged = run(&dir, &["check", "index.html", "--json"]);
    assert_eq!(code(&piped), 0);
    // Piped stdout is the JSON report, exactly as with --json.
    assert_eq!(piped.stdout, flagged.stdout);
    let report = json(&piped);
    for key in [
        "ok",
        "width",
        "height",
        "duration",
        "elements",
        "clips",
        "animations",
        "audio",
        "videos",
        "cameras",
        "effects",
        "dependencies",
        "variables",
        "scenes3d",
        "layout",
    ] {
        assert!(report.get(key).is_some(), "report lost {key}");
    }
    assert_eq!(
        (report["ok"].clone(), report["width"].clone()),
        (true.into(), 160.into())
    );
    // An invalid composition is still a JSON report with ok, error, and fix, and exit code 1.
    fs::write(
        dir.join("bad.html"),
        HTML.replace("data-duration='0.2' style", "data-duration='x' style"),
    )
    .unwrap();
    let bad = run(&dir, &["check", "bad.html"]);
    assert_eq!(code(&bad), 1);
    let report = json(&bad);
    assert_eq!(report["ok"], false);
    assert!(report["error"].is_string() && report["fix"].is_string());
}

#[test]
fn check_and_preview_reject_mistakes_like_render() {
    let dir = project("check-usage", None);
    for (command, mistake) in [
        ("check", &["--samples", "x"][..]),
        ("check", &["--safe-zone", "nowhere"]),
        ("check", &["--variables", "[1]"]),
        ("check", &["--jsno"]),
        ("preview", &["--fps", "0"]),
        ("preview", &["--at", "-1"]),
        ("preview", &["--variables", "{"]),
        ("preview", &["--wacth"]),
    ] {
        let extra: &[&str] = if command == "preview" {
            &["--no-open"]
        } else {
            &[]
        };
        let project = run(&dir, &[&[command][..], extra, mistake].concat());
        let alone = run(
            &dir,
            &[&[command, "index.html"][..], extra, mistake].concat(),
        );
        assert_eq!(
            (code(&project), code(&alone)),
            (2, 2),
            "{command} {mistake:?}: {}",
            stderr(&alone)
        );
        assert_eq!(stderr(&project), stderr(&alone), "{command} {mistake:?}");
        assert!(stderr(&alone).contains(&format!("aether-motion {command} --help")));
    }
    for command in ["check", "preview"] {
        let two = run(&dir, &[command, "index.html", "other.html"]);
        assert_eq!(code(&two), 2, "{command}");
        assert!(stderr(&two).contains("takes one composition"));
        let missing = run(
            &dir,
            &[command, "missing.html", "--no-open"][..if command == "preview" { 3 } else { 2 }],
        );
        assert_eq!(code(&missing), 1, "{command}");
        assert!(stderr(&missing).contains("composition not found: missing.html"));
    }
}

#[test]
fn failed_renders_keep_the_previous_output() {
    let dir = scratch("cli-safe");
    // Renders fine for 0.1 s, then a missing image fails the export partway through.
    let late = HTML.replace(
        "</div></div>",
        "</div><img class='clip' data-start='0.1' data-duration='0.1' data-track-index='1' src='gone.png' style='left:0;top:0;width:20px;height:20px'></div>",
    );
    fs::write(dir.join("index.html"), &late).unwrap();
    fs::write(dir.join("ok.html"), HTML).unwrap();
    fs::create_dir_all(dir.join("out")).unwrap();
    let keep = |name: &str| fs::write(dir.join(name), b"previous").unwrap();
    keep("named.mp4");
    keep("out/index.mp4");
    keep("still.png");
    keep("odd.mp4");
    for (args, file) in [
        // A named MP4 replaced with --force: the encode fails at frame 3.
        (
            &["render", "index.html", "-o", "named.mp4", "--force"][..],
            "named.mp4",
        ),
        // The automatic project output, replaced on every render.
        (&["render"], "out/index.mp4"),
        // A PNG whose frame fails to draw.
        (
            &["render", "index.html", "-o", "still.png", "--frame", "4"],
            "still.png",
        ),
        // The encoder refuses odd dimensions after the composition parsed.
        (
            &[
                "render", "ok.html", "-o", "odd.mp4", "--force", "--scale", "0.5",
            ],
            "odd.mp4",
        ),
    ] {
        let out = run(&dir, args);
        assert_eq!(code(&out), 1, "{args:?}: {}", stderr(&out));
        assert!(stderr(&out).contains("error: "), "{args:?}");
        assert_eq!(
            fs::read(dir.join(file)).unwrap(),
            b"previous",
            "{args:?} destroyed {file}"
        );
    }
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .chain(fs::read_dir(dir.join("out")).unwrap())
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("aether-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging files left behind: {leftovers:?}"
    );
    // A successful --force swaps the new file in.
    assert_eq!(
        code(&run(
            &dir,
            &["render", "ok.html", "-o", "named.mp4", "--force"]
        )),
        0
    );
    assert_ne!(fs::read(dir.join("named.mp4")).unwrap(), b"previous");
}
