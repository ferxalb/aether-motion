//! Opt-in batched texture sampling for textured meshes (Cargo feature `cpp-kernel`).
//!
//! The CPU rasterizer normally shades each covered sample on its own ([`sample::shade`]). With
//! this feature, the samples of one triangle within one tile that pass the depth test are
//! collected into a [`Batch`] first; [`flush`] then reads each texture of the look for all of
//! them in one call, and finishes every sample with [`sample::finish`]. Each triangle covers a
//! sample once, so deferring its depth writes to the end of the triangle changes nothing.
//!
//! The batched read is one of these [`Mode`]s, chosen once per process (`AETHER_TEX_KERNEL`
//! overrides the default, for measurements):
//! - `off`: no batching; the per-sample Rust path of default builds.
//! - `rust`, `rust-avx2`: the batched algorithm in Rust (per-texture constants hoisted,
//!   power-of-two wrapping as a mask), compiled for the baseline ISA or with AVX2 enabled.
//! - `cpp-scalar`, `cpp-avx2`, `cpp-avx512`: the C++ kernels of `native/texture_kernel.cpp`.
//!
//! The default is `cpp-avx512` on CPUs with AVX-512F, else `cpp-avx2` with AVX2, else `off`.
//! Every mode returns exactly what [`Texture::sample`] returns (see the tests), so the mode
//! never changes a pixel. With `AETHER_PROFILE=1`, the texture reads are timed as `tex-kernel`.

use super::sample::{self, Planes, Point, Slot, Surfaces};
use super::texture::srgb_lut;
use super::{Texture, Wrap};
use crate::profile::{self, Phase};
use crate::scene3d::raster::COVERED;
use std::sync::OnceLock;

/// How textures of batched samples are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Rust,
    RustAvx2,
    CppScalar,
    CppAvx2,
    CppAvx512,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Rust => "rust",
            Self::RustAvx2 => "rust-avx2",
            Self::CppScalar => "cpp-scalar",
            Self::CppAvx2 => "cpp-avx2",
            Self::CppAvx512 => "cpp-avx512",
        }
    }

    /// Whether this CPU can run the mode.
    fn supported(self) -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            match self {
                Self::RustAvx2 | Self::CppAvx2 => std::arch::is_x86_feature_detected!("avx2"),
                Self::CppAvx512 => {
                    std::arch::is_x86_feature_detected!("avx2")
                        && std::arch::is_x86_feature_detected!("avx512f")
                }
                _ => true,
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            !matches!(self, Self::RustAvx2 | Self::CppAvx2 | Self::CppAvx512)
        }
    }
}

const MODES: [Mode; 6] = [
    Mode::Off,
    Mode::Rust,
    Mode::RustAvx2,
    Mode::CppScalar,
    Mode::CppAvx2,
    Mode::CppAvx512,
];

/// The mode of this process.
pub fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let default = [Mode::CppAvx512, Mode::CppAvx2]
            .into_iter()
            .find(|m| m.supported())
            .unwrap_or(Mode::Off);
        let Ok(name) = std::env::var("AETHER_TEX_KERNEL") else {
            return default;
        };
        match MODES.into_iter().find(|m| m.name() == name) {
            Some(m) if m.supported() => m,
            _ => {
                eprintln!(
                    "AETHER_TEX_KERNEL={name} is unknown or unsupported on this CPU; using {}",
                    default.name()
                );
                default
            }
        }
    })
}

// ---------------------------------------------------------------------------------------------
// C ABI (mirrors `native/texture_kernel.cpp`).

#[repr(C)]
struct AmLevel {
    rgba: *const u8,
    width: i32,
    height: i32,
}

#[repr(C)]
struct AmTexture {
    levels: *const AmLevel,
    level_count: u32,
    encoding: u32,
    wrap: u32,
    alpha: u32,
    srgb: *const f64,
}

const _: () = assert!(std::mem::size_of::<AmLevel>() == 16);
const _: () = assert!(std::mem::size_of::<AmTexture>() == 32);

type Kernel = unsafe extern "C" fn(
    texture: *const AmTexture,
    u: *const f64,
    v: *const f64,
    lod: *const f64,
    n: usize,
    out: *mut f64,
);

extern "C" {
    fn aether_tex_sample_scalar(
        texture: *const AmTexture,
        u: *const f64,
        v: *const f64,
        lod: *const f64,
        n: usize,
        out: *mut f64,
    );
    #[cfg(target_arch = "x86_64")]
    fn aether_tex_sample_avx2(
        texture: *const AmTexture,
        u: *const f64,
        v: *const f64,
        lod: *const f64,
        n: usize,
        out: *mut f64,
    );
    #[cfg(target_arch = "x86_64")]
    fn aether_tex_sample_avx512(
        texture: *const AmTexture,
        u: *const f64,
        v: *const f64,
        lod: *const f64,
        n: usize,
        out: *mut f64,
    );
}

