//! Zone checks: warns when text leaves a platform safe area (`--safe-zone`) or overlaps a reserved
//! caption band (`--caption-zone`).
//!
//! The composition is probed at sample times (`Renderer::probe_text_at`) and each text ink box is
//! compared with the excluded regions of every selected zone. Findings are grouped by their owner,
//! the nearest ancestor-or-self with an `id` (so the generated words of one captions element become
//! one finding), and reported at the first time they are seen.
//!
//! Zone numbers: Reels, Shorts, and the 10% `standard` margins follow HyperFrames' documented
//! definitions. TikTok's exact stepped edge is not documented there; it uses the TikTok In-Feed
//! template drawn at 720x1280 (160 top, 440 bottom, 80 sides, and a 200-wide button column starting
//! 720 above the bottom edge), scaled to the frame. All zones are reference guides, not guarantees.
//! Opt out with `data-layout-allow-safe-zone` / `data-layout-allow-caption-zone` on the text or any
//! ancestor.
use crate::model::Composition;
use crate::parse::parse_file;
use crate::render::Renderer;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::path::Path;

/// Overlap below this many composition px is rounding noise, not a violation.
const TOLERANCE: f32 = 0.5;
/// Findings kept in a report; more are counted in `truncated`.
const MAX_FINDINGS: usize = 100;
/// Most sample times one check renders.
const MAX_SAMPLES: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Tiktok,
    Reels,
    Shorts,
    Standard,
}

impl Zone {
    pub const PLATFORMS: [Zone; 3] = [Zone::Tiktok, Zone::Reels, Zone::Shorts];

    pub fn name(self) -> &'static str {
        match self {
            Zone::Tiktok => "tiktok",
            Zone::Reels => "reels",
            Zone::Shorts => "shorts",
            Zone::Standard => "standard",
        }
    }

    fn is_platform(self) -> bool {
        self != Zone::Standard
    }

    /// Excluded regions as `(edge, [left, top, right, bottom])` in composition px.
    fn blocked(self, w: f32, h: f32) -> Vec<(&'static str, [f32; 4])> {
        let edges = |l: f32, t: f32, r: f32, b: f32| {
            vec![
                ("left", [0.0, 0.0, l, h]),
                ("top", [0.0, 0.0, w, t]),
                ("right", [w - r, 0.0, w, h]),
                ("bottom", [0.0, h - b, w, h]),
            ]
        };
        match self {
            Zone::Standard => edges(w * 0.1, h * 0.1, w * 0.1, h * 0.1),
            Zone::Reels => edges(w * 0.06, h * 0.14, w * 0.06, h * 0.35),
            Zone::Shorts => edges(
                w * 48.0 / 1080.0,
                h * 288.0 / 1920.0,
                w * 192.0 / 1080.0,
                h * 672.0 / 1920.0,
            ),
            Zone::Tiktok => {
                let (fx, fy) = (w / 720.0, h / 1280.0);
                let step = 560.0 * fy;
                vec![
                    ("left", [0.0, 0.0, 80.0 * fx, h]),
                    ("top", [0.0, 0.0, w, 160.0 * fy]),
                    ("bottom", [0.0, h - 440.0 * fy, w, h]),
                    ("right", [w - 80.0 * fx, 0.0, w, step]),
                    ("right", [w - 200.0 * fx, step, w, h]),
                ]
            }
        }
    }
}

/// Parses `--safe-zone`: `off`, or a comma list of `tiktok`, `reels`, `shorts`, `standard`.
/// `None` means no zone was requested: use `default_zones`.
pub fn parse_zones(raw: Option<&str>) -> Result<Option<Vec<Zone>>> {
    let Some(raw) = raw else { return Ok(None) };
    if raw == "off" {
        return Ok(Some(Vec::new()));
    }
    let mut zones = Vec::new();
    for part in raw.split(',') {
        zones.push(match part.trim() {
            "tiktok" => Zone::Tiktok,
            "reels" => Zone::Reels,
            "shorts" => Zone::Shorts,
            "standard" => Zone::Standard,
            other => bail!("--safe-zone must be tiktok, reels, shorts, standard, or off: {other}"),
        });
    }
    Ok(Some(zones))
}

/// True for 9:16 frames (within 0.5%), where the platform zones apply.
pub fn is_vertical(width: u32, height: u32) -> bool {
    (width as f64 / height as f64 - 9.0 / 16.0).abs() < 0.005 * 9.0 / 16.0
}

/// Platform zones on 9:16 frames, title-safe margins on any other frame.
pub fn default_zones(width: u32, height: u32) -> Vec<Zone> {
    if is_vertical(width, height) {
        Zone::PLATFORMS.to_vec()
    } else {
        vec![Zone::Standard]
    }
}

