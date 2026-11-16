//! Safe-zone and caption-zone checks: findings, opt-outs, and 9:16 versus other frames.
mod common;

use aether_motion::agent::zones::{check_zones, default_zones, parse_caption_zone, Zone, ZoneOptions};
use common::scratch;
use serde_json::Value;

fn project(width: u32, height: u32, body: &str) -> std::path::PathBuf {
    let dir = scratch("zones");
    let html = format!(
        r#"<!doctype html><html><body>
<div id="root" style="background:#000000" data-composition-id="z" data-width="{width}" data-height="{height}" data-duration="2">
{body}
</div></body></html>"#
    );
    let path = dir.join("index.html");
    std::fs::write(&path, html).unwrap();
    path
}

fn text(id: &str, top: u32, extra: &str) -> String {
    format!(
        r#"<div id="{id}" class="clip" data-start="0" data-duration="2" data-track-index="0" {extra}
 style="left:100px;top:{top}px;width:880px;color:#ffffff;font-size:60px;text-align:center">Hello zone</div>"#
    )
}

fn options(zones: Vec<Zone>) -> ZoneOptions {
    ZoneOptions {
        zones,
        caption: None,
        times: vec![1.0],
    }
}

fn codes(report: &aether_motion::agent::zones::ZoneReport) -> Vec<(String, String)> {
    report
        .findings
        .iter()
        .map(|f| {
            (
                f["code"].as_str().unwrap().to_string(),
                f["element"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn text_in_the_bottom_ui_band_is_reported_with_platforms_and_edges() {
    // 1080x1920: y=1500 is inside the bottom 35% (Reels) and the bottom 672 px (Shorts).
    let path = project(1080, 1920, &(text("low", 1500, "") + &text("mid", 900, "")));
    let report = check_zones(&path, &options(default_zones(1080, 1920))).unwrap();
    assert_eq!(codes(&report), [("outside_safe_zone".into(), "low".into())]);
    let finding = &report.findings[0];
    let platforms: Vec<&str> = finding["platforms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(platforms.contains(&"reels") && platforms.contains(&"shorts"), "{platforms:?}");
    assert!(finding["edges"].as_array().unwrap().iter().any(|e| e == "bottom"));
    assert_eq!(finding["severity"], "warning");
}

#[test]
fn opt_out_attribute_silences_text_and_descendants() {
    let own = project(1080, 1920, &text("low", 1500, "data-layout-allow-safe-zone"));
    assert!(check_zones(&own, &options(default_zones(1080, 1920))).unwrap().findings.is_empty());

    let wrapped = project(
        1080,
        1920,
        &format!(
            r#"<div id="wrap" class="clip" data-start="0" data-duration="2" data-track-index="0" data-layout-allow-safe-zone style="left:0px;top:0px;width:1080px;height:1920px">{}</div>"#,
            text("low", 1500, "")
        ),
    );
    assert!(check_zones(&wrapped, &options(default_zones(1080, 1920))).unwrap().findings.is_empty());
}

#[test]
fn platform_zones_skip_non_vertical_frames_but_standard_applies() {
    // 1920x1080 with text 20 px from the left edge: outside the 10% title-safe margin.
    let body = r#"<div id="edge" class="clip" data-start="0" data-duration="2" data-track-index="0"
 style="left:20px;top:500px;width:600px;color:#ffffff;font-size:60px">Edge text</div>"#;
    let path = project(1920, 1080, body);
    let skipped = check_zones(&path, &options(vec![Zone::Tiktok])).unwrap();
    assert!(skipped.findings.is_empty() && skipped.notes[0].contains("9:16"));
    let standard = check_zones(&path, &options(default_zones(1920, 1080))).unwrap();
    assert_eq!(codes(&standard), [("outside_safe_zone".into(), "edge".into())]);
    assert_eq!(standard.findings[0]["platforms"][0], "standard");
}

#[test]
fn caption_zone_flags_text_in_the_band_unless_allowed() {
    let zone = parse_caption_zone("x0=0;y0=0.7;x1=1;y1=1;severity=error").unwrap();
    let opts = ZoneOptions {
        zones: Vec::new(),
        caption: Some(zone),
        times: vec![1.0],
    };
    let path = project(1080, 1920, &(text("title", 1500, "") + &text("top", 300, "")));
    let report = check_zones(&path, &opts).unwrap();
    assert_eq!(codes(&report), [("caption_zone_collision".into(), "title".into())]);
    assert!(report.caption_errors);

    let allowed = project(1080, 1920, &text("title", 1500, "data-layout-allow-caption-zone"));
    assert!(check_zones(&allowed, &opts).unwrap().findings.is_empty());
}

#[test]
fn generated_caption_words_collapse_into_one_finding_for_the_host() {
    let dir = scratch("zones-captions");
    std::fs::write(
        dir.join("t.json"),
        r#"[{"text":"Hello","start":0.2,"end":0.6},{"text":"wide","start":0.6,"end":1.0},{"text":"world","start":1.0,"end":1.5}]"#,
    )
    .unwrap();
    let body = r#"<div id="subs" class="clip" data-start="0" data-duration="2" data-track-index="0"
 style="left:0px;top:1700px;width:1080px;height:100px;color:#ffffff;font-size:64px;text-align:center"
 data-hf-captions="t.json"></div>"#;
    let html = format!(
        r#"<!doctype html><html><body><div id="root" style="background:#000000" data-composition-id="z" data-width="1080" data-height="1920" data-duration="2">{body}</div></body></html>"#
    );
    let path = dir.join("index.html");
    std::fs::write(&path, html).unwrap();
    let report = check_zones(
        &path,
        &ZoneOptions {
            zones: default_zones(1080, 1920),
            caption: None,
            times: vec![0.4, 0.8, 1.2],
        },
    )
    .unwrap();
    assert_eq!(codes(&report), [("outside_safe_zone".into(), "subs".into())]);
    let first: &Value = &report.findings[0];
    assert!((first["time"].as_f64().unwrap() - 0.4).abs() < 1e-9, "reported at the first time seen");
}