/// Most mip levels a kernel call describes (a 4096 px edge has 13).
const MAX_LEVELS: usize = 16;
/// Batches are padded to a multiple of this many samples (the widest SIMD group).
const LANES: usize = 8;

/// Samples `texture` at `n = u.len()` points into `out` (`4 * n`, channel-major) with `mode`.
/// `n` must be a multiple of [`LANES`]. Alpha is only guaranteed when `alpha` is set.
#[allow(clippy::too_many_arguments)]
fn read(
    mode: Mode,
    texture: &Texture,
    wrap: Wrap,
    alpha: bool,
    u: &[f64],
    v: &[f64],
    lod: &[f64],
    out: &mut [f64],
) {
    let n = u.len();
    assert!(n.is_multiple_of(LANES) && v.len() == n && lod.len() == n && out.len() == 4 * n);
    let kernel: Kernel = match mode {
        Mode::Off | Mode::Rust => return rust_read(texture, wrap, u, v, lod, out),
        #[cfg(target_arch = "x86_64")]
        Mode::RustAvx2 => {
            // SAFETY: the mode is only selected when the CPU has AVX2.
            return unsafe { rust_read_avx2(texture, wrap, u, v, lod, out) };
        }
        #[cfg(target_arch = "x86_64")]
        Mode::CppAvx2 => aether_tex_sample_avx2,
        #[cfg(target_arch = "x86_64")]
        Mode::CppAvx512 => aether_tex_sample_avx512,
        _ => aether_tex_sample_scalar,
    };
    let count = texture.levels();
    if count > MAX_LEVELS {
        return rust_read(texture, wrap, u, v, lod, out);
    }
    let mut levels: [AmLevel; MAX_LEVELS] = std::array::from_fn(|_| AmLevel {
        rgba: std::ptr::null(),
        width: 0,
        height: 0,
    });
    for (i, level) in levels.iter_mut().enumerate().take(count) {
        let (pixels, width, height) = texture.level_raw(i);
        *level = AmLevel {
            rgba: pixels.as_ptr().cast(),
            width: width as i32,
            height: height as i32,
        };
    }
    let raw = AmTexture {
        levels: levels.as_ptr(),
        level_count: count as u32,
        encoding: u32::from(!texture.is_srgb()),
        wrap: u32::from(wrap == Wrap::Clamp),
        alpha: u32::from(alpha),
        srgb: srgb_lut().as_ptr(),
    };
    // SAFETY: `raw` borrows the texture's levels (each `width * height` RGBA texels, at most
    // 4096 px per edge so the sizes fit `i32`) and the static sRGB table for the call; the
    // slices hold `n` and `4 * n` values as the ABI requires, and `n` is a multiple of 8.
    unsafe {
        kernel(
            &raw,
            u.as_ptr(),
            v.as_ptr(),
            lod.as_ptr(),
            n,
            out.as_mut_ptr(),
        )
    }
}

// ---------------------------------------------------------------------------------------------
// The same batched algorithm in Rust: per-texture constants hoisted out of the loop, and repeat
// wrapping of power-of-two levels as a mask. Arithmetic is that of `Texture::sample`.