/// A fractional band reserved for captions: `x0=0;y0=0.7;x1=1;y1=1[;severity=warning|error]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptionZone {
    pub rect: [f32; 4],
    pub error: bool,
}

pub fn parse_caption_zone(raw: &str) -> Result<CaptionZone> {
    let (mut x0, mut y0, mut x1, mut y1) = (None, None, None, None);
    let mut error = false;
    for pair in raw.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("--caption-zone expects key=value pairs: {pair}"))?;
        let number = || -> Result<f32> {
            value
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|v| (0.0..=1.0).contains(v))
                .ok_or_else(|| anyhow!("--caption-zone {key} must be a fraction from 0 to 1: {value}"))
        };
        match key.trim() {
            "x0" => x0 = Some(number()?),
            "y0" => y0 = Some(number()?),
            "x1" => x1 = Some(number()?),
            "y1" => y1 = Some(number()?),
            "severity" => {
                error = match value.trim() {
                    "warning" => false,
                    "error" => true,
                    other => bail!("--caption-zone severity must be warning or error: {other}"),
                }
            }
            other => bail!("--caption-zone does not support {other}; use x0, y0, x1, y1, and severity"),
        }
    }
    let (Some(x0), Some(y0), Some(x1), Some(y1)) = (x0, y0, x1, y1) else {
        bail!("--caption-zone needs x0, y0, x1, and y1");
    };
    if x0 >= x1 || y0 >= y1 {
        bail!("--caption-zone needs x0 < x1 and y0 < y1");
    }
    Ok(CaptionZone {
        rect: [x0, y0, x1, y1],
        error,
    })
}

/// Sample times: `at` when given, otherwise `samples` midpoints of equal slices of the duration.
pub fn sample_times(duration: f64, samples: usize, at: &[f64]) -> Result<Vec<f64>> {
    if !at.is_empty() {
        if let Some(bad) = at.iter().find(|t| !t.is_finite() || **t < 0.0 || **t >= duration) {
            bail!("--at time {bad} is outside the composition (0 to {duration} s)");
        }
        return Ok(at.to_vec());
    }
    if samples == 0 || samples > MAX_SAMPLES {
        bail!("--samples must be between 1 and {MAX_SAMPLES}");
    }
    Ok((0..samples)
        .map(|i| (i as f64 + 0.5) / samples as f64 * duration)
        .collect())
}

/// Raw zone-check inputs shared by the CLI flags and the `aether_check` arguments.
#[derive(Clone, Debug, Default)]
pub struct ZoneRequest<'a> {
    pub safe_zone: Option<&'a str>,
    pub caption_zone: Option<&'a str>,
    pub samples: Option<usize>,
    pub at: Vec<f64>,
}

impl ZoneRequest<'_> {
    /// Resolves the request for a `width`x`height` composition lasting `duration` seconds;
    /// `None` when no zone is selected (`--safe-zone off` and no caption band).
    pub fn plan(&self, width: u32, height: u32, duration: f64) -> Result<Option<ZoneOptions>> {
        let requested = parse_zones(self.safe_zone)?;
        let caption = self.caption_zone.map(parse_caption_zone).transpose()?;
        let zones = requested.unwrap_or_else(|| default_zones(width, height));
        if zones.is_empty() && caption.is_none() {
            return Ok(None);
        }
        Ok(Some(ZoneOptions {
            zones,
            caption,
            times: sample_times(duration, self.samples.unwrap_or(9), &self.at)?,
        }))
    }
}

/// Adds `layout` (findings, truncated, notes, samples) to a successful `agent::check` report.
/// Reports that failed validation are left alone. Returns the zone report when zones ran.
pub fn attach(report: &mut Value, path: &Path, request: &ZoneRequest) -> Result<Option<ZoneReport>> {
    if report["ok"] != true {
        return Ok(None);
    }
    let (width, height) = (
        report["width"].as_u64().unwrap_or(0) as u32,
        report["height"].as_u64().unwrap_or(0) as u32,
    );
    let duration = report["duration"].as_f64().unwrap_or(0.0);
    let Some(options) = request.plan(width, height, duration)? else {
        return Ok(None);
    };
    let outcome = check_zones(path, &options)?;
    report["layout"] = json!({
        "findings": outcome.findings,
        "truncated": outcome.truncated,
        "notes": outcome.notes,
        "samples": outcome.times,
    });
    Ok(Some(outcome))
}

#[derive(Clone, Debug, Default)]
pub struct ZoneOptions {
    pub zones: Vec<Zone>,
    pub caption: Option<CaptionZone>,
    pub times: Vec<f64>,
}

