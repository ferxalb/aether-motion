//! Type checking and lowering of a parsed shader program to `vm::Code`.
//!
//! Values have 1 to 4 components. Every vector operation is lowered to scalar nodes of a
//! single-assignment graph; scalars broadcast against vectors. While the graph is built,
//! operations on constants are folded with the same scalar functions the machine runs (so
//! folding never changes a value), identical operations are shared, and `x + 0`, `x * 1`, and
//! similar identities are dropped. Nodes the color does not reach are discarded, and registers are
//! reused once their last reader has run.

use super::syntax::{BinOp, Expr, Pos, Program};
use super::vm::{Code, Input, Instr, Kind};
use anyhow::{bail, Result};
use std::collections::HashMap;

/// Largest graph accepted after lowering.
const MAX_NODES: usize = 8192;
/// Most registers a program may use at once.
const MAX_REGISTERS: usize = 1024;
/// Octaves of `fbm` when the call gives none.
const DEFAULT_OCTAVES: u8 = 5;
const MAX_OCTAVES: u8 = 8;

/// Frame values every program can read, in `Shader::values` order; user uniforms follow.
pub const FRAME_VALUES: [&str; 4] = ["t", "size.x", "size.y", "aspect"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node {
    Input(Input),
    /// Bits of an `f32`.
    Const(u32),
    Uniform(u16),
    Op(Kind, [u32; 3]),
}

/// A declared uniform: its name and its first frame-value slot (one slot per component).
pub struct UniformSlot {
    pub name: String,
    pub components: usize,
    pub slot: usize,
}

struct Builder {
    nodes: Vec<Node>,
    shared: HashMap<Node, u32>,
}

type Value = Vec<u32>;

impl Builder {
    fn node(&mut self, node: Node) -> Result<u32> {
        if let Some(&id) = self.shared.get(&node) {
            return Ok(id);
        }
        if self.nodes.len() >= MAX_NODES {
            bail!("the program is too large: it lowers to more than {MAX_NODES} operations");
        }
        let id = self.nodes.len() as u32;
        self.nodes.push(node);
        self.shared.insert(node, id);
        Ok(id)
    }

    fn constant(&mut self, value: f32) -> Result<u32> {
        // One zero: `-0.0` and `0.0` behave alike in every operation the language has.
        let value = if value == 0.0 { 0.0 } else { value };
        self.node(Node::Const(value.to_bits()))
    }

    fn const_value(&self, id: u32) -> Option<f32> {
        match self.nodes[id as usize] {
            Node::Const(bits) => Some(f32::from_bits(bits)),
            _ => None,
        }
    }

    fn op(&mut self, kind: Kind, args: &[u32]) -> Result<u32> {
        let mut operands = [args[0]; 3];
        operands[..args.len()].copy_from_slice(args);
        if kind.commutative() && operands[1] < operands[0] {
            operands.swap(0, 1);
        }
        let constants: Vec<Option<f32>> = args.iter().map(|&a| self.const_value(a)).collect();
        if constants.iter().all(Option::is_some) {
            let v: Vec<f32> = constants.into_iter().flatten().collect();
            let get = |i: usize| v.get(i).copied().unwrap_or(0.0);
            return self.constant(kind.scalar(get(0), get(1), get(2)));
        }
        let is = |i: usize, value: f32| constants.get(i).copied().flatten() == Some(value);
        match kind {
            Kind::Add if is(1, 0.0) => return Ok(args[0]),
            Kind::Add if is(0, 0.0) => return Ok(args[1]),
            Kind::Sub if is(1, 0.0) => return Ok(args[0]),
            Kind::Mul if is(1, 1.0) => return Ok(args[0]),
            Kind::Mul if is(0, 1.0) => return Ok(args[1]),
            Kind::Div if is(1, 1.0) => return Ok(args[0]),
            Kind::Mix if is(2, 0.0) => return Ok(args[0]),
            Kind::Mix if is(2, 1.0) => return Ok(args[1]),
            _ => {}
        }
        self.node(Node::Op(kind, operands))
    }
}

struct Scope<'u> {
    lets: HashMap<String, Value>,
    uniforms: &'u [UniformSlot],
}