#[inline(always)]
fn rust_read_generic(
    texture: &Texture,
    wrap: Wrap,
    u: &[f64],
    v: &[f64],
    lod: &[f64],
    out: &mut [f64],
) {
    let n = u.len();
    let srgb = texture.is_srgb();
    let table = srgb_lut();
    let count = texture.levels();
    let top = (count - 1) as f64;
    let texel = |level: usize, x: i64, y: i64| -> [f64; 4] {
        let (pixels, w, h) = texture.level_raw(level);
        let (w, h) = (w as i64, h as i64);
        let (x, y) = match wrap {
            Wrap::Repeat if w & (w - 1) == 0 && h & (h - 1) == 0 => (x & (w - 1), y & (h - 1)),
            Wrap::Repeat => (x.rem_euclid(w), y.rem_euclid(h)),
            Wrap::Clamp => (x.clamp(0, w - 1), y.clamp(0, h - 1)),
        };
        let p = pixels[(y * w + x) as usize];
        let alpha = f64::from(p[3]) / 255.0;
        if srgb {
            [
                table[p[0] as usize],
                table[p[1] as usize],
                table[p[2] as usize],
                alpha,
            ]
        } else {
            [
                f64::from(p[0]) / 255.0,
                f64::from(p[1]) / 255.0,
                f64::from(p[2]) / 255.0,
                alpha,
            ]
        }
    };
    let bilinear = |level: usize, uv: [f64; 2]| -> [f64; 4] {
        let (_, w, h) = texture.level_raw(level);
        let x = uv[0] * w as f64 - 0.5;
        let y = uv[1] * h as f64 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (ix, iy) = (x0 as i64, y0 as i64);
        let (ix1, iy1) = (ix.wrapping_add(1), iy.wrapping_add(1));
        let (p00, p10, p01, p11) = (
            texel(level, ix, iy),
            texel(level, ix1, iy),
            texel(level, ix, iy1),
            texel(level, ix1, iy1),
        );
        std::array::from_fn(|i| {
            let top = p00[i] + (p10[i] - p00[i]) * fx;
            let bottom = p01[i] + (p11[i] - p01[i]) * fx;
            top + (bottom - top) * fy
        })
    };
    for i in 0..n {
        let lod = if lod[i].is_finite() {
            lod[i].clamp(0.0, top)
        } else {
            0.0
        };
        let low = lod.floor();
        let fraction = lod - low;
        let uv = [u[i], v[i]];
        let a = bilinear(low as usize, uv);
        let s = if fraction <= 0.0 || low >= top {
            a
        } else {
            let b = bilinear(low as usize + 1, uv);
            std::array::from_fn(|c| a[c] + (b[c] - a[c]) * fraction)
        };
        for (c, value) in s.into_iter().enumerate() {
            out[c * n + i] = value;
        }
    }
}