/// Result of a zone check. `failed` is true when a finding must change the exit code.
#[derive(Debug, Default)]
pub struct ZoneReport {
    pub findings: Vec<Value>,
    pub truncated: usize,
    pub notes: Vec<String>,
    pub times: Vec<f64>,
    /// Number of `caption_zone_collision` findings (warnings or errors).
    pub caption_collisions: usize,
    /// True when a caption-zone finding has `severity` `error`.
    pub caption_errors: bool,
}

struct Draft {
    owner: usize,
    time: f64,
    rect: [f32; 4],
    text: String,
    /// `(zone name, edges)` per violated zone, in zone order.
    zones: Vec<(&'static str, Vec<&'static str>)>,
}

fn overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[2].min(b[2]) - a[0].max(b[0]) > TOLERANCE && a[3].min(b[3]) - a[1].max(b[1]) > TOLERANCE
}

/// Nearest ancestor-or-self with an `id` (or the root).
fn owner_of(c: &Composition, mut idx: usize) -> usize {
    while c.elements[idx].id.is_none() {
        match c.elements[idx].parent {
            Some(parent) => idx = parent,
            None => break,
        }
    }
    idx
}

fn allowed(c: &Composition, mut idx: usize, pick: impl Fn(&crate::model::LayoutHints) -> bool) -> bool {
    loop {
        if pick(&c.elements[idx].layout) {
            return true;
        }
        match c.elements[idx].parent {
            Some(parent) => idx = parent,
            None => return false,
        }
    }
}

fn snippet(c: &Composition, idx: usize) -> String {
    match &c.elements[idx].content {
        crate::model::Content::Text(text) => text.chars().take(40).collect(),
        _ => String::new(),
    }
}

fn label(c: &Composition, idx: usize) -> String {
    c.elements[idx]
        .id
        .clone()
        .unwrap_or_else(|| format!("<{}>", c.elements[idx].tag))
}

fn merge(drafts: &mut Vec<Draft>, new: Draft) {
    let Some(existing) = drafts.iter_mut().find(|d| d.owner == new.owner) else {
        drafts.push(new);
        return;
    };
    for (zone, edges) in new.zones {
        match existing.zones.iter_mut().find(|(name, _)| *name == zone) {
            Some((_, known)) => {
                for edge in edges {
                    if !known.contains(&edge) {
                        known.push(edge);
                    }
                }
            }
            None => existing.zones.push((zone, edges)),
        }
    }
    let r = &mut existing.rect;
    *r = [r[0].min(new.rect[0]), r[1].min(new.rect[1]), r[2].max(new.rect[2]), r[3].max(new.rect[3])];
}

fn box_json(rect: [f32; 4]) -> Value {
    let round = |v: f32| (f64::from(v) * 10.0).round() / 10.0;
    json!({"x": round(rect[0]), "y": round(rect[1]), "w": round(rect[2] - rect[0]), "h": round(rect[3] - rect[1])})
}

