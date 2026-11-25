//! Opt-in phase profile: with `AETHER_PROFILE=1` in the environment, renders and exports add up the
//! thread time spent in each [`Phase`] and print a summary on stderr when they finish.
//!
//! Phases are summed across worker threads, so with several threads their total exceeds the wall
//! time. `Raster` covers drawing a frame; `Bloom`, `LayerBlur`, `EffectLayer`, `PlaneLayer`,
//! `PlaneWarp`, `Scene3d`, `ShaderEval`, `ShaderBlend`, and `TexKernel` are parts of it
//! (`EffectLayer` is every element drawn through an offscreen effect layer, bloom included). When
//! the variable is not set, a phase costs one relaxed atomic load; profiling never changes output.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// A timed part of a render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Phase {
    /// Drawing frames (everything the renderer does for a frame).
    Raster,
    /// RGBA to YUV 4:2:0 conversion for the encoder.
    Convert,
    /// H.264 encoding of converted frames.
    Encode,
    /// `data-hf-bloom` halos (part of `Raster`).
    Bloom,
    /// `data-hf-motion-blur` sampling and averaging (part of `Raster`).
    LayerBlur,
    /// Elements drawn through an offscreen effect layer (part of `Raster`).
    EffectLayer,
    SvgParse,
    SvgRaster,
    ImageFit,
    ImageBlit,
    ImagePack,
    /// Resampling zoom-ladder levels of vector artwork to a frame's fitted size (`svg_levels.rs`).
    ImageResample,
    Finish,
    /// Projected planes of 2.5D spaces drawn on their offscreen layer (part of `Raster`).
    PlaneLayer,
    /// Projected planes warped onto the frame (part of `Raster`).
    PlaneWarp,
    /// `data-hf-3d` scenes: evaluation, rasterization (CPU or GPU), and compositing (part of
    /// `Raster`).
    Scene3d,
    /// `data-hf-shader` evaluation (part of `Raster`).
    ShaderEval,
    /// `data-hf-shader` resampling and compositing (part of `Raster`).
    ShaderBlend,
    /// Batched texture reads of textured 3D meshes in `cpp-kernel` builds (part of `Scene3d`).
    TexKernel,
}

const PHASES: [Phase; 19] = [
    Phase::Raster,
    Phase::Convert,
    Phase::Encode,
    Phase::Bloom,
    Phase::LayerBlur,
    Phase::EffectLayer,
    Phase::SvgParse,
    Phase::SvgRaster,
    Phase::ImageFit,
    Phase::ImageBlit,
    Phase::ImagePack,
    Phase::ImageResample,
    Phase::Finish,
    Phase::PlaneLayer,
    Phase::PlaneWarp,
    Phase::Scene3d,
    Phase::ShaderEval,
    Phase::ShaderBlend,
    Phase::TexKernel,
];

static NANOS: [AtomicU64; PHASES.len()] = [const { AtomicU64::new(0) }; PHASES.len()];
static CALLS: [AtomicU64; PHASES.len()] = [const { AtomicU64::new(0) }; PHASES.len()];

impl Phase {
    fn index(self) -> usize {
        self as usize
    }

    fn name(self) -> &'static str {
        match self {
            Self::Raster => "raster",
            Self::Convert => "convert",
            Self::Encode => "encode",
            Self::Bloom => "bloom",
            Self::LayerBlur => "layer-blur",
            Self::EffectLayer => "effect-layer",
            Self::SvgParse => "svg-parse",
            Self::SvgRaster => "svg-raster",
            Self::ImageFit => "image-fit",
            Self::ImageBlit => "image-blit",
            Self::ImagePack => "image-pack",
            Self::ImageResample => "image-resample",
            Self::Finish => "finish",
            Self::PlaneLayer => "plane-layer",
            Self::PlaneWarp => "plane-warp",
            Self::Scene3d => "scene-3d",
            Self::ShaderEval => "shader-eval",
            Self::ShaderBlend => "shader-blend",
            Self::TexKernel => "tex-kernel",
        }
    }
}

/// Whether `AETHER_PROFILE` is set to a value other than `0` or empty.
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var("AETHER_PROFILE").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Runs `f`, adding its duration to `phase` when profiling is enabled.
#[inline]
pub fn time<T>(phase: Phase, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let start = Instant::now();
    let out = f();
    let i = phase.index();
    NANOS[i].fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    CALLS[i].fetch_add(1, Ordering::Relaxed);
    out
}

/// Adds `elapsed` to `phase` (for work timed by hand around code that borrows mutably).
pub fn add(phase: Phase, elapsed: std::time::Duration) {
    let i = phase.index();
    NANOS[i].fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    CALLS[i].fetch_add(1, Ordering::Relaxed);
}

/// Fitted image residency and raster sizes, recorded only in profiling runs.
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum ImageMetric {
    Hit,
    Miss,
    Eviction,
    Packed,
    PeakCacheBytes,
    LargestRasterBytes,
    RasterBytes,
    /// Vector fits too large to cache, rasterized for their visible part only.
    Visible,
}

const IMAGE_METRICS: [(&str, bool); 8] = [
    ("hits", false),
    ("misses", false),
    ("evictions", false),
    ("packed", false),
    ("peak worker fitted bytes", true),
    ("largest fitted raster bytes", true),
    ("total fitted raster bytes", false),
    ("visible-region rasters", false),
];
static IMAGE_VALUES: [AtomicU64; IMAGE_METRICS.len()] =
    [const { AtomicU64::new(0) }; IMAGE_METRICS.len()];

#[inline]
pub(crate) fn image(metric: ImageMetric, value: usize) {
    if enabled() {
        let i = metric as usize;
        if IMAGE_METRICS[i].1 {
            IMAGE_VALUES[i].fetch_max(value as u64, Ordering::Relaxed);
        } else {
            IMAGE_VALUES[i].fetch_add(value as u64, Ordering::Relaxed);
        }
    }
}

/// The summary printed by [`report`]: one line per phase that ran.
pub fn summary() -> String {
    let mut out = String::from("profile (thread seconds, summed across workers):");
    for phase in PHASES {
        let i = phase.index();
        let calls = CALLS[i].load(Ordering::Relaxed);
        if calls == 0 {
            continue;
        }
        let seconds = NANOS[i].load(Ordering::Relaxed) as f64 / 1e9;
        out.push_str(&format!(
            "\n  {:<13} {:>9.3} s  {:>7} calls  {:>8.2} ms/call",
            phase.name(),
            seconds,
            calls,
            seconds * 1e3 / calls as f64
        ));
    }
    if IMAGE_VALUES[ImageMetric::Miss as usize].load(Ordering::Relaxed) != 0 {
        out.push_str("\n  fitted image cache:");
        for ((name, _), value) in IMAGE_METRICS.iter().zip(&IMAGE_VALUES) {
            out.push_str(&format!(" {name}={};", value.load(Ordering::Relaxed)));
        }
    }
    out
}

/// Prints [`summary`] on stderr when profiling is enabled.
pub fn report() {
    if enabled() {
        eprintln!("{}", summary());
    }
}
