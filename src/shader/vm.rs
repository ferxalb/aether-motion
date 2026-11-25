//! The shader virtual machine: compiled scalar instructions run over chunks of grid points.
//!
//! The compiler (`compile.rs`) lowers every vector operation to scalar instructions on numbered
//! registers. A register holds one value for each of `CHUNK` neighboring grid points, so each
//! instruction is one tight loop of fixed length that the compiler vectorizes, and the
//! interpretation cost is paid once per instruction and chunk instead of once per pixel. Input,
//! constant, and uniform registers are filled before the instructions run; the allocator never
//! gives an instruction's destination the register of one of its operands, which lets the
//! loops borrow them safely through `split_at_mut`.
//!
//! On x86-64 the whole loop is also compiled with AVX2 and chosen at run time. Neither build
//! enables FMA, and every operation is IEEE-exact integer or `f32` arithmetic (see `math.rs`),
//! so both builds give identical values.

use super::{math, noise};

/// Grid points evaluated together.
pub const CHUNK: usize = 64;

type Lane = [f32; CHUNK];

/// A scalar operation. Unary kinds read `a`; binary kinds read `a` and `b`; `Mix`, `Smoothstep`,
/// `Noise3`, and `Fbm3` also read `c`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Neg,
    Abs,
    Floor,
    Fract,
    Sqrt,
    Sin,
    Cos,
    Exp2,
    Log2,
    Sign,
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    Mod,
    Step,
    Pow,
    Atan2,
    /// `a + (b - a) * c`.
    Mix,
    /// `smoothstep(a, b, c)`.
    Smoothstep,
    Noise2,
    Noise3,
    Voronoi,
    Hash2,
    /// Fractal noise with the given octave count.
    Fbm2(u8),
    Fbm3(u8),
}

impl Kind {
    /// Number of operands.
    pub fn arity(self) -> usize {
        use Kind::*;
        match self {
            Neg | Abs | Floor | Fract | Sqrt | Sin | Cos | Exp2 | Log2 | Sign => 1,
            Add | Sub | Mul | Div | Min | Max | Mod | Step | Pow | Atan2 | Noise2 | Voronoi
            | Hash2 | Fbm2(_) => 2,
            Mix | Smoothstep | Noise3 | Fbm3(_) => 3,
        }
    }

    pub fn commutative(self) -> bool {
        matches!(self, Kind::Add | Kind::Mul | Kind::Min | Kind::Max)
    }