const BUILTIN_NAMES: [&str; 7] = ["uv", "px", "size", "t", "aspect", "pi", "tau"];

/// Whether `name` is reserved by the language (built-in values and functions).
pub fn is_reserved(name: &str) -> bool {
    BUILTIN_NAMES.contains(&name) || FUNCTIONS.contains(&name) || name == "let"
}

const FUNCTIONS: [&str; 46] = [
    "vec2",
    "vec3",
    "vec4",
    "rgb",
    "rgba",
    "abs",
    "floor",
    "ceil",
    "fract",
    "sqrt",
    "sin",
    "cos",
    "exp",
    "exp2",
    "log",
    "log2",
    "sign",
    "saturate",
    "min",
    "max",
    "mod",
    "step",
    "pow",
    "atan",
    "clamp",
    "mix",
    "smoothstep",
    "length",
    "distance",
    "dot",
    "normalize",
    "rotate",
    "noise",
    "fbm",
    "voronoi",
    "hash",
    "hsv",
    "palette",
    "luma",
    "over",
    "ridge",
    "remap",
    "tile",
    "polar",
    "grain",
    "vignette",
];

fn compile_err<T>(pos: Pos, message: impl std::fmt::Display) -> Result<T> {
    bail!("{message} (at {pos})")
}

impl Builder {
    fn expr(&mut self, expr: &Expr, scope: &Scope) -> Result<Value> {
        match expr {
            Expr::Number(value) => Ok(vec![self.constant(*value)?]),
            Expr::Color(components) => components.iter().map(|&c| self.constant(c)).collect(),
            Expr::Name(name, pos) => self.name(name, *pos, scope),
            Expr::Neg(inner) => {
                let value = self.expr(inner, scope)?;
                value.iter().map(|&v| self.op(Kind::Neg, &[v])).collect()
            }
            Expr::Binary(op, left, right, pos) => {
                let a = self.expr(left, scope)?;
                let b = self.expr(right, scope)?;
                let kind = match op {
                    BinOp::Add => Kind::Add,
                    BinOp::Sub => Kind::Sub,
                    BinOp::Mul => Kind::Mul,
                    BinOp::Div => Kind::Div,
                    BinOp::Mod => Kind::Mod,
                };
                self.zip(kind, &[a, b], *pos)
            }
            Expr::Swizzle(inner, indices, pos) => {
                let value = self.expr(inner, scope)?;
                indices
                    .iter()
                    .map(|&i| {
                        value.get(i).copied().ok_or_else(|| {
                            anyhow::anyhow!(
                                "swizzle component {} does not exist on a {}-component value (at {pos})",
                                "xyzw".as_bytes()[i] as char,
                                value.len()
                            )
                        })
                    })
                    .collect()
            }
            Expr::Call(name, args, pos) => {
                let values = args
                    .iter()
                    .map(|arg| self.expr(arg, scope))
                    .collect::<Result<Vec<_>>>()?;
                self.call(name, args, values, *pos)
            }
        }
    }

    fn name(&mut self, name: &str, pos: Pos, scope: &Scope) -> Result<Value> {
        if let Some(value) = scope.lets.get(name) {
            return Ok(value.clone());
        }
        if let Some(uniform) = scope.uniforms.iter().find(|u| u.name == name) {
            return (0..uniform.components)
                .map(|i| self.node(Node::Uniform((uniform.slot + i) as u16)))
                .collect();
        }
        Ok(match name {
            "uv" => vec![
                self.node(Node::Input(Input::UvX))?,
                self.node(Node::Input(Input::UvY))?,
            ],
            "px" => vec![
                self.node(Node::Input(Input::PxX))?,
                self.node(Node::Input(Input::PxY))?,
            ],
            "t" => vec![self.node(Node::Uniform(0))?],
            "size" => vec![self.node(Node::Uniform(1))?, self.node(Node::Uniform(2))?],
            "aspect" => vec![self.node(Node::Uniform(3))?],
            "pi" => vec![self.constant(std::f32::consts::PI)?],
            "tau" => vec![self.constant(std::f32::consts::TAU)?],
            _ if FUNCTIONS.contains(&name) => {
                return compile_err(pos, format!("{name} is a function; call it as {name}(...)"))
            }
            _ => return compile_err(pos, format!("unknown name '{name}'")),
        })
    }

