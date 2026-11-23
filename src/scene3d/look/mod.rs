//! Surface looks for native 3D meshes: image textures, normal and emissive maps, UV transforms,
//! smooth shading, alpha cutouts, and a fixed set of named procedural shaders.
//!
//! A [`Look`] is the part of a mesh's appearance that needs per-sample evaluation. Meshes without
//! one keep the flat, one-color-per-triangle path that every backend draws. Meshes with a look
//! are drawn by the deterministic CPU rasterizer, which interpolates UVs, world position, and
//! normals perspective-correctly and calls [`sample::shade`] for each covered sample. Shaders are
//! closed-form Rust functions of UV, world position, normal, and clip time, never user code, so a
//! frame stays a pure function of time.

pub mod sample;
pub mod shader;
pub mod texture;

use super::Channel;
pub use shader::{Shader, ShaderKind};
use std::sync::Arc;
pub use texture::Texture;

/// Most distinct textures one scene may reference.
pub const MAX_TEXTURES: usize = 16;
/// Most texels (all levels excluded) the textures of one scene may hold.
pub const MAX_SCENE_TEXELS: u64 = 32 << 20;

/// How UVs outside 0..1 are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    Clamp,
}

/// The per-sample appearance of one mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// Base color texture (sRGB), multiplied by the material color.
    pub albedo: Option<Arc<Texture>>,
    /// Tangent-space normal map (+y of the image points toward -v, as in OpenGL conventions).
    pub normal_map: Option<Arc<Texture>>,
    /// Emissive texture (sRGB), multiplied by `emissive`.
    pub emissive_map: Option<Arc<Texture>>,
    /// Linear emitted light, added after lighting.
    pub emissive: [f64; 3],
    pub uv_scale: [f64; 2],
    pub uv_offset: [f64; 2],
    /// UV travel per second.
    pub uv_scroll: [f64; 2],
    pub wrap: Wrap,
    /// 0 flattens the normal map, 1 applies it as authored.
    pub normal_strength: f64,
    /// Samples whose texture alpha is below this are not drawn (0 disables the test).
    pub alpha_cutoff: f64,
    /// Interpolated vertex normals instead of one normal per face.
    pub smooth: bool,
    pub shader: Option<Shader>,
    /// Dissolve progress (0 to 1), animatable with the `progress` keyframe channel.
    pub progress: Channel,
}

impl Look {
    /// A look that changes nothing: used as the starting point of the parser.
    pub fn plain() -> Self {
        Self {
            albedo: None,
            normal_map: None,
            emissive_map: None,
            emissive: [0.0; 3],
            uv_scale: [1.0; 2],
            uv_offset: [0.0; 2],
            uv_scroll: [0.0; 2],
            wrap: Wrap::Repeat,
            normal_strength: 1.0,
            alpha_cutoff: 0.0,
            smooth: false,
            shader: None,
            progress: Channel::constant(0.0),
        }
    }

    /// Whether the picture of this look changes with the clip time.
    pub fn is_animated(&self) -> bool {
        self.uv_scroll != [0.0; 2]
            || self.progress.is_animated()
            || self.shader.as_ref().is_some_and(Shader::is_animated)
    }

    /// Whether the look reads UVs (so the mesh must have them).
    pub fn needs_uvs(&self) -> bool {
        self.albedo.is_some()
            || self.normal_map.is_some()
            || self.emissive_map.is_some()
            || self.shader.as_ref().is_some_and(Shader::needs_uvs)
    }

    /// Texels held by this look's distinct textures, for the scene budget.
    pub fn textures(&self) -> impl Iterator<Item = &Arc<Texture>> {
        [&self.albedo, &self.normal_map, &self.emissive_map]
            .into_iter()
            .flatten()
    }
}
