//! Zoom-aware SVG reuse (`render/paint/svg_levels.rs`): frames of vector artwork under an animated
//! camera zoom are resampled from shared ladder rasters. These tests pin that the result does not
//! depend on render order or cache state, that frames at rest still use the exact raster, that a
//! resampled frame stays close to the exact one without shifting, and that a fit beyond the
//! per-image cache limit renders its visible part instead of failing.

mod common;

use aether_motion::{parse::parse_file, render::Renderer};

/// Transparent line art: hairlines, a translucent masked disc, and a soft shadowed square.
const ART: &str = r##"<svg viewBox="0 0 240 140" style="left:40px;top:20px;width:240px;height:140px">
  <defs>
    <mask id="m"><rect width="240" height="140" fill="#fff"/><circle cx="170" cy="70" r="14" fill="#000"/></mask>
    <filter id="s"><feGaussianBlur stdDeviation="1.5"/></filter>
  </defs>
  <path d="M4 136 L236 4 M4 4 L236 136" stroke="#e8f0ff" stroke-width="0.7" fill="none"/>
  <path d="M20 20 H220 V120 H20 Z" stroke="#ffcc33" stroke-width="1.2" fill="none"/>
  <circle cx="170" cy="70" r="40" fill="#3388ff" fill-opacity="0.6" mask="url(#m)"/>
  <rect x="44" y="50" width="40" height="40" fill="#ff3366" filter="url(#s)"/>
</svg>"##;

/// A 320x180 scene whose camera runs `camera` over `ART` for two seconds.
fn scene(name: &str, camera: &str) -> std::path::PathBuf {
    let path = common::scratch(name).join("scene.html");
    std::fs::write(
        &path,
        format!(
            r#"<main data-composition-id="svg-zoom" data-width="320" data-height="180" data-duration="2" style="background:#101418">
  <div id="cam" class="clip" data-start="0" data-duration="2" data-track-index="0"
       style="left:0;top:0;width:320px;height:180px" data-hf-camera='{camera}'>{ART}</div>
</main>"#
        ),
    )
    .unwrap();
    path
}

/// Zoom 1 to 2 over the first second (linear), then held.
const ZOOM: &str = r#"[{"at":0,"zoom":1},{"at":1,"zoom":2,"x":30,"y":-10}]"#;

fn render(path: &std::path::Path, times: &[f64]) -> Vec<Vec<u8>> {
    let composition = parse_file(path).unwrap();
    let mut renderer = Renderer::new(&composition, 1.0).unwrap();
    times
        .iter()
        .map(|&t| renderer.render_at(t).unwrap().to_vec())
        .collect()
}

#[test]
fn zooming_frames_do_not_depend_on_render_order() {
    let path = scene("order", ZOOM);
    let times: Vec<f64> = (0..12).map(|i| f64::from(i) / 12.0).collect();
    let forward = render(&path, &times);
    let reversed: Vec<f64> = times.iter().rev().copied().collect();
    let mut backward = render(&path, &reversed);
    backward.reverse();
    for (i, t) in times.iter().enumerate() {
        assert!(forward[i] == backward[i], "reverse order differs at t={t}");
        let alone = render(&path, &[*t]).remove(0);
        assert!(forward[i] == alone, "a cold single frame differs at t={t}");
    }
}