    /// Applies a scalar `kind` component by component, broadcasting 1-component operands.
    fn zip(&mut self, kind: Kind, operands: &[Value], pos: Pos) -> Result<Value> {
        let width = operands.iter().map(Vec::len).max().unwrap_or(1);
        for value in operands {
            if value.len() != 1 && value.len() != width {
                return compile_err(
                    pos,
                    format!(
                        "cannot combine a {}-component value with a {width}-component value",
                        value.len()
                    ),
                );
            }
        }
        (0..width)
            .map(|i| {
                let args: Vec<u32> = operands
                    .iter()
                    .map(|v| if v.len() == 1 { v[0] } else { v[i] })
                    .collect();
                self.op(kind, &args)
            })
            .collect()
    }

    fn map(&mut self, kind: Kind, value: &Value) -> Result<Value> {
        value.iter().map(|&v| self.op(kind, &[v])).collect()
    }

    fn scalar(&mut self, value: f32) -> Result<Value> {
        Ok(vec![self.constant(value)?])
    }

    fn dot(&mut self, a: &Value, b: &Value, pos: Pos) -> Result<u32> {
        let products = self.zip(Kind::Mul, &[a.clone(), b.clone()], pos)?;
        let mut sum = products[0];
        for &p in &products[1..] {
            sum = self.op(Kind::Add, &[sum, p])?;
        }
        Ok(sum)
    }

    fn clamp(&mut self, x: &Value, lo: &Value, hi: &Value, pos: Pos) -> Result<Value> {
        let low = self.zip(Kind::Max, &[x.clone(), lo.clone()], pos)?;
        self.zip(Kind::Min, &[low, hi.clone()], pos)
    }

