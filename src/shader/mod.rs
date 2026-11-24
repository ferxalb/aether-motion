//! Procedural shader layers (`data-hf-shader`), independent of HTML and pixels.
//!
//! A shader is a small expression program evaluated at every pixel of an element's padding box:
//! `syntax.rs` parses it, `compile.rs` type-checks it and lowers it to scalar instructions, and
//! `vm.rs` runs them over chunks of grid points with vectorizable loops. `math.rs` and `noise.rs`
//! hold the deterministic functions it calls, so a pixel is a pure function of the program, the
//! frame values, and its position on every machine. `Shader` adds the declared uniforms and their
//! keyframes, and the frame values one instant gives (`t`, the box size, and the uniforms).
//! `parse/shader.rs` reads the attributes; `render/paint/shader.rs` places the grid, resamples it,
//! and composites it. See `docs/shaders.md`.

mod compile;
mod math;
mod noise;
mod syntax;
mod vm;

pub use vm::Grid;

use crate::model::Track;
use anyhow::{bail, Result};

/// Most uniforms a shader can declare.
pub const MAX_UNIFORMS: usize = 32;

/// A declared uniform: its name and resting components (1 to 4), each with an optional track.
#[derive(Clone, Debug)]
pub struct Uniform {
    pub name: String,
    pub value: Vec<f32>,
    /// One track per component (empty tracks hold `value`); filled from the keyframes.
    pub tracks: Vec<Track>,
}

/// A compiled shader with its uniforms and render options.
#[derive(Clone, Debug)]
pub struct Shader {
    code: vm::Code,
    uniforms: Vec<Uniform>,
    reads_time: bool,
    /// Evaluation resolution relative to the output (1, 0.5, or 0.25); lower values evaluate
    /// fewer points and interpolate between them.
    pub resolution: f32,
    /// Ordered dither of half an 8-bit level, which hides banding in smooth fields.
    pub dither: bool,
}

/// The scratch registers and output buffer of `Shader::run`, reused across frames.
#[derive(Default)]
pub struct Scratch {
    registers: Vec<[f32; vm::CHUNK]>,
}

impl Shader {
    /// Compiles `source` with the given uniforms (`value` gives each one's component count).
    pub fn compile(
        source: &str,
        uniforms: Vec<Uniform>,
        resolution: f32,
        dither: bool,
    ) -> Result<Self> {
        if uniforms.len() > MAX_UNIFORMS {
            bail!("at most {MAX_UNIFORMS} uniforms are allowed");
        }
        let mut slots = Vec::with_capacity(uniforms.len());
        let mut slot = compile::FRAME_VALUES.len();
        for uniform in &uniforms {
            if compile::is_reserved(&uniform.name) {
                bail!("uniform '{}' uses a built-in name", uniform.name);
            }
            if !valid_name(&uniform.name) {
                bail!(
                    "uniform name '{}' must start with a letter or '_' and contain only letters, digits, and '_'",
                    uniform.name
                );
            }
            if !(1..=4).contains(&uniform.value.len()) {
                bail!("uniform '{}' must have 1 to 4 components", uniform.name);
            }
            slots.push(compile::UniformSlot {
                name: uniform.name.clone(),
                components: uniform.value.len(),
                slot,
            });
            slot += uniform.value.len();
        }
        let program = syntax::parse(source)?;
        let (code, reads_time) = compile::compile(&program, &slots)?;
        Ok(Self {
            code,
            uniforms,
            reads_time,
            resolution,
            dither,
        })
    }

    pub fn uniforms(&self) -> &[Uniform] {
        &self.uniforms
    }

    pub fn uniforms_mut(&mut self) -> &mut [Uniform] {
        &mut self.uniforms
    }

    /// Whether the image can change over time: the program reads `t` or a uniform has keyframes.
    pub fn is_animated(&self) -> bool {
        self.reads_time
            || self
                .uniforms
                .iter()
                .any(|u| u.tracks.iter().any(|track| !track.0.is_empty()))
    }

    /// Number of scalar instructions run per grid point.
    pub fn instruction_count(&self) -> usize {
        self.code.instrs.len()
    }

    /// The frame values at `elapsed` seconds into the clip for a padding box of `size` CSS px:
    /// `t`, the size, the aspect ratio, then every uniform component.
    pub fn values(&self, elapsed: f64, size: [f32; 2]) -> Vec<f32> {
        let aspect = if size[1] > 0.0 {
            size[0] / size[1]
        } else {
            1.0
        };
        let mut values = vec![elapsed as f32, size[0], size[1], aspect];
        for uniform in &self.uniforms {
            for (i, &base) in uniform.value.iter().enumerate() {
                let track = uniform.tracks.get(i);
                values.push(track.map_or(base, |track| track.eval(elapsed, base, true, true)));
            }
        }
        values
    }

