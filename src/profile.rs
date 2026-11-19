//! Opt-in phase profile: with `AETHER_PROFILE=1` in the environment, renders and exports add up the
//! thread time spent in each [`Phase`] and print a summary on stderr when they finish.
//!
//! Phases are summed across worker threads, so with several threads their total exceeds the wall
//! time. `Raster` covers drawing a frame; `Bloom`, `LayerBlur`, and `EffectLayer` are parts of it
//! (`EffectLayer` is every element drawn through an offscreen effect layer, bloom included). When
//! the variable is not set, a phase costs one relaxed atomic load; profiling never changes output.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// A timed part of a render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

const PHASES: [Phase; 6] = [
    Phase::Raster,
    Phase::Convert,
    Phase::Encode,
    Phase::Bloom,
    Phase::LayerBlur,
    Phase::EffectLayer,
];

static NANOS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static CALLS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

impl Phase {
    fn index(self) -> usize {
        PHASES.iter().position(|&p| p == self).unwrap()
    }

    fn name(self) -> &'static str {
        match self {
            Self::Raster => "raster",
            Self::Convert => "convert",
            Self::Encode => "encode",
            Self::Bloom => "bloom",
            Self::LayerBlur => "layer-blur",
            Self::EffectLayer => "effect-layer",
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
    out
}

/// Prints [`summary`] on stderr when profiling is enabled.
pub fn report() {
    if enabled() {
        eprintln!("{}", summary());
    }
}