    fn call(&mut self, name: &str, exprs: &[Expr], args: Vec<Value>, pos: Pos) -> Result<Value> {
        let count = |n: std::ops::RangeInclusive<usize>| -> Result<()> {
            if !n.contains(&args.len()) {
                let expected = if n.start() == n.end() {
                    n.start().to_string()
                } else {
                    format!("{} to {}", n.start(), n.end())
                };
                return compile_err(
                    pos,
                    format!("{name}() takes {expected} arguments, got {}", args.len()),
                );
            }
            Ok(())
        };
        let width = |value: &Value, allowed: &[usize], what: &str| -> Result<()> {
            if !allowed.contains(&value.len()) {
                return compile_err(
                    pos,
                    format!(
                        "{what} of {name}() must have {} components, got {}",
                        allowed
                            .iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join(" or "),
                        value.len()
                    ),
                );
            }
            Ok(())
        };
        let unary = |builder: &mut Self, kind: Kind| -> Result<Value> {
            count(1..=1)?;
            builder.map(kind, &args[0])
        };
        let binary = |builder: &mut Self, kind: Kind| -> Result<Value> {
            count(2..=2)?;
            builder.zip(kind, &args, pos)
        };
        match name {
            "vec2" | "vec3" | "vec4" | "rgb" | "rgba" => {
                let n = match name {
                    "vec2" => 2,
                    "vec3" | "rgb" => 3,
                    _ => 4,
                };
                count(1..=4)?;
                if args.len() == 1 && args[0].len() == 1 {
                    return Ok(vec![args[0][0]; n]);
                }
                let flat: Value = args.concat();
                if flat.len() != n {
                    return compile_err(
                        pos,
                        format!("{name}() needs {n} components in total, got {}", flat.len()),
                    );
                }
                Ok(flat)
            }
            "abs" => unary(self, Kind::Abs),
            "floor" => unary(self, Kind::Floor),
            "ceil" => {
                count(1..=1)?;
                let negated = self.map(Kind::Neg, &args[0])?;
                let floored = self.map(Kind::Floor, &negated)?;
                self.map(Kind::Neg, &floored)
            }
            "fract" => unary(self, Kind::Fract),
            "sqrt" => unary(self, Kind::Sqrt),
            "sin" => unary(self, Kind::Sin),
            "cos" => unary(self, Kind::Cos),
            "exp2" => unary(self, Kind::Exp2),
            "log2" => unary(self, Kind::Log2),
            "sign" => unary(self, Kind::Sign),
            "exp" => {
                count(1..=1)?;
                let k = self.scalar(std::f32::consts::LOG2_E)?;
                let scaled = self.zip(Kind::Mul, &[args[0].clone(), k], pos)?;
                self.map(Kind::Exp2, &scaled)
            }
            "log" => {
                count(1..=1)?;
                let l = self.map(Kind::Log2, &args[0])?;
                let k = self.scalar(std::f32::consts::LN_2)?;
                self.zip(Kind::Mul, &[l, k], pos)
            }
            "saturate" => {
                count(1..=1)?;
                let (lo, hi) = (self.scalar(0.0)?, self.scalar(1.0)?);
                self.clamp(&args[0], &lo, &hi, pos)
            }
            "min" => binary(self, Kind::Min),
            "max" => binary(self, Kind::Max),
            "mod" => binary(self, Kind::Mod),
            "step" => binary(self, Kind::Step),
            "pow" => binary(self, Kind::Pow),
            "atan" => {
                count(1..=2)?;
                if args.len() == 1 {
                    let one = self.scalar(1.0)?;
                    self.zip(Kind::Atan2, &[args[0].clone(), one], pos)
                } else {
                    self.zip(Kind::Atan2, &args, pos)
                }
            }
            "clamp" => {
                count(3..=3)?;
                self.clamp(&args[0], &args[1], &args[2], pos)
            }
            "mix" => {
                count(3..=3)?;
                self.zip(Kind::Mix, &args, pos)
            }
            "smoothstep" => {
                count(3..=3)?;
                self.zip(Kind::Smoothstep, &args, pos)
            }
            "remap" => {
                // remap(x, in0, in1, out0, out1), clamped.
                count(5..=5)?;
                let t = self.zip(Kind::Sub, &[args[0].clone(), args[1].clone()], pos)?;
                let span = self.zip(Kind::Sub, &[args[2].clone(), args[1].clone()], pos)?;
                let t = self.zip(Kind::Div, &[t, span], pos)?;
                let (lo, hi) = (self.scalar(0.0)?, self.scalar(1.0)?);
                let t = self.clamp(&t, &lo, &hi, pos)?;
                self.zip(Kind::Mix, &[args[3].clone(), args[4].clone(), t], pos)
            }
            "length" => {
                count(1..=1)?;
                let d = self.dot(&args[0], &args[0], pos)?;
                Ok(vec![self.op(Kind::Sqrt, &[d])?])
            }
            "distance" => {
                count(2..=2)?;
                let diff = self.zip(Kind::Sub, &args, pos)?;
                let d = self.dot(&diff, &diff, pos)?;
                Ok(vec![self.op(Kind::Sqrt, &[d])?])
            }
            "dot" => {
                count(2..=2)?;
                Ok(vec![self.dot(&args[0], &args[1], pos)?])
            }
            "normalize" => {
                count(1..=1)?;
                let d = self.dot(&args[0], &args[0], pos)?;
                let length = vec![self.op(Kind::Sqrt, &[d])?];
                self.zip(Kind::Div, &[args[0].clone(), length], pos)
            }
            "rotate" => {
                // rotate(p, angle) turns a 2D point counterclockwise (in y-down screen space:
                // clockwise) around the origin.
                count(2..=2)?;
                width(&args[0], &[2], "the point")?;
                width(&args[1], &[1], "the angle")?;
                let (c, s) = (
                    self.op(Kind::Cos, &[args[1][0]])?,
                    self.op(Kind::Sin, &[args[1][0]])?,
                );
                let (x, y) = (args[0][0], args[0][1]);
                let xc = self.op(Kind::Mul, &[x, c])?;
                let ys = self.op(Kind::Mul, &[y, s])?;
                let xs = self.op(Kind::Mul, &[x, s])?;
                let yc = self.op(Kind::Mul, &[y, c])?;
                Ok(vec![
                    self.op(Kind::Sub, &[xc, ys])?,
                    self.op(Kind::Add, &[xs, yc])?,
                ])
            }
            "polar" => {
                // polar(p) = (distance from the origin, angle in turns from 0 to 1).
                count(1..=1)?;
                width(&args[0], &[2], "the point")?;
                let d = self.dot(&args[0], &args[0], pos)?;
                let r = self.op(Kind::Sqrt, &[d])?;
                let angle = self.op(Kind::Atan2, &[args[0][1], args[0][0]])?;
                let k = self.constant(1.0 / std::f32::consts::TAU)?;
                let turns = self.op(Kind::Mul, &[angle, k])?;
                Ok(vec![r, self.op(Kind::Fract, &[turns])?])
            }
            "tile" => {
                // tile(p, n) = fract(p * n): repeats the unit square n times.
                count(2..=2)?;
                let scaled = self.zip(Kind::Mul, &args, pos)?;
                self.map(Kind::Fract, &scaled)
            }
            "noise" => {
                count(1..=1)?;
                width(&args[0], &[2, 3], "the point")?;
                let p = &args[0];
                Ok(vec![if p.len() == 2 {
                    self.op(Kind::Noise2, &[p[0], p[1]])?
                } else {
                    self.op(Kind::Noise3, &[p[0], p[1], p[2]])?
                }])
            }
            "ridge" => {
                // Ridged noise: 1 - |2 noise - 1|, sharp crests for mountains and lightning.
                count(1..=2)?;
                let n = self.call(
                    if args.len() == 2 { "fbm" } else { "noise" },
                    exprs,
                    args.clone(),
                    pos,
                )?;
                let two = self.constant(2.0)?;
                let one = self.constant(1.0)?;
                let scaled = self.op(Kind::Mul, &[n[0], two])?;
                let centered = self.op(Kind::Sub, &[scaled, one])?;
                let folded = self.op(Kind::Abs, &[centered])?;
                Ok(vec![self.op(Kind::Sub, &[one, folded])?])
            }
            "fbm" => {
                count(1..=2)?;
                width(&args[0], &[2, 3], "the point")?;
                let octaves = match exprs.get(1) {
                    None => DEFAULT_OCTAVES,
                    Some(Expr::Number(n))
                        if n.fract() == 0.0 && (1.0..=f32::from(MAX_OCTAVES)).contains(n) =>
                    {
                        *n as u8
                    }
                    Some(_) => {
                        return compile_err(
                            pos,
                            format!("the octave count of fbm() must be a whole number from 1 to {MAX_OCTAVES}"),
                        )
                    }
                };
                let p = &args[0];
                Ok(vec![if p.len() == 2 {
                    self.op(Kind::Fbm2(octaves), &[p[0], p[1]])?
                } else {
                    self.op(Kind::Fbm3(octaves), &[p[0], p[1], p[2]])?
                }])
            }
            "voronoi" => {
                count(1..=1)?;
                width(&args[0], &[2], "the point")?;
                Ok(vec![self.op(Kind::Voronoi, &[args[0][0], args[0][1]])?])
            }
            "hash" => {
                count(1..=1)?;
                width(&args[0], &[1, 2], "the point")?;
                let y = match args[0].get(1) {
                    Some(&y) => y,
                    None => self.constant(0.0)?,
                };
                Ok(vec![self.op(Kind::Hash2, &[args[0][0], y])?])
            }
            "grain" => {
                // grain(amount): per-pixel animated noise in [-amount/2, amount/2], for film
                // texture. Reads px and t, so it changes every frame.
                count(1..=1)?;
                width(&args[0], &[1], "the amount")?;
                let px = self.name(
                    "px",
                    pos,
                    &Scope {
                        lets: HashMap::new(),
                        uniforms: &[],
                    },
                )?;
                let t = self.node(Node::Uniform(0))?;
                let rate = self.constant(24.0 * 97.0)?;
                let shift = self.op(Kind::Mul, &[t, rate])?;
                let shift = self.op(Kind::Floor, &[shift])?;
                let x = self.op(Kind::Add, &[px[0], shift])?;
                let h = self.op(Kind::Hash2, &[x, px[1]])?;
                let half = self.constant(0.5)?;
                let centered = self.op(Kind::Sub, &[h, half])?;
                Ok(vec![self.op(Kind::Mul, &[centered, args[0][0]])?])
            }
            "vignette" => {
                // vignette(strength): 1 at the center falling toward 1 - strength in the corners.
                count(1..=1)?;
                width(&args[0], &[1], "the strength")?;
                let uv = self.name(
                    "uv",
                    pos,
                    &Scope {
                        lets: HashMap::new(),
                        uniforms: &[],
                    },
                )?;
                let half = self.scalar(0.5)?;
                let centered = self.zip(Kind::Sub, &[uv, half], pos)?;
                let d = self.dot(&centered, &centered, pos)?;
                let k = self.constant(2.0)?;
                let falloff = self.op(Kind::Mul, &[d, k])?;
                let amount = self.op(Kind::Mul, &[falloff, args[0][0]])?;
                let one = self.constant(1.0)?;
                Ok(vec![self.op(Kind::Sub, &[one, amount])?])
            }
            "hsv" => {
                count(1..=3)?;
                let flat: Value = args.concat();
                if flat.len() != 3 {
                    return compile_err(pos, "hsv() needs a hue, saturation, and value");
                }
                let (h, s, v) = (vec![flat[0]], vec![flat[1]], vec![flat[2]]);
                let k = vec![
                    self.constant(1.0)?,
                    self.constant(2.0 / 3.0)?,
                    self.constant(1.0 / 3.0)?,
                ];
                let shifted = self.zip(Kind::Add, &[h, k], pos)?;
                let wrapped = self.map(Kind::Fract, &shifted)?;
                let six = self.scalar(6.0)?;
                let three = self.scalar(3.0)?;
                let scaled = self.zip(Kind::Mul, &[wrapped, six], pos)?;
                let centered = self.zip(Kind::Sub, &[scaled, three], pos)?;
                let p = self.map(Kind::Abs, &centered)?;
                let one = self.scalar(1.0)?;
                let zero = self.scalar(0.0)?;
                let p = self.zip(Kind::Sub, &[p, one.clone()], pos)?;
                let p = self.clamp(&p, &zero, &one, pos)?;
                let c = self.zip(Kind::Mix, &[one, p, s], pos)?;
                self.zip(Kind::Mul, &[c, v], pos)
            }
            "palette" => {
                // Cosine palette a + b cos(tau (c t + d)).
                count(5..=5)?;
                width(&args[0], &[1], "the position")?;
                let ct = self.zip(Kind::Mul, &[args[3].clone(), args[0].clone()], pos)?;
                let phase = self.zip(Kind::Add, &[ct, args[4].clone()], pos)?;
                let tau = self.scalar(std::f32::consts::TAU)?;
                let angle = self.zip(Kind::Mul, &[phase, tau], pos)?;
                let wave = self.map(Kind::Cos, &angle)?;
                let swing = self.zip(Kind::Mul, &[args[2].clone(), wave], pos)?;
                let color = self.zip(Kind::Add, &[args[1].clone(), swing], pos)?;
                if color.len() == 1 {
                    return Ok(vec![color[0]; 3]);
                }
                Ok(color)
            }
            "luma" => {
                count(1..=1)?;
                width(&args[0], &[3, 4], "the color")?;
                let weights = vec![
                    self.constant(0.2126)?,
                    self.constant(0.7152)?,
                    self.constant(0.0722)?,
                ];
                Ok(vec![self.dot(&args[0][..3].to_vec(), &weights, pos)?])
            }
            "over" => {
                // over(top, bottom): source-over compositing of straight-alpha colors.
                count(2..=2)?;
                width(&args[0], &[4], "the top color")?;
                width(&args[1], &[3, 4], "the bottom color")?;
                let one = self.constant(1.0)?;
                let top_alpha = args[0][3];
                let bottom_alpha = match args[1].get(3) {
                    Some(&a) => a,
                    None => one,
                };
                let rest = self.op(Kind::Sub, &[one, top_alpha])?;
                let under = self.op(Kind::Mul, &[bottom_alpha, rest])?;
                let alpha = self.op(Kind::Add, &[top_alpha, under])?;
                let mut out = Vec::with_capacity(4);
                for (&top_color, &bottom_color) in args[0][..3].iter().zip(&args[1][..3]) {
                    let top = self.op(Kind::Mul, &[top_color, top_alpha])?;
                    let bottom = self.op(Kind::Mul, &[bottom_color, under])?;
                    let sum = self.op(Kind::Add, &[top, bottom])?;
                    out.push(self.op(Kind::Div, &[sum, alpha])?);
                }
                if args[1].len() == 3 {
                    return Ok(out);
                }
                out.push(alpha);
                Ok(out)
            }
            _ => compile_err(pos, format!("unknown function '{name}'")),
        }
    }
}

