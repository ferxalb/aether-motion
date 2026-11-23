//! Materials and lights: flat (per-face) Lambert with optional Blinn-Phong specular, point lights
//! with windowed inverse-square falloff, and opt-in hard shadows.
//!
//! Colors are mixed in linear light: the base color and light colors are decoded from sRGB, the
//! face's diffuse term `ambient + sum(color x intensity x attenuation x visibility x
//! max(0, n . l))` multiplies the base color, a white specular term `ks x max(0, n . h)^shininess`
//! is added per light, and the result is clamped and encoded back to 8-bit sRGB. One color per
//! triangle, computed on the CPU in `f64`, is what every backend fills the triangle with.

use super::mesh::{cross, dot, sub};
use super::{LightKind, Material, Shading};
use crate::model::Color;

/// Lights used when a scene declares none: a soft white ambient and a key light from the upper
/// left, slightly in front.
pub const DEFAULT_RIG: [(f64, [f64; 3]); 2] = [(0.35, [0.0; 3]), (0.9, [0.45, 0.8, -0.4])];

/// Offset (scene px) along the face normal that keeps a shadow ray off its own surface.
const SHADOW_BIAS: f64 = 0.5;

/// A world-space triangle that can block light.
pub type Caster = [[f64; 3]; 3];

/// The lights of a scene at one instant, in world space and linear light.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lighting {
    pub ambient: [f64; 3],
    /// `(direction of travel, unit, world; linear color x intensity)`.
    pub directional: Vec<([f64; 3], [f64; 3])>,
    /// Parallel to `directional`: whether each light is shadowed by scene meshes.
    pub directional_shadows: Vec<bool>,
    pub points: Vec<PointLight>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointLight {
    pub position: [f64; 3],
    pub color: [f64; 3],
    pub range: f64,
    pub shadows: bool,
}

impl PointLight {
    /// Windowed inverse square: 1 at the light, 0 at `range`.
    fn attenuation(&self, distance: f64) -> f64 {
        let ratio = distance / self.range;
        let window = (1.0 - ratio.powi(4)).clamp(0.0, 1.0);
        let reference = self.range * 0.25;
        window * window / (1.0 + (distance / reference).powi(2))
    }
}