    /// The operation on one value: the reference every chunk loop matches, also used to fold
    /// constants at compile time.
    #[inline(always)]
    pub fn scalar(self, a: f32, b: f32, c: f32) -> f32 {
        use Kind::*;
        match self {
            Neg => -a,
            Abs => a.abs(),
            Floor => math::floor(a),
            Fract => math::fract(a),
            Sqrt => math::sqrt(a),
            Sin => math::sin(a),
            Cos => math::cos(a),
            Exp2 => math::exp2(a),
            Log2 => math::log2(a),
            Sign => math::sign(a),
            Add => a + b,
            Sub => a - b,
            Mul => a * b,
            Div => math::div(a, b),
            Min => a.min(b),
            Max => a.max(b),
            Mod => math::modulo(a, b),
            Step => math::step(a, b),
            Pow => math::pow(a, b),
            Atan2 => math::atan2(a, b),
            Mix => a + (b - a) * c,
            Smoothstep => math::smoothstep(a, b, c),
            Noise2 => noise::noise2(a, b),
            Noise3 => noise::noise3(a, b, c),
            Voronoi => noise::voronoi2(a, b),
            Hash2 => noise::hash2(a, b),
            Fbm2(octaves) => noise::fbm2(a, b, octaves),
            Fbm3(octaves) => noise::fbm3(a, b, c, octaves),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Instr {
    pub kind: Kind,
    pub dst: u16,
    pub args: [u16; 3],
}

/// Per-point inputs, written into their registers for every chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    /// Box-relative position, 0 to 1 across the padding box.
    UvX,
    UvY,
    /// Position in CSS px from the padding box's top-left corner.
    PxX,
    PxY,
}

/// A compiled program, ready to run.
#[derive(Clone, Debug)]
pub struct Code {
    pub instrs: Vec<Instr>,
    pub registers: usize,
    pub inputs: Vec<(u16, Input)>,
    pub constants: Vec<(u16, f32)>,
    /// Registers filled from the frame values (`Shader::values` order).
    pub uniforms: Vec<(u16, u16)>,
    /// Registers of the red, green, blue, and alpha results (straight alpha).
    pub output: [u16; 4],
}

/// The grid of points one run evaluates: point `(i, j)` has `uv = origin + (i, j) * step`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    pub origin: [f32; 2],
    pub step: [f32; 2],
    /// Padding box size in CSS px, for `px`.
    pub size: [f32; 2],
}

impl Code {
    /// Evaluates the program on every point of `grid` into `out` as premultiplied RGBA in
    /// `[0, 1]` (four values per point, row by row). `values` are the frame values the uniform
    /// registers read; `scratch` holds the registers between calls.
    pub fn run(
        &self,
        values: &[f32],
        grid: &Grid,
        out: &mut Vec<f32>,
        scratch: &mut Vec<Lane>,
        avx2: bool,
    ) {
        out.clear();
        out.resize(grid.width * grid.height * 4, 0.0);
        scratch.clear();
        scratch.resize(self.registers.max(1), [0.0; CHUNK]);
        #[cfg(target_arch = "x86_64")]
        if avx2 {
            #[target_feature(enable = "avx2")]
            fn run_avx2(
                code: &Code,
                values: &[f32],
                grid: &Grid,
                out: &mut [f32],
                regs: &mut [Lane],
            ) {
                run_impl(code, values, grid, out, regs)
            }
            // SAFETY: the caller checked that the CPU supports AVX2.
            return unsafe { run_avx2(self, values, grid, out, scratch) };
        }
        let _ = avx2;
        run_impl(self, values, grid, out, scratch)
    }
}

#[inline(always)]
fn run_impl(code: &Code, values: &[f32], grid: &Grid, out: &mut [f32], regs: &mut [Lane]) {
    for &(reg, value) in &code.constants {
        regs[reg as usize] = [value; CHUNK];
    }
    for &(reg, slot) in &code.uniforms {
        regs[reg as usize] = [values[slot as usize]; CHUNK];
    }
    for j in 0..grid.height {
        let v = grid.origin[1] + j as f32 * grid.step[1];
        let mut x0 = 0;
        while x0 < grid.width {
            for &(reg, input) in &code.inputs {
                let lane = &mut regs[reg as usize];
                match input {
                    Input::UvX => {
                        for (i, value) in lane.iter_mut().enumerate() {
                            *value = grid.origin[0] + (x0 + i) as f32 * grid.step[0];
                        }
                    }
                    Input::PxX => {
                        for (i, value) in lane.iter_mut().enumerate() {
                            *value =
                                (grid.origin[0] + (x0 + i) as f32 * grid.step[0]) * grid.size[0];
                        }
                    }
                    Input::UvY => *lane = [v; CHUNK],
                    Input::PxY => *lane = [v * grid.size[1]; CHUNK],
                }
            }
            for instr in &code.instrs {
                execute(instr, regs);
            }
            let count = CHUNK.min(grid.width - x0);
            let [r, g, b, a] = code.output.map(|reg| regs[reg as usize]);
            let row = &mut out[(j * grid.width + x0) * 4..(j * grid.width + x0 + count) * 4];
            let (pixels, _) = row.as_chunks_mut::<4>();
            for (i, pixel) in pixels.iter_mut().enumerate() {
                #[allow(clippy::manual_clamp)]
                let alpha = a[i].max(0.0).min(1.0);
                #[allow(clippy::manual_clamp)]
                let unit = |c: f32| c.max(0.0).min(1.0) * alpha;
                pixel[0] = unit(r[i]);
                pixel[1] = unit(g[i]);
                pixel[2] = unit(b[i]);
                pixel[3] = alpha;
            }
            x0 += CHUNK;
        }
    }
}

/// Borrows the destination register mutably and the operand registers immutably. The
/// destination is never one of the operands (`compile.rs` guarantees it).
#[inline(always)]
fn operands<'r>(regs: &'r mut [Lane], instr: &Instr) -> (&'r mut Lane, [&'r Lane; 3]) {
    let dst = instr.dst as usize;
    let (before, rest) = regs.split_at_mut(dst);
    let (target, after) = rest.split_first_mut().expect("destination register exists");
    let pick = |reg: u16| -> &'r Lane {
        let reg = reg as usize;
        if reg < dst {
            &before[reg]
        } else {
            &after[reg - dst - 1]
        }
    };
    let args = instr.args.map(|reg| {
        // Unused operand slots repeat the first one.
        pick(reg)
    });
    (target, args)
}