/// Compiles `program`. `uniforms` lists the declared uniforms and their slots.
pub fn compile(program: &Program, uniforms: &[UniformSlot]) -> Result<(Code, bool)> {
    let mut builder = Builder {
        nodes: Vec::new(),
        shared: HashMap::new(),
    };
    let mut scope = Scope {
        lets: HashMap::new(),
        uniforms,
    };
    for (name, expr, pos) in &program.bindings {
        if is_reserved(name) {
            return compile_err(*pos, format!("'{name}' is a built-in name"));
        }
        // A later binding may rebind a name (its expression still reads the earlier value), but
        // never a uniform, so a declared input cannot be hidden by accident.
        if uniforms.iter().any(|u| &u.name == name) {
            return compile_err(*pos, format!("'{name}' is a uniform; choose another name"));
        }
        let value = builder.expr(expr, &scope)?;
        scope.lets.insert(name.clone(), value);
    }
    let color = builder.expr(&program.result, &scope)?;
    let one = builder.constant(1.0)?;
    let output = match color.len() {
        1 => [color[0], color[0], color[0], one],
        3 => [color[0], color[1], color[2], one],
        4 => [color[0], color[1], color[2], color[3]],
        n => bail!(
            "the final expression is a {n}-component value; it must be a gray level, rgb(...), or rgba(...)"
        ),
    };
    allocate(&builder.nodes, output)
}