/// Whether the segment from `from` along unit `toward`, up to `limit`, hits any caster.
fn blocked(casters: &[Caster], from: [f64; 3], toward: [f64; 3], limit: f64) -> bool {
    casters.iter().any(|tri| {
        let e1 = sub(tri[1], tri[0]);
        let e2 = sub(tri[2], tri[0]);
        let p = cross(toward, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-12 {
            return false;
        }
        let inv = 1.0 / det;
        let s = sub(from, tri[0]);
        let u = dot(s, p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let q = cross(s, e1);
        let v = dot(toward, q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        let t = dot(e2, q) * inv;
        t > 0.0 && t < limit
    })
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
        shadows: bool,
        position: [f64; 3],
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
                    self.directional_shadows.push(shadows);
                }
            }
            LightKind::Point { range } => self.points.push(PointLight {
                position,
                color: c,
                range: *range,
                shadows,
            }),
        }
    }

    /// The default rig ([`DEFAULT_RIG`]).
    pub fn default_rig() -> Self {
        let mut lighting = Self::default();
        let white = Color::WHITE;
        lighting.add(
            &LightKind::Ambient,
            white,
            DEFAULT_RIG[0].0,
            false,
            [0.0; 3],
            |d| d,
        );
        lighting.add(
            &LightKind::Directional {
                direction: DEFAULT_RIG[1].1,
            },
            white,
            DEFAULT_RIG[1].0,
            false,
            [0.0; 3],
            |d| d,
        );
        lighting
    }

    /// The color of a face with unit `normal` (facing the viewer) at world point `at`, seen along
    /// unit `view` (toward the viewer). `casters` are the triangles that shadow lit faces.
    pub fn shade(
        &self,
        material: &Material,
        normal: [f64; 3],
        at: [f64; 3],
        view: [f64; 3],
        casters: &[Caster],
    ) -> [u8; 3] {
        let base = linear(material.color);
        match material.shading {
            Shading::Unlit => [material.color.r, material.color.g, material.color.b],
            Shading::Lambert => {
                let origin = std::array::from_fn(|i| at[i] + normal[i] * SHADOW_BIAS);
                let mut lit = self.ambient;
                let mut gloss = [0.0; 3];
                let mut add = |toward: [f64; 3], color: [f64; 3], strength: f64| {
                    let amount = dot(normal, toward);
                    if amount <= 0.0 || strength <= 0.0 {
                        return;
                    }
                    for (l, c) in lit.iter_mut().zip(color) {
                        *l += c * amount * strength;
                    }
                    if material.specular > 0.0 {
                        let sum = [0, 1, 2].map(|i| toward[i] + view[i]);
                        let length = dot(sum, sum).sqrt();
                        if length > 1e-12 {
                            let h = dot(normal, sum.map(|v| v / length)).max(0.0);
                            let k = material.specular * h.powf(material.shininess) * strength;
                            for (g, c) in gloss.iter_mut().zip(color) {
                                *g += c * k;
                            }
                        }
                    }
                };
                for (i, (direction, color)) in self.directional.iter().enumerate() {
                    let toward = direction.map(|v| -v);
                    let shadowed = self.directional_shadows[i]
                        && dot(normal, toward) > 0.0
                        && blocked(casters, origin, toward, f64::INFINITY);
                    add(toward, *color, if shadowed { 0.0 } else { 1.0 });
                }
                for point in &self.points {
                    let delta = sub(point.position, at);
                    let distance = dot(delta, delta).sqrt();
                    if distance <= 1e-9 || distance >= point.range {
                        continue;
                    }
                    let toward = delta.map(|v| v / distance);
                    let shadowed = point.shadows
                        && dot(normal, toward) > 0.0
                        && blocked(casters, origin, toward, distance);
                    let strength = if shadowed {
                        0.0
                    } else {
                        point.attenuation(distance)
                    };
                    add(toward, point.color, strength);
                }
                std::array::from_fn(|i| to_srgb(base[i] * lit[i] + gloss[i]))
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
            false,
            [0.0; 3],
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
            specular: 0.0,
            shininess: 32.0,
        };
        // Facing the light head on gives the base color; edge on gives black.
        assert_eq!(
            lighting.shade(&material, [0.0, 0.0, 1.0], [0.0; 3], [0.0, 0.0, 1.0], &[]),
            [200, 100, 50]
        );
        assert_eq!(
            lighting.shade(&material, [1.0, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0], &[]),
            [0, 0, 0]
        );
        let unlit = Material {
            shading: Shading::Unlit,
            ..material
        };
        assert_eq!(
            lighting.shade(&unlit, [1.0, 0.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0], &[]),
            [200, 100, 50]
        );
    }

    #[test]
    fn point_light_falls_off_and_casters_shadow_it() {
        let mut lighting = Lighting::default();
        lighting.add(
            &LightKind::Point { range: 400.0 },
            Color::WHITE,
            1.0,
            true,
            [0.0, 0.0, 100.0],
            |d| d,
        );
        let material = Material {
            shading: Shading::Lambert,
            color: Color::WHITE,
            cull: Cull::Back,
            specular: 0.0,
            shininess: 32.0,
        };
        let shade = |z: f64, casters: &[Caster]| {
            lighting.shade(
                &material,
                [0.0, 0.0, 1.0],
                [0.0, 0.0, z],
                [0.0, 0.0, 1.0],
                casters,
            )
        };
        let near = shade(50.0, &[]);
        let far = shade(-200.0, &[]);
        assert!(near[0] > far[0] && far[0] > 0);
        assert_eq!(shade(-400.0, &[]), [0, 0, 0]);
        let wall = [[-50.0, -50.0, 75.0], [50.0, -50.0, 75.0], [0.0, 50.0, 75.0]];
        assert_eq!(shade(50.0, &[wall]), [0, 0, 0]);
    }
}