#[inline(always)]
fn execute(instr: &Instr, regs: &mut [Lane]) {
    let (d, [a, b, c]) = operands(regs, instr);
    macro_rules! lanes {
        (|$x:ident| $body:expr) => {
            for (out, &$x) in d.iter_mut().zip(a.iter()) {
                *out = $body;
            }
        };
        (|$x:ident, $y:ident| $body:expr) => {
            for ((out, &$x), &$y) in d.iter_mut().zip(a.iter()).zip(b.iter()) {
                *out = $body;
            }
        };
        (|$x:ident, $y:ident, $z:ident| $body:expr) => {
            for (((out, &$x), &$y), &$z) in d.iter_mut().zip(a.iter()).zip(b.iter()).zip(c.iter()) {
                *out = $body;
            }
        };
    }
    use Kind::*;
    match instr.kind {
        Neg => lanes!(|x| -x),
        Abs => lanes!(|x| x.abs()),
        Floor => lanes!(|x| math::floor(x)),
        Fract => lanes!(|x| math::fract(x)),
        Sqrt => lanes!(|x| math::sqrt(x)),
        Sin => lanes!(|x| math::sin(x)),
        Cos => lanes!(|x| math::cos(x)),
        Exp2 => lanes!(|x| math::exp2(x)),
        Log2 => lanes!(|x| math::log2(x)),
        Sign => lanes!(|x| math::sign(x)),
        Add => lanes!(|x, y| x + y),
        Sub => lanes!(|x, y| x - y),
        Mul => lanes!(|x, y| x * y),
        Div => lanes!(|x, y| math::div(x, y)),
        Min => lanes!(|x, y| x.min(y)),
        Max => lanes!(|x, y| x.max(y)),
        Mod => lanes!(|x, y| math::modulo(x, y)),
        Step => lanes!(|x, y| math::step(x, y)),
        Pow => lanes!(|x, y| math::pow(x, y)),
        Atan2 => lanes!(|x, y| math::atan2(x, y)),
        Mix => lanes!(|x, y, z| x + (y - x) * z),
        Smoothstep => lanes!(|x, y, z| math::smoothstep(x, y, z)),
        Noise2 => lanes!(|x, y| noise::noise2(x, y)),
        Noise3 => lanes!(|x, y, z| noise::noise3(x, y, z)),
        Voronoi => lanes!(|x, y| noise::voronoi2(x, y)),
        Hash2 => lanes!(|x, y| noise::hash2(x, y)),
        Fbm2(octaves) => fbm2_lanes(d, a, b, octaves),
        Fbm3(octaves) => fbm3_lanes(d, a, b, c, octaves),
    }
}

/// `noise::fbm2` with the octave loop outside the lane loop, so each octave vectorizes; the
/// per-lane arithmetic is the same sequence, so the values are identical.
#[inline(always)]
fn fbm2_lanes(d: &mut Lane, a: &Lane, b: &Lane, octaves: u8) {
    let (mut x, mut y) = (*a, *b);
    let mut sum = [0.0_f32; CHUNK];
    let mut amplitude = 0.5_f32;
    let mut total = 0.0_f32;
    for _ in 0..octaves {
        for i in 0..CHUNK {
            sum[i] += amplitude * noise::gradient2(x[i], y[i]);
            (x[i], y[i]) = noise::octave_step(x[i], y[i]);
        }
        total += amplitude;
        amplitude *= 0.5;
    }
    for i in 0..CHUNK {
        d[i] = noise::to_unit(sum[i] / total);
    }
}

#[inline(always)]
fn fbm3_lanes(d: &mut Lane, a: &Lane, b: &Lane, c: &Lane, octaves: u8) {
    let (mut x, mut y, mut z) = (*a, *b, *c);
    let mut sum = [0.0_f32; CHUNK];
    let mut amplitude = 0.5_f32;
    let mut total = 0.0_f32;
    for _ in 0..octaves {
        for i in 0..CHUNK {
            sum[i] += amplitude * noise::gradient3(x[i], y[i], z[i]);
            (x[i], y[i]) = noise::octave_step(x[i], y[i]);
            z[i] = z[i] * 2.0 + 7.77;
        }
        total += amplitude;
        amplitude *= 0.5;
    }
    for i in 0..CHUNK {
        d[i] = noise::to_unit(sum[i] / total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_fbm_matches_the_scalar_reference() {
        let a: Lane = std::array::from_fn(|i| i as f32 * 0.137 - 3.0);
        let b: Lane = std::array::from_fn(|i| i as f32 * -0.071 + 1.5);
        let c: Lane = std::array::from_fn(|i| i as f32 * 0.013);
        let mut d = [0.0; CHUNK];
        fbm2_lanes(&mut d, &a, &b, 5);
        for i in 0..CHUNK {
            assert_eq!(d[i].to_bits(), noise::fbm2(a[i], b[i], 5).to_bits());
        }
        fbm3_lanes(&mut d, &a, &b, &c, 4);
        for i in 0..CHUNK {
            assert_eq!(d[i].to_bits(), noise::fbm3(a[i], b[i], c[i], 4).to_bits());
        }
    }
}