/// Schedules the nodes the output reaches in post-order (each operand chain right before its
/// reader, newest operand first, which keeps few values alive), assigns registers (pinned for inputs, constants, and
/// uniforms; reused for operations), and reports whether the program reads `t`.
fn allocate(nodes: &[Node], output: [u32; 4]) -> Result<(Code, bool)> {
    let operands_of = |id: u32| -> &[u32] {
        match &nodes[id as usize] {
            Node::Op(kind, args) => &args[..kind.arity()],
            _ => &[],
        }
    };
    let mut order: Vec<u32> = Vec::new();
    let mut done = vec![false; nodes.len()];
    let mut stack: Vec<(u32, bool)> = output.iter().rev().map(|&id| (id, false)).collect();
    while let Some((id, expanded)) = stack.pop() {
        if done[id as usize] {
            continue;
        }
        if expanded {
            done[id as usize] = true;
            order.push(id);
            continue;
        }
        stack.push((id, true));
        // The newest operand (usually the deepest chain) is scheduled first: pushed last.
        let mut args = operands_of(id).to_vec();
        args.sort_unstable();
        for arg in args {
            if !done[arg as usize] {
                stack.push((arg, false));
            }
        }
    }
    // Position of the last reader of each node, so its register can be reused afterwards.
    let mut last_use = vec![usize::MAX; nodes.len()];
    for (position, &id) in order.iter().enumerate() {
        for &arg in operands_of(id) {
            last_use[arg as usize] = position;
        }
    }
    let mut register = vec![u16::MAX; nodes.len()];
    let mut next = 0_usize;
    let mut code = Code {
        instrs: Vec::new(),
        registers: 0,
        inputs: Vec::new(),
        constants: Vec::new(),
        uniforms: Vec::new(),
        output: [0; 4],
    };
    let mut reads_time = false;
    for &id in &order {
        let reg = next as u16;
        match nodes[id as usize] {
            Node::Input(input) => code.inputs.push((reg, input)),
            Node::Const(bits) => code.constants.push((reg, f32::from_bits(bits))),
            Node::Uniform(slot) => {
                reads_time |= slot == 0;
                code.uniforms.push((reg, slot));
            }
            Node::Op(..) => continue,
        }
        register[id as usize] = reg;
        next += 1;
    }
    let pinned = next;
    let mut free: Vec<u16> = Vec::new();
    for (position, &id) in order.iter().enumerate() {
        let Node::Op(kind, args) = nodes[id as usize] else {
            continue;
        };
        let dst = match free.pop() {
            Some(reg) => reg,
            None => {
                next += 1;
                if next > MAX_REGISTERS {
                    bail!("the program keeps too many values alive at once (more than {MAX_REGISTERS})");
                }
                (next - 1) as u16
            }
        };
        register[id as usize] = dst;
        let arity = kind.arity();
        let mut used = [register[args[0] as usize]; 3];
        for k in 0..arity {
            used[k] = register[args[k] as usize];
        }
        debug_assert!(used.iter().all(|&r| r != dst && r != u16::MAX));
        code.instrs.push(Instr {
            kind,
            dst,
            args: used,
        });
        // Free the operands read here for the last time; after choosing `dst`, so the
        // destination never aliases an operand.
        for (k, &arg) in args[..arity].iter().enumerate() {
            if args[..k].contains(&arg) {
                continue;
            }
            let reg = register[arg as usize];
            if reg as usize >= pinned
                && last_use[arg as usize] == position
                && !output.contains(&arg)
            {
                free.push(reg);
            }
        }
    }
    code.output = output.map(|id| register[id as usize]);
    code.registers = next;
    Ok((code, reads_time))
}

