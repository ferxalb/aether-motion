//! The named procedural shaders (`shader="toon|rim|hologram|checker|stripes|gradient|noise|water|
//! dissolve"`): closed-form functions of UV, world position, normal, and clip time.
//!
//! A shader runs in two steps around lighting. [`Shader::surface`] edits the sample's albedo,
//! normal, and emitted light before lights are applied (and may discard the sample), and
//! [`Shader::finish`] edits the accumulated diffuse light (toon banding). Nothing here reads
//! state other than its arguments, and noise comes from an integer hash, so a frame is the same
//! on every thread, segment, and CPU.

use super::sample::dot3;

/// Which shader a mesh uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderKind {
    /// Lit with the diffuse light cut into `steps` flat bands.
    Toon,
    /// Adds a colored glow where the surface turns away from the camera.
    Rim,
    /// Glowing scanlines and a rim over a darkened surface.
    Hologram,
    /// Two colors in alternating UV cells.
    Checker,
    /// Two colors in alternating UV stripes along u.
    Stripes,
    /// Two colors blended along v.
    Gradient,
    /// Two colors blended by fractal noise of the world position.
    Noise,
    /// Animated ripples that bend the normal, with a view-angle blend between two colors.
    Water,
    /// Noise eats the surface away as the `progress` channel rises, with a glowing edge.
    Dissolve,
}

impl ShaderKind {
    pub const ALL: [Self; 9] = [
        Self::Toon,
        Self::Rim,
        Self::Hologram,
        Self::Checker,
        Self::Stripes,
        Self::Gradient,
        Self::Noise,
        Self::Water,
        Self::Dissolve,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Toon => "toon",
            Self::Rim => "rim",
            Self::Hologram => "hologram",
            Self::Checker => "checker",
            Self::Stripes => "stripes",
            Self::Gradient => "gradient",
            Self::Noise => "noise",
            Self::Water => "water",
            Self::Dissolve => "dissolve",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// A shader and its parameters. Colors are linear RGB.
#[derive(Clone, Debug, PartialEq)]
pub struct Shader {
    pub kind: ShaderKind,
    pub color: [f64; 3],
    pub color2: [f64; 3],
    /// Pattern frequency: cells or stripes across UV 0..1 (`checker`, `stripes`), features per
    /// 100 px (`noise`, `water`, `dissolve`), or scanlines per 100 px (`hologram`).
    pub scale: f64,
    /// Animation rate: scanline px/s (`hologram`), drift per second (`noise`), wave speed (`water`).
    pub speed: f64,
    /// Effect amount: glow gain, wave slope (`water`), or edge width (`dissolve`).
    pub strength: f64,
    /// Falloff exponent of the rim and fresnel terms.
    pub power: f64,
    /// Band count of `toon`.
    pub steps: u32,
}

impl Shader {
    /// The shader with its documented defaults. `color` and `color2` are linear RGB.
    pub fn new(kind: ShaderKind) -> Self {
        let base = Self {
            kind,
            color: [1.0; 3],
            color2: [0.0; 3],
            scale: 8.0,
            speed: 0.0,
            strength: 1.0,
            power: 3.0,
            steps: 3,
        };
        match kind {
            ShaderKind::Toon => base,
            ShaderKind::Rim => Self {
                color: [0.1, 0.35, 1.0],
                ..base
            },
            ShaderKind::Hologram => Self {
                color: [0.0, 0.6, 1.0],
                scale: 8.0,
                speed: 60.0,
                power: 2.0,
                ..base
            },
            ShaderKind::Checker | ShaderKind::Stripes => Self {
                color: [0.8, 0.8, 0.8],
                color2: [0.05, 0.05, 0.05],
                ..base
            },
            ShaderKind::Gradient => Self {
                color: [0.9, 0.5, 0.1],
                color2: [0.1, 0.2, 0.7],
                ..base
            },
            ShaderKind::Noise => Self {
                color: [0.05, 0.05, 0.08],
                color2: [0.7, 0.7, 0.75],
                scale: 3.0,
                ..base
            },
            ShaderKind::Water => Self {
                color: [0.0, 0.05, 0.2],
                color2: [0.15, 0.6, 0.8],
                scale: 3.0,
                speed: 1.0,
                strength: 0.15,
                ..base
            },
            ShaderKind::Dissolve => Self {
                color: [1.0, 0.35, 0.05],
                scale: 3.0,
                strength: 0.08,
                ..base
            },
        }
    }

    pub fn is_animated(&self) -> bool {
        match self.kind {
            ShaderKind::Hologram | ShaderKind::Water => true,
            ShaderKind::Noise => self.speed != 0.0,
            _ => false,
        }
    }

    pub fn needs_uvs(&self) -> bool {
        matches!(
            self.kind,
            ShaderKind::Checker | ShaderKind::Stripes | ShaderKind::Gradient
        )
    }

    /// Whether the shader defines the surface color itself (the material color is not applied).
    pub fn replaces_color(&self) -> bool {
        matches!(
            self.kind,
            ShaderKind::Checker
                | ShaderKind::Stripes
                | ShaderKind::Gradient
                | ShaderKind::Noise
                | ShaderKind::Water
        )
    }
}

/// What a shader knows about one sample.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub uv: [f64; 2],
    pub pos: [f64; 3],
    /// Unit vector from the surface toward the camera.
    pub eye: [f64; 3],
    pub time: f64,
    /// The look's `progress` channel at this instant.
    pub progress: f64,
}

/// The sample being shaded, before lighting.
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub albedo: [f64; 3],
    /// Unit shading normal, facing the viewer.
    pub normal: [f64; 3],
    /// Linear light added after lighting.
    pub emissive: [f64; 3],
    /// Scales diffuse and specular light.
    pub lit: f64,
    pub discard: bool,
}

