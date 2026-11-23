//! Per-sample shading for meshes with a [`Look`]: the data the frame builder prepares for each
//! triangle ([`ShadedTri`]), the per-instant state ([`Shaded`]), and [`shade`], which the CPU
//! rasterizer calls for every covered sample.
//!
//! The rasterizer interpolates [`ATTRS`] values per corner (UV, world position, normal)
//! perspective-correctly. [`shade`] then reads the textures at a mip level chosen from the screen
//! derivatives of the UVs, perturbs the normal, runs the named shader, applies the scene's lights
//! (shadows are decided once per triangle), and returns an sRGB color, or `None` when the sample
//! is cut out. Everything is `f64` arithmetic on the sample's coordinates, so the result does not
//! depend on threads, tiles, or render segments.

use super::shader::{Input, Surface};
use super::{Look, Wrap};
use crate::scene3d::camera::View;
use crate::scene3d::shade::{to_srgb, Lighting};
use crate::scene3d::Material;
use std::sync::OnceLock;

/// Interpolated values per corner: `u, v, x, y, z, nx, ny, nz`.
pub const ATTRS: usize = 8;

pub fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot3(v, v).sqrt();
    (length > 1e-12 && length.is_finite()).then(|| v.map(|c| c / length))
}

/// A triangle that is shaded per sample.
#[derive(Clone, Debug)]
pub struct ShadedTri {
    /// Index into the scene's looks.
    pub look: u32,
    pub material: Material,
    /// Material color in linear light.
    pub tint: [f64; 3],
    pub uv: [[f64; 2]; 3],
    /// Unit normals at the corners, already facing the viewer.
    pub normal: [[f64; 3]; 3],
    /// Surface direction of increasing u, and of "up" in the image (-v).
    pub tangent: [f64; 3],
    pub bitangent: [f64; 3],
    /// Shadow decisions of this face ([`Lighting::shadow_mask`]).
    pub shadowed: u32,
}

impl ShadedTri {
    /// The corner attributes the rasterizer interpolates.
    pub fn attributes(&self, corners: &[[f64; 3]; 3]) -> [[f64; ATTRS]; 3] {
        std::array::from_fn(|i| {
            let (uv, p, n) = (self.uv[i], corners[i], self.normal[i]);
            [uv[0], uv[1], p[0], p[1], p[2], n[0], n[1], n[2]]
        })
    }

    /// Tangent and bitangent from the corner positions and UVs, or the zero vectors when the UV
    /// mapping is degenerate (a normal map then does nothing).
    pub fn frame(corners: &[[f64; 3]; 3], uv: &[[f64; 2]; 3]) -> ([f64; 3], [f64; 3]) {
        let e1: [f64; 3] = std::array::from_fn(|i| corners[1][i] - corners[0][i]);
        let e2: [f64; 3] = std::array::from_fn(|i| corners[2][i] - corners[0][i]);
        let (du1, dv1) = (uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]);
        let (du2, dv2) = (uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-18 {
            return ([0.0; 3], [0.0; 3]);
        }
        let tangent: [f64; 3] = std::array::from_fn(|i| (e1[i] * dv2 - e2[i] * dv1) / det);
        let toward_v: [f64; 3] = std::array::from_fn(|i| (e2[i] * du1 - e1[i] * du2) / det);
        (
            unit(tangent).unwrap_or([0.0; 3]),
            unit(toward_v.map(|c: f64| -c)).unwrap_or([0.0; 3]),
        )
    }
}

/// Everything per-sample shading needs about one drawn instant.
#[derive(Debug, Default)]
pub struct Shaded {
    pub lighting: Lighting,
    pub tris: Vec<ShadedTri>,
    /// Clip time of the instant.
    pub time: f64,
    /// Each look's `progress` channel at `time`.
    pub progress: Vec<f64>,
}

/// The borrowed state the rasterizer shades with.
#[derive(Clone, Copy)]
pub struct Surfaces<'a> {
    pub looks: &'a [Look],
    pub shaded: &'a Shaded,
    pub view: &'a View,
}

/// Planes of `1/w` and `attribute/w` over the sample grid: `plane[0]` is `1/w`, `plane[1 + k]`
/// is attribute `k`, each as `[value at origin, d/dx, d/dy]`.
pub type Planes = [[f64; 3]; ATTRS + 1];

const ENCODE_STEPS: usize = 16_384;

fn encode_table() -> &'static [u8] {
    static TABLE: OnceLock<Vec<u8>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=ENCODE_STEPS)
            .map(|i| to_srgb(i as f64 / ENCODE_STEPS as f64))
            .collect()
    })
}

fn encode(v: f64) -> u32 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    u32::from(encode_table()[(v * ENCODE_STEPS as f64 + 0.5) as usize])
}

/// Mip level for a texture of `size` texels whose UVs change by `d` (per sample step).
fn level_of(size: [usize; 2], d: [[f64; 2]; 2]) -> f64 {
    let length = |step: [f64; 2]| (step[0] * size[0] as f64).hypot(step[1] * size[1] as f64);
    let rho = length(d[0]).max(length(d[1]));
    if rho > 1.0 {
        rho.log2()
    } else {
        0.0
    }
}