#[cfg(test)]
mod tests {
    use super::super::syntax::parse;
    use super::*;

    fn build(source: &str) -> Result<Code> {
        compile(&parse(source)?, &[]).map(|(code, _)| code)
    }

    #[test]
    fn folds_constants_and_shares_operations() {
        let code = build("let a = 2 * 3 + 1; rgb(uv.x * a, uv.x * a, a / 10)").unwrap();
        // One multiplication, shared by red and green; blue folded to 0.7.
        assert_eq!(code.instrs.len(), 1);
        assert!(code.constants.iter().any(|&(_, v)| v == 0.7));
    }

    #[test]
    fn reuses_registers() {
        let source = (0..200)
            .map(|i| format!("let v{i} = sin(uv.x * {i}.5);"))
            .collect::<String>()
            + "rgb("
            + &(0..200)
                .map(|i| format!("v{i}"))
                .collect::<Vec<_>>()
                .join(" + ")
            + ")";
        let code = build(&source).unwrap();
        // Each of the 200 distinct constants has its own register; the operations share a few.
        let working = code.registers - code.constants.len() - code.inputs.len();
        assert!(working < 8, "{working} working registers");
    }

    #[test]
    fn rejects_type_errors() {
        for (source, message) in [
            ("uv + vec3(1)", "cannot combine"),
            ("uv", "2-component"),
            ("fbm(uv, 12)", "octave"),
            ("let uv = 1; uv", "built-in"),
            ("let a = uv.x; let a = a * vec2(1); a", "2-component"),
            ("foo(1)", "unknown function"),
            ("bar", "unknown name"),
            ("uv.z", "does not exist"),
            ("mix(1, 2)", "takes 3"),
        ] {
            let error = build(source).unwrap_err().to_string();
            assert!(error.contains(message), "{source}: {error}");
        }
    }

    #[test]
    fn reports_time_use() {
        let program = parse("noise(vec3(uv * 4, t))").unwrap();
        assert!(compile(&program, &[]).unwrap().1);
        let program = parse("noise(uv * 4)").unwrap();
        assert!(!compile(&program, &[]).unwrap().1);
    }
}