/// Probes the composition at `options.times` and reports zone findings.
pub fn check_zones(path: &Path, options: &ZoneOptions) -> Result<ZoneReport> {
    let c = parse_file(path)?;
    let (w, h) = (c.width as f32, c.height as f32);
    let mut report = ZoneReport {
        times: options.times.clone(),
        ..ZoneReport::default()
    };
    let mut zones = Vec::new();
    for &zone in &options.zones {
        if zone.is_platform() && !is_vertical(c.width, c.height) {
            report.notes.push(format!(
                "{} applies only to 9:16 compositions; this one is {}x{}",
                zone.name(),
                c.width,
                c.height
            ));
        } else {
            zones.push(zone);
        }
    }
    let scale = 0.25_f32.max(1.0 / c.width.min(c.height) as f32);
    let mut renderer = Renderer::new(&c, scale)?;
    let mut safe: Vec<Draft> = Vec::new();
    let mut band: Vec<Draft> = Vec::new();
    for &time in &options.times {
        for text in renderer.probe_text_at(time)? {
            let owner = owner_of(&c, text.element);
            if !allowed(&c, text.element, |l| l.allow_safe_zone) {
                let mut hits = Vec::new();
                for &zone in &zones {
                    let mut edges: Vec<&'static str> = Vec::new();
                    for (edge, blocked) in zone.blocked(w, h) {
                        if overlap(text.rect, blocked) && !edges.contains(&edge) {
                            edges.push(edge);
                        }
                    }
                    if !edges.is_empty() {
                        hits.push((zone.name(), edges));
                    }
                }
                if !hits.is_empty() {
                    merge(
                        &mut safe,
                        Draft { owner, time, rect: text.rect, text: snippet(&c, text.element), zones: hits },
                    );
                }
            }
            if let Some(zone) = options.caption {
                let region = [zone.rect[0] * w, zone.rect[1] * h, zone.rect[2] * w, zone.rect[3] * h];
                if overlap(text.rect, region) && !allowed(&c, text.element, |l| l.allow_caption_zone) {
                    merge(
                        &mut band,
                        Draft { owner, time, rect: text.rect, text: snippet(&c, text.element), zones: Vec::new() },
                    );
                }
            }
        }
    }
    let mut findings = Vec::new();
    for d in &safe {
        let platforms: Vec<&str> = d.zones.iter().map(|(name, _)| *name).collect();
        let mut edges: Vec<&str> = Vec::new();
        for (_, zone_edges) in &d.zones {
            for edge in zone_edges {
                if !edges.contains(edge) {
                    edges.push(edge);
                }
            }
        }
        findings.push(json!({
            "code": "outside_safe_zone",
            "severity": "warning",
            "element": label(&c, d.owner),
            "text": d.text,
            "platforms": platforms,
            "edges": edges,
            "box": box_json(d.rect),
            "time": (d.time * 1000.0).round() / 1000.0,
            "fix": "move the text inside the safe area, or add data-layout-allow-safe-zone to the text or an ancestor if it is intentional",
        }));
    }
    let severity = if options.caption.is_some_and(|z| z.error) { "error" } else { "warning" };
    for d in &band {
        findings.push(json!({
            "code": "caption_zone_collision",
            "severity": severity,
            "element": label(&c, d.owner),
            "text": d.text,
            "box": box_json(d.rect),
            "time": (d.time * 1000.0).round() / 1000.0,
            "fix": "move this text out of the caption band, or add data-layout-allow-caption-zone to the narrowest wrapper if it is deliberate",
        }));
    }
    report.caption_collisions = band.len();
    report.caption_errors = !band.is_empty() && severity == "error";
    if findings.len() > MAX_FINDINGS {
        report.truncated = findings.len() - MAX_FINDINGS;
        findings.truncate(MAX_FINDINGS);
    }
    report.findings = findings;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_frames_get_platform_zones_and_others_get_standard() {
        assert_eq!(default_zones(1080, 1920), Zone::PLATFORMS);
        assert_eq!(default_zones(720, 1280), Zone::PLATFORMS);
        assert_eq!(default_zones(1920, 1080), [Zone::Standard]);
    }

    #[test]
    fn shorts_zone_matches_the_documented_pixels() {
        let blocked = Zone::Shorts.blocked(1080.0, 1920.0);
        let edge = |name: &str| blocked.iter().find(|(e, _)| *e == name).unwrap().1;
        assert_eq!(edge("left")[2], 48.0);
        assert_eq!(edge("right")[0], 1080.0 - 192.0);
        assert_eq!(edge("top")[3], 288.0);
        assert_eq!(edge("bottom")[1], 1920.0 - 672.0);
    }

    #[test]
    fn reels_zone_uses_fractions() {
        let blocked = Zone::Reels.blocked(1000.0, 2000.0);
        let bottom = blocked.iter().find(|(e, _)| *e == "bottom").unwrap().1;
        assert_eq!(bottom[1], 2000.0 * 0.65);
    }

    #[test]
    fn overlap_ignores_touching_boxes() {
        assert!(!overlap([0.0, 0.0, 10.0, 10.0], [10.0, 0.0, 20.0, 10.0]));
        assert!(overlap([0.0, 0.0, 10.0, 10.0], [8.0, 0.0, 20.0, 10.0]));
    }

    #[test]
    fn parses_caption_zones() {
        let zone = parse_caption_zone("x0=0;y0=0.7;x1=1;y1=1;severity=error").unwrap();
        assert_eq!(zone.rect, [0.0, 0.7, 1.0, 1.0]);
        assert!(zone.error);
        assert!(parse_caption_zone("x0=0;y0=0.7;x1=1").is_err());
        assert!(parse_caption_zone("x0=0.5;y0=0;x1=0.5;y1=1").is_err());
        assert!(parse_caption_zone("x0=0;y0=0;x1=1;y1=1;seek=2").unwrap_err().to_string().contains("seek"));
    }

    #[test]
    fn samples_are_midpoints_or_explicit() {
        assert_eq!(sample_times(10.0, 2, &[]).unwrap(), [2.5, 7.5]);
        assert_eq!(sample_times(10.0, 9, &[1.0]).unwrap(), [1.0]);
        assert!(sample_times(10.0, 9, &[10.0]).is_err());
    }
}