fn rust_read(texture: &Texture, wrap: Wrap, u: &[f64], v: &[f64], lod: &[f64], out: &mut [f64]) {
    rust_read_generic(texture, wrap, u, v, lod, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn rust_read_avx2(
    texture: &Texture,
    wrap: Wrap,
    u: &[f64],
    v: &[f64],
    lod: &[f64],
    out: &mut [f64],
) {
    rust_read_generic(texture, wrap, u, v, lod, out);
}

// ---------------------------------------------------------------------------------------------
// Batching in the rasterizer.

/// Samples of one triangle within one tile, collected by the rasterizer, and the buffers the
/// texture reads use. Reused across triangles and tiles; it never shrinks.
#[derive(Default)]
pub struct Batch {
    /// Index into the tile buffers, sample center, and depth of each collected sample.
    at: Vec<usize>,
    xy: Vec<[f64; 2]>,
    z: Vec<f32>,
    /// Points of the collected samples in front of the eye, and their index into `at`.
    points: Vec<Point>,
    owner: Vec<u32>,
    u: Vec<f64>,
    v: Vec<f64>,
    lod: Vec<f64>,
    out: [Vec<f64>; 3],
}

impl Batch {
    /// Whether samples of triangle `tri` should be collected instead of shaded one by one.
    pub fn wants(surfaces: &Surfaces, tri: u32) -> bool {
        if mode() == Mode::Off {
            return false;
        }
        let look = &surfaces.looks[surfaces.shaded.tris[tri as usize].look as usize];
        look.albedo.is_some() || look.normal_map.is_some() || look.emissive_map.is_some()
    }

    pub fn clear(&mut self) {
        self.at.clear();
        self.xy.clear();
        self.z.clear();
    }

    /// Adds a sample that passed the depth test.
    #[inline]
    pub fn push(&mut self, at: usize, x: f64, y: f64, z: f32) {
        self.at.push(at);
        self.xy.push([x, y]);
        self.z.push(z);
    }
}

/// Shades the collected samples of triangle `tri` and writes the drawn ones to the tile.
pub fn flush(
    batch: &mut Batch,
    surfaces: &Surfaces,
    tri: u32,
    planes: &Planes,
    color: &mut [u32],
    depth: &mut [f32],
) {
    if batch.at.is_empty() {
        return;
    }
    let mode = mode();
    let shaded_tri = &surfaces.shaded.tris[tri as usize];
    let look = &surfaces.looks[shaded_tri.look as usize];
    batch.points.clear();
    batch.owner.clear();
    for (k, &[x, y]) in batch.xy.iter().enumerate() {
        if let Some(p) = sample::point(surfaces, shaded_tri, planes, x, y) {
            batch.points.push(p);
            batch.owner.push(k as u32);
        }
    }
    let count = batch.points.len();
    if count == 0 {
        return;
    }
    let padded = count.div_ceil(LANES) * LANES;
    let slots = [
        (Slot::Albedo, &look.albedo, look.alpha_cutoff > 0.0),
        (Slot::Normal, &look.normal_map, false),
        (Slot::Emissive, &look.emissive_map, false),
    ];
    let start = profile::enabled().then(std::time::Instant::now);
    for (slot, texture, alpha) in slots {
        let Some(texture) = texture else { continue };
        batch.u.clear();
        batch.v.clear();
        batch.lod.clear();
        for p in &batch.points {
            batch.u.push(p.uv[0]);
            batch.v.push(p.uv[1]);
            batch.lod.push(p.lod(texture));
        }
        // Pad with copies of the last sample, so every SIMD group is full.
        for _ in count..padded {
            batch.u.push(batch.u[count - 1]);
            batch.v.push(batch.v[count - 1]);
            batch.lod.push(batch.lod[count - 1]);
        }
        let out = &mut batch.out[slot as usize];
        out.resize(4 * padded, 0.0);
        read(
            mode, texture, look.wrap, alpha, &batch.u, &batch.v, &batch.lod, out,
        );
    }
    if let Some(start) = start {
        profile::add(Phase::TexKernel, start.elapsed());
    }
    let out = &batch.out;
    for (j, p) in batch.points.iter().enumerate() {
        let sampled = |slot: Slot, _: &Texture| {
            let o = &out[slot as usize];
            [o[j], o[padded + j], o[2 * padded + j], o[3 * padded + j]]
        };
        if let Some(rgb) = sample::finish(surfaces, shaded_tri, p, sampled) {
            let k = batch.owner[j] as usize;
            let at = batch.at[k];
            depth[at] = batch.z[k];
            color[at] = rgb | COVERED;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d::look::texture::Encoding;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    fn texture(rng: &mut Rng, w: usize, h: usize, encoding: Encoding) -> Texture {
        let pixels = (0..w * h)
            .map(|_| {
                let b = rng.next().to_le_bytes();
                [b[0], b[1], b[2], b[3]]
            })
            .collect();
        Texture::from_pixels(w, h, pixels, encoding)
    }

    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    /// Every mode the CPU supports returns `Texture::sample` bit for bit.
    #[test]
    fn every_mode_matches_texture_sample() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        // UV specials stay where `Texture::sample` is defined in debug builds (its texel
        // `x + 1` must not overflow `i64`); LOD specials cover every class of value.
        let uv_special = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            1e-300,
            1e9,
            -1e9,
            1e15,
            -1e15,
            -1e300,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        let lod_special = [
            0.0,
            -0.0,
            0.5,
            -3.0,
            1e300,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        let mut checked = 0;
        for (w, h) in [(256, 256), (64, 32), (7, 5), (1, 1), (300, 4)] {
            for encoding in [Encoding::Srgb, Encoding::Linear] {
                let texture = texture(&mut rng, w, h, encoding);
                for wrap in [Wrap::Repeat, Wrap::Clamp] {
                    let n = 4096;
                    let mut u = Vec::with_capacity(n);
                    let mut v = Vec::with_capacity(n);
                    let mut lod = Vec::with_capacity(n);
                    for i in 0..n {
                        // Runs of nearby samples (like a span), with occasional special values.
                        let base = (i / 16) as f64 * 0.37;
                        let pick = |rng: &mut Rng, scale: f64, special: &[f64]| {
                            if rng.next().is_multiple_of(23) {
                                special[(rng.next() % special.len() as u64) as usize]
                            } else {
                                (base + rng.unit() * 0.05) * scale - 0.5 * scale
                            }
                        };
                        u.push(pick(&mut rng, 6.0, &uv_special));
                        v.push(pick(&mut rng, 6.0, &uv_special));
                        lod.push(if rng.next().is_multiple_of(5) {
                            pick(&mut rng, 20.0, &lod_special)
                        } else {
                            (i / 64) as f64 * 0.31 % 10.0 + rng.unit() * 0.01
                        });
                    }
                    let expected: Vec<[f64; 4]> = (0..n)
                        .map(|i| texture.sample([u[i], v[i]], lod[i], wrap))
                        .collect();
                    for mode in MODES.into_iter().filter(|m| m.supported()) {
                        for alpha in [true, false] {
                            let mut out = vec![0.0; 4 * n];
                            read(mode, &texture, wrap, alpha, &u, &v, &lod, &mut out);
                            for (i, e) in expected.iter().enumerate() {
                                let channels = if alpha { 4 } else { 3 };
                                for c in 0..channels {
                                    let got = out[c * n + i];
                                    assert!(
                                        same(got, e[c]),
                                        "{mode:?} {w}x{h} {encoding:?} {wrap:?} sample {i} \
                                         uv ({}, {}) lod {} channel {c}: {got} != {}",
                                        u[i],
                                        v[i],
                                        lod[i],
                                        e[c]
                                    );
                                }
                            }
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked >= 40);
    }
}