#[test]
fn frames_at_rest_use_the_exact_raster() {
    // Once the zoom holds, the frame equals a camera that never zooms.
    let moving = render(&scene("rest-moving", ZOOM), &[1.0, 1.5]);
    let fixed = render(
        &scene("rest-fixed", r#"[{"at":0,"zoom":2,"x":30,"y":-10}]"#),
        &[1.0],
    );
    assert!(moving[0] == fixed[0], "the frame where the zoom lands");
    assert!(moving[1] == fixed[0], "a held frame");
}

/// Mean and maximum absolute channel difference, and the luminance-weighted centroid shift (px).
fn compare(a: &[u8], b: &[u8], width: usize) -> (f64, u8, f64) {
    let (mut sum, mut max) = (0_u64, 0_u8);
    let mut moments = [[0.0_f64; 3]; 2];
    for (i, (p, q)) in a
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .enumerate()
    {
        for c in 0..3 {
            let d = p[c].abs_diff(q[c]);
            sum += u64::from(d);
            max = max.max(d);
        }
        let (x, y) = ((i % width) as f64, (i / width) as f64);
        for (m, px) in moments.iter_mut().zip([p, q]) {
            let l = f64::from(px[0]) + f64::from(px[1]) + f64::from(px[2]);
            m[0] += l * x;
            m[1] += l * y;
            m[2] += l;
        }
    }
    let [m, n] = moments;
    let shift = (m[0] / m[2] - n[0] / n[2]).hypot(m[1] / m[2] - n[1] / n[2]);
    (sum as f64 / (a.len() / 4 * 3) as f64, max, shift)
}

#[test]
fn resampled_frames_stay_close_to_the_exact_raster() {
    // Linear zoom: at t = 0.5 the camera is at 1.5x, between ladder levels. Classic cameras
    // interpolate the translation `-zoom * focus` (-30, 10 here), so the focus is (20, -20/3).
    let moving = render(&scene("close-moving", ZOOM), &[0.5]).remove(0);
    let exact = render(
        &scene(
            "close-exact",
            r#"[{"at":0,"zoom":1.5,"x":20,"y":-6.666666666666667}]"#,
        ),
        &[0.0],
    )
    .remove(0);
    assert!(moving != exact, "the zooming frame went through the ladder");
    let (mean, max, shift) = compare(&moving, &exact, 320);
    // This art is a worst case: 0.7-unit hairlines are about one pixel wide at 1.5x, so the
    // remaining difference is antialiasing at their edges (measured: mean 1.34, max 74, 0.17 px).
    assert!(mean < 2.0, "mean difference {mean}");
    assert!(max <= 96, "max difference {max}");
    assert!(shift < 0.25, "centroid shift {shift} px");
}

#[test]
fn contain_and_cover_fits_stay_exact_while_zooming() {
    // A level's rounded aspect ratio would misplace letterboxed or cropped artwork, so these fits
    // are rasterized exactly on zooming frames too.
    let dir = common::scratch("contain");
    let art = ART.replace(
        r#" style="left:40px;top:20px;width:240px;height:140px""#,
        r#" xmlns="http://www.w3.org/2000/svg" width="240" height="140""#,
    );
    std::fs::write(dir.join("art.svg"), art).unwrap();
    for fit in ["contain", "cover"] {
        let scene = |name: &str, camera: &str| {
            let path = dir.join(format!("{name}-{fit}.html"));
            std::fs::write(
                &path,
                format!(
                    r#"<main data-composition-id="fit" data-width="320" data-height="180" data-duration="2" style="background:#101418">
  <div class="clip" data-start="0" data-duration="2" data-track-index="0"
       style="left:0;top:0;width:320px;height:180px" data-hf-camera='{camera}'>
    <img src="art.svg" style="position:absolute;left:30px;top:25px;width:250px;height:121px;object-fit:{fit}"></div>
</main>"#
                ),
            )
            .unwrap();
            path
        };
        let moving = render(&scene("moving", ZOOM), &[0.5]).remove(0);
        let exact = render(
            &scene(
                "exact",
                r#"[{"at":0,"zoom":1.5,"x":20,"y":-6.666666666666667}]"#,
            ),
            &[0.0],
        )
        .remove(0);
        assert!(moving == exact, "{fit}");
    }
}

#[test]
fn a_vector_fit_beyond_the_cache_limit_renders_its_visible_part() {
    // 4200 x 4200 px is 67 MiB as RGBA: more than one cached image may hold.
    let path = common::scratch("oversized").join("scene.html");
    std::fs::write(
        &path,
        r##"<main data-composition-id="big" data-width="64" data-height="64" data-duration="1" style="background:#000">
  <svg class="clip" data-start="0" data-duration="1" data-track-index="0" viewBox="0 0 100 100"
       style="left:-2068px;top:-2068px;width:4200px;height:4200px">
    <rect width="100" height="100" fill="#204080"/><circle cx="50" cy="50" r="0.5" fill="#ffffff"/>
  </svg>
</main>"##,
    )
    .unwrap();
    let frame = render(&path, &[0.0]).remove(0);
    let at = |x: usize, y: usize| &frame[(y * 64 + x) * 4..][..3];
    // The circle (21 px radius at this size) is centered on the frame; the corners show the plate.
    assert_eq!(at(32, 32), [255, 255, 255]);
    assert_eq!(at(0, 0), [0x20, 0x40, 0x80]);
    assert_eq!(at(63, 63), [0x20, 0x40, 0x80]);
}