/// The sRGB color (`R | G << 8 | B << 16`) of the sample at `(x, y)` (sample units), or `None`
/// when the sample is cut out and must not be drawn.
pub fn shade(surfaces: &Surfaces, tri: &ShadedTri, planes: &Planes, x: f64, y: f64) -> Option<u32> {
    let at = |p: &[f64; 3]| p[0] + p[1] * x + p[2] * y;
    let winv = at(&planes[0]);
    if winv.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let q = 1.0 / winv;
    let attr: [f64; ATTRS] = std::array::from_fn(|k| at(&planes[k + 1]) * q);
    let look = &surfaces.looks[tri.look as usize];
    let shaded = surfaces.shaded;
    let time = shaded.time;
    let (su, sv) = (look.uv_scale[0], look.uv_scale[1]);
    let uv = [
        attr[0] * su + look.uv_offset[0] + look.uv_scroll[0] * time,
        attr[1] * sv + look.uv_offset[1] + look.uv_scroll[1] * time,
    ];
    // d(uv)/dx and d(uv)/dy of the transformed UVs: d(A/W) = (dA - a dW) / W.
    let grad = |axis: usize| {
        let wg = planes[0][axis];
        [
            (planes[1][axis] - attr[0] * wg) * q * su,
            (planes[2][axis] - attr[1] * wg) * q * sv,
        ]
    };
    let steps = [grad(1), grad(2)];
    let lod = |texture: &super::Texture| level_of([texture.width(), texture.height()], steps);
    let wrap: Wrap = look.wrap;

    let mut albedo = [1.0; 3];
    let mut alpha = 1.0;
    if let Some(texture) = &look.albedo {
        let s = texture.sample(uv, lod(texture), wrap);
        albedo = [s[0], s[1], s[2]];
        alpha = s[3];
    }
    if look.alpha_cutoff > 0.0 && alpha < look.alpha_cutoff {
        return None;
    }
    let replaces = look.shader.as_ref().is_some_and(|s| s.replaces_color());
    if !replaces {
        albedo = std::array::from_fn(|i| albedo[i] * tri.tint[i]);
    }

    let pos = [attr[2], attr[3], attr[4]];
    let mut normal = unit([attr[5], attr[6], attr[7]])?;
    if let Some(map) = &look.normal_map {
        let (t, b) = (tri.tangent, tri.bitangent);
        if t != [0.0; 3] && look.normal_strength > 0.0 {
            if let Some(t) = unit(std::array::from_fn(|i| t[i] - normal[i] * dot3(normal, t))) {
                let side = cross3(normal, t);
                let b = if dot3(side, b) < 0.0 {
                    side.map(|c| -c)
                } else {
                    side
                };
                let s = map.sample(uv, lod(map), wrap);
                let k = look.normal_strength;
                let local = [
                    (s[0] * 2.0 - 1.0) * k,
                    (s[1] * 2.0 - 1.0) * k,
                    s[2] * 2.0 - 1.0,
                ];
                let bent = std::array::from_fn(|i| {
                    t[i] * local[0] + b[i] * local[1] + normal[i] * local[2]
                });
                normal = unit(bent).unwrap_or(normal);
            }
        }
    }

    let mut emissive = look.emissive;
    if let Some(map) = &look.emissive_map {
        let s = map.sample(uv, lod(map), wrap);
        emissive = std::array::from_fn(|i| emissive[i] * s[i]);
    }
    let eye = unit(surfaces.view.toward_viewer(pos)).unwrap_or(normal);
    let mut surface = Surface {
        albedo,
        normal,
        emissive,
        lit: 1.0,
        discard: false,
    };
    if let Some(shader) = &look.shader {
        let input = Input {
            uv,
            pos,
            eye,
            time,
            progress: shaded
                .progress
                .get(tri.look as usize)
                .copied()
                .unwrap_or(0.0),
        };
        shader.surface(&input, &mut surface);
        if surface.discard {
            return None;
        }
    }

    let linear: [f64; 3] = if tri.material.shading == crate::scene3d::Shading::Unlit {
        std::array::from_fn(|i| surface.albedo[i] + surface.emissive[i])
    } else {
        let (mut lit, gloss) = shaded.lighting.terms(
            tri.material.specular,
            tri.material.shininess,
            surface.normal,
            pos,
            eye,
            &mut |source, _, _| tri.shadowed & source.bit() != 0,
        );
        if let Some(shader) = &look.shader {
            shader.finish(&mut lit);
        }
        std::array::from_fn(|i| {
            surface.albedo[i] * lit[i] * surface.lit + gloss[i] * surface.lit + surface.emissive[i]
        })
    };
    Some(encode(linear[0]) | encode(linear[1]) << 8 | encode(linear[2]) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_follows_the_uv_axes() {
        // u grows along +x, v grows along +y (down the image), so "up in the image" is -y.
        let corners = [[0.0; 3], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
        let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let (t, b) = ShadedTri::frame(&corners, &uv);
        assert_eq!(t, [1.0, 0.0, 0.0]);
        assert_eq!(b, [0.0, -1.0, 0.0]);
        assert_eq!(ShadedTri::frame(&corners, &[[0.0; 2]; 3]).0, [0.0; 3]);
    }

    #[test]
    fn encoding_matches_the_exact_transfer_function() {
        for i in 0..=255u32 {
            let linear = crate::scene3d::shade::to_linear(i as u8);
            assert_eq!(encode(linear), i);
        }
        assert_eq!(encode(f64::NAN), 0);
        assert_eq!(encode(7.0), 255);
    }

    #[test]
    fn minified_footprints_pick_coarser_levels() {
        assert_eq!(
            level_of([256, 256], [[1.0 / 512.0, 0.0], [0.0, 1.0 / 512.0]]),
            0.0
        );
        let level = level_of([256, 256], [[4.0 / 256.0, 0.0], [0.0, 1.0 / 256.0]]);
        assert!((level - 2.0).abs() < 1e-9, "{level}");
    }
}
