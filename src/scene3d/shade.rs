//! Materials and lights: flat (per-face) Lambert and unlit shading.
//!
//! Colors are mixed in linear light: the base color and light colors are decoded from sRGB, the
//! face's diffuse term `ambient + sum(color x intensity x max(0, n . -direction))` multiplies the
//! base color, and the result is clamped and encoded back to 8-bit sRGB. One color per triangle,
//! computed on the CPU in `f64`, is what every backend fills the triangle with.

use super::mesh::dot;
use super::{LightKind, Material, Shading};
use crate::model::Color;

/// Lights used when a scene declares none: a soft white ambient and a key light from the upper
/// left, slightly in front.
pub const DEFAULT_RIG: [(f64, [f64; 3]); 2] = [(0.35, [0.0; 3]), (0.9, [0.45, 0.8, -0.4])];

/// The lights of a scene at one instant, in world space and linear light.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lighting {
    pub ambient: [f64; 3],
    /// `(direction of travel, unit, world; linear color x intensity)`.
    pub directional: Vec<([f64; 3], [f64; 3])>,
}

pub fn to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn to_srgb(v: f64) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let c = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
}

fn linear(color: Color) -> [f64; 3] {
    [color.r, color.g, color.b].map(to_linear)
}

impl Lighting {
    pub fn add(
        &mut self,
        kind: &LightKind,
        color: Color,
        intensity: f64,
        turn: impl Fn([f64; 3]) -> [f64; 3],
    ) {
        let c = linear(color).map(|v| v * intensity);
        match kind {
            LightKind::Ambient => {
                for (a, v) in self.ambient.iter_mut().zip(c) {
                    *a += v;
                }
            }
            LightKind::Directional { direction } => {
                let d = turn(*direction);
                let length = dot(d, d).sqrt();
                if length > 1e-12 && length.is_finite() {
                    self.directional.push((d.map(|v| v / length), c));
                }
            }
        }
    }

    /// The default rig ([`DEFAULT_RIG`]).
    pub fn default_rig() -> Self {
        let mut lighting = Self::default();
        let white = Color::WHITE;
        lighting.add(&LightKind::Ambient, white, DEFAULT_RIG[0].0, |d| d);
        lighting.add(
            &LightKind::Directional {
                direction: DEFAULT_RIG[1].1,
            },
            white,
            DEFAULT_RIG[1].0,
            |d| d,
        );
        lighting
    }

    /// The color of a face with unit normal `normal` (facing the viewer).
    pub fn shade(&self, material: &Material, normal: [f64; 3]) -> [u8; 3] {
        let base = linear(material.color);
        match material.shading {
            Shading::Unlit => [material.color.r, material.color.g, material.color.b],
            Shading::Lambert => {
                let mut lit = self.ambient;
                for (direction, color) in &self.directional {
                    let amount = (-dot(normal, *direction)).max(0.0);
                    for (l, c) in lit.iter_mut().zip(color) {
                        *l += c * amount;
                    }
                }
                std::array::from_fn(|i| to_srgb(base[i] * lit[i]))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d::Cull;

    #[test]
    fn transfer_functions_round_trip() {
        for c in 0..=255u8 {
            assert_eq!(to_srgb(to_linear(c)), c);
        }
    }

    #[test]
    fn lambert_follows_the_light_and_unlit_ignores_it() {
        let mut lighting = Lighting::default();
        lighting.add(
            &LightKind::Directional {
                direction: [0.0, 0.0, -1.0],
            },
            Color::WHITE,
            1.0,
            |d| d,
        );
        let material = Material {
            shading: Shading::Lambert,
            color: Color {
                r: 200,
                g: 100,
                b: 50,
                a: 255,
            },
            cull: Cull::Back,
        };
        // Facing the light head on gives the base color; edge on gives black.
        assert_eq!(lighting.shade(&material, [0.0, 0.0, 1.0]), [200, 100, 50]);
        assert_eq!(lighting.shade(&material, [1.0, 0.0, 0.0]), [0, 0, 0]);
        let unlit = Material {
            shading: Shading::Unlit,
            ..material
        };
        assert_eq!(lighting.shade(&unlit, [1.0, 0.0, 0.0]), [200, 100, 50]);
    }
}