    /// A key that changes whenever the image can: 0 for a still shader, otherwise a hash of
    /// the frame values that vary (the size is part of the element's own frame state).
    pub fn key(&self, elapsed: f64) -> u64 {
        if !self.is_animated() {
            return 0;
        }
        self.values(elapsed, [0.0, 0.0])
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |key, value| {
                (key ^ u64::from(value.to_bits())).wrapping_mul(0x0100_0000_01b3)
            })
            | 1
    }

    /// Evaluates the shader over `grid` into `out` (premultiplied RGBA in `[0, 1]`, four values
    /// per point). `avx2` selects the AVX2 build of the loops (identical values).
    pub fn run(
        &self,
        values: &[f32],
        grid: &Grid,
        out: &mut Vec<f32>,
        scratch: &mut Scratch,
        avx2: bool,
    ) {
        self.code
            .run(values, grid, out, &mut scratch.registers, avx2)
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(width: usize, height: usize) -> Grid {
        Grid {
            width,
            height,
            origin: [0.5 / width as f32, 0.5 / height as f32],
            step: [1.0 / width as f32, 1.0 / height as f32],
            size: [width as f32, height as f32],
        }
    }

    fn render(shader: &Shader, width: usize, height: usize, elapsed: f64, avx2: bool) -> Vec<f32> {
        let mut out = Vec::new();
        let values = shader.values(elapsed, [width as f32, height as f32]);
        shader.run(
            &values,
            &grid(width, height),
            &mut out,
            &mut Scratch::default(),
            avx2,
        );
        out
    }

    #[test]
    fn evaluates_a_gradient() {
        let shader = Shader::compile("rgb(uv.x, uv.y, 0.25)", Vec::new(), 1.0, false).unwrap();
        let out = render(&shader, 130, 3, 0.0, false);
        assert_eq!(out.len(), 130 * 3 * 4);
        // Point (129, 2): uv = (129.5 / 130, 2.5 / 3).
        let last = &out[(2 * 130 + 129) * 4..];
        assert_eq!(last[0], 0.5 / 130.0 + 129.0 * (1.0 / 130.0));
        assert_eq!(last[2], 0.25);
        assert_eq!(last[3], 1.0);
    }

    #[test]
    fn premultiplies_and_clamps() {
        let shader = Shader::compile("rgba(2, 0.5, -1, 0.5)", Vec::new(), 1.0, false).unwrap();
        let out = render(&shader, 1, 1, 0.0, false);
        assert_eq!(out, vec![0.5, 0.25, 0.0, 0.5]);
    }

    #[test]
    fn simd_build_matches_the_baseline() {
        let source = "let p = rotate(uv - 0.5, t) * 6;
            let n = fbm(vec3(p, t * 0.2), 6);
            let c = palette(n, #808080, #808080, vec3(1), vec3(0, 0.33, 0.67));
            let v = voronoi(p * 1.5) + ridge(p, 3) * 0.2 + hash(floor(px / 8)) * 0.1;
            rgba(c * smoothstep(0.1, 0.9, v) + grain(0.04), 0.9)";
        let shader = Shader::compile(source, Vec::new(), 1.0, true).unwrap();
        let scalar = render(&shader, 97, 23, 1.37, false);
        let wide = render(
            &shader,
            97,
            23,
            1.37,
            cfg!(target_arch = "x86_64") && std::arch::is_x86_feature_detected!("avx2"),
        );
        assert!(scalar
            .iter()
            .zip(&wide)
            .all(|(a, b)| a.to_bits() == b.to_bits()));
        assert!(scalar.iter().any(|&v| v > 0.1 && v < 0.8));
    }

    #[test]
    fn uniforms_and_keyframes_drive_values() {
        let mut uniforms = vec![Uniform {
            name: "glow".into(),
            value: vec![0.25],
            tracks: Vec::new(),
        }];
        uniforms[0].tracks = vec![Track(vec![
            crate::model::TrackPoint {
                at: 0.0,
                value: 0.0,
                easing: Default::default(),
            },
            crate::model::TrackPoint {
                at: 2.0,
                value: 1.0,
                easing: Default::default(),
            },
        ])];
        let shader = Shader::compile("vec3(glow)", uniforms, 1.0, false).unwrap();
        assert!(shader.is_animated());
        assert_eq!(render(&shader, 1, 1, 1.0, false)[0], 0.5);
        assert_ne!(shader.key(0.5), shader.key(1.0));
        let still = Shader::compile("vec3(uv.x)", Vec::new(), 1.0, false).unwrap();
        assert!(!still.is_animated());
        assert_eq!(still.key(3.0), 0);
        assert!(Shader::compile(
            "1",
            vec![Uniform {
                name: "noise".into(),
                value: vec![1.0],
                tracks: Vec::new()
            }],
            1.0,
            false
        )
        .is_err());
    }
}