fn mix(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A pseudo-random value in `[0, 1)` for an integer lattice point.
fn hash(x: i64, y: i64, z: i64) -> f64 {
    let mut h = (x as u32).wrapping_mul(0x85eb_ca6b)
        ^ (y as u32).wrapping_mul(0xc2b2_ae35)
        ^ (z as u32).wrapping_mul(0x27d4_eb2f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    f64::from(h >> 8) / f64::from(1_u32 << 24)
}

fn value_noise(p: [f64; 3]) -> f64 {
    let cell = p.map(f64::floor);
    let f = [p[0] - cell[0], p[1] - cell[1], p[2] - cell[2]].map(smooth);
    let (x, y, z) = (cell[0] as i64, cell[1] as i64, cell[2] as i64);
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let corner = |dx, dy, dz| hash(x + dx, y + dy, z + dz);
    let near = lerp(
        lerp(corner(0, 0, 0), corner(1, 0, 0), f[0]),
        lerp(corner(0, 1, 0), corner(1, 1, 0), f[0]),
        f[1],
    );
    let far = lerp(
        lerp(corner(0, 0, 1), corner(1, 0, 1), f[0]),
        lerp(corner(0, 1, 1), corner(1, 1, 1), f[0]),
        f[1],
    );
    lerp(near, far, f[2])
}

/// Three octaves of value noise, in `[0, 1)`.
fn fbm(p: [f64; 3]) -> f64 {
    let mut sum = 0.0;
    let mut amplitude = 0.5;
    let mut frequency = 1.0;
    for _ in 0..3 {
        sum += amplitude * value_noise(p.map(|v| v * frequency));
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    sum / 0.875
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = dot3(v, v).sqrt();
    if length > 1e-12 {
        v.map(|c| c / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

impl Shader {
    /// Edits the sample before lighting.
    pub fn surface(&self, input: &Input, s: &mut Surface) {
        let facing = dot3(s.normal, input.eye).clamp(0.0, 1.0);
        let edge = (1.0 - facing).powf(self.power);
        match self.kind {
            ShaderKind::Toon => {}
            ShaderKind::Rim => {
                for (e, c) in s.emissive.iter_mut().zip(self.color) {
                    *e += c * edge * self.strength;
                }
            }
            ShaderKind::Hologram => {
                let phase = (input.pos[1] + input.time * self.speed) * self.scale / 100.0;
                let lines = 0.5 + 0.5 * (phase * std::f64::consts::TAU).sin();
                let flicker = 0.9 + 0.1 * (input.time * 37.0).sin();
                s.albedo = s.albedo.map(|c| c * 0.15);
                s.lit = 0.3;
                for (e, c) in s.emissive.iter_mut().zip(self.color) {
                    *e += c * (0.25 + 0.75 * lines * lines + edge) * flicker * self.strength;
                }
            }
            ShaderKind::Checker => {
                let cell = |v: f64| (v * self.scale).floor() as i64;
                let odd = (cell(input.uv[0]) + cell(input.uv[1])).rem_euclid(2) == 1;
                let c = if odd { self.color2 } else { self.color };
                s.albedo = std::array::from_fn(|i| s.albedo[i] * c[i]);
            }
            ShaderKind::Stripes => {
                let odd = ((input.uv[0] * self.scale).floor() as i64).rem_euclid(2) == 1;
                let c = if odd { self.color2 } else { self.color };
                s.albedo = std::array::from_fn(|i| s.albedo[i] * c[i]);
            }
            ShaderKind::Gradient => {
                let c = mix(self.color, self.color2, input.uv[1].clamp(0.0, 1.0));
                s.albedo = std::array::from_fn(|i| s.albedo[i] * c[i]);
            }
            ShaderKind::Noise => {
                let k = self.scale / 100.0;
                let drift = input.time * self.speed;
                let n = fbm([
                    input.pos[0] * k + drift,
                    input.pos[1] * k,
                    input.pos[2] * k - drift,
                ]);
                let c = mix(self.color, self.color2, smooth(n));
                s.albedo = std::array::from_fn(|i| s.albedo[i] * c[i]);
            }
            ShaderKind::Water => {
                let k = self.scale / 100.0 * std::f64::consts::TAU;
                let t = input.time * self.speed;
                let (x, z) = (input.pos[0] * k, input.pos[2] * k);
                // Two crossing wave trains; their slopes tilt the normal.
                let sx = (x * 0.8 + z * 0.6 + t).cos() * 0.6 + (x * 1.7 - t * 1.3).cos() * 0.4;
                let sz =
                    (z * 0.9 - x * 0.4 + t * 0.8).cos() * 0.6 + (z * 1.9 + t * 1.1).cos() * 0.4;
                // The scene's up is -y: a flat water plane has the normal (0, -1, 0).
                let tilted = [
                    s.normal[0] + sx * self.strength,
                    s.normal[1],
                    s.normal[2] + sz * self.strength,
                ];
                s.normal = normalize(tilted);
                let fresnel = (1.0 - dot3(s.normal, input.eye).clamp(0.0, 1.0)).powf(self.power);
                s.albedo = mix(self.color, self.color2, fresnel.clamp(0.0, 1.0));
            }
            ShaderKind::Dissolve => {
                let k = self.scale / 100.0;
                let n = fbm([input.pos[0] * k, input.pos[1] * k, input.pos[2] * k]);
                // Map progress 0..1 across the noise range plus the edge band so 1 removes all.
                let threshold = input.progress.clamp(0.0, 1.0) * (1.0 + self.strength);
                if n < threshold - self.strength {
                    s.discard = true;
                } else if n < threshold {
                    let glow = 1.0 - (threshold - n) / self.strength.max(1e-9);
                    for (e, c) in s.emissive.iter_mut().zip(self.color) {
                        *e += c * (1.0 + 2.0 * glow);
                    }
                }
            }
        }
    }

    /// Edits the accumulated diffuse light.
    pub fn finish(&self, lit: &mut [f64; 3]) {
        if self.kind == ShaderKind::Toon {
            let steps = f64::from(self.steps.max(1));
            for v in lit {
                *v = (*v * steps).ceil() / steps;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(uv: [f64; 2]) -> Input {
        Input {
            uv,
            pos: [10.0, 20.0, 30.0],
            eye: [0.0, 0.0, 1.0],
            time: 0.5,
            progress: 0.0,
        }
    }

    fn surface() -> Surface {
        Surface {
            albedo: [1.0; 3],
            normal: [0.0, 0.0, 1.0],
            emissive: [0.0; 3],
            lit: 1.0,
            discard: false,
        }
    }

    #[test]
    fn names_round_trip() {
        for kind in ShaderKind::ALL {
            assert_eq!(ShaderKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(ShaderKind::from_name("phong"), None);
    }

    #[test]
    fn checker_alternates_between_cells() {
        let shader = Shader::new(ShaderKind::Checker);
        let at = |uv| {
            let mut s = surface();
            shader.surface(&input(uv), &mut s);
            s.albedo
        };
        assert_ne!(at([0.01, 0.01]), at([0.2, 0.01]));
        assert_eq!(at([0.01, 0.01]), at([0.2, 0.2]));
    }

    #[test]
    fn dissolve_discards_more_as_progress_rises() {
        let shader = Shader::new(ShaderKind::Dissolve);
        let gone = |progress: f64| {
            (0..400)
                .filter(|&i| {
                    let mut s = surface();
                    let mut input = input([0.0; 2]);
                    input.pos = [f64::from(i) * 7.3, f64::from(i) * 3.1, 0.0];
                    input.progress = progress;
                    shader.surface(&input, &mut s);
                    s.discard
                })
                .count()
        };
        assert_eq!(gone(0.0), 0);
        let (half, all) = (gone(0.5), gone(1.0));
        assert!(half > 0 && half < all, "{half} {all}");
        assert_eq!(all, 400);
    }

    #[test]
    fn noise_is_a_pure_function_of_its_inputs() {
        let p = [1.37, -4.2, 9.9];
        assert_eq!(fbm(p), fbm(p));
        assert!((0.0..1.0).contains(&fbm(p)));
    }

    #[test]
    fn toon_bands_the_light() {
        let shader = Shader::new(ShaderKind::Toon);
        let mut lit = [0.31, 0.5, 0.99];
        shader.finish(&mut lit);
        assert_eq!(lit, [1.0 / 3.0, 2.0 / 3.0, 1.0]);
    }
}
