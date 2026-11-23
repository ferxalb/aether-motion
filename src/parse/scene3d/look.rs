//! Reads the look attributes of an `<hf-mesh>`: textures and maps, UV transforms, smooth shading,
//! alpha cutouts, and the named shaders with their `shader-*` parameters.
//!
//! Texture files are local paths relative to the composition, noted as parse inputs like images.
//! Each file is decoded once per scene however many meshes use it, within the per-scene limits of
//! [`MAX_TEXTURES`] and [`MAX_SCENE_TEXELS`].

use super::{scalar, Keyed};
use crate::model::Color;
use crate::parse::css::vars::substitute;
use crate::parse::css::Vars;
use crate::parse::html::attr;
use crate::parse::values::color::parse_color;
use crate::scene3d::look::texture::Encoding;
use crate::scene3d::look::{
    Look, Shader, ShaderKind, Texture, Wrap, MAX_SCENE_TEXELS, MAX_TEXTURES,
};
use crate::scene3d::shade::to_linear;
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Attributes of `<hf-mesh>` that belong to a look.
pub(super) const LOOK_ATTRIBUTES: [&str; 22] = [
    "texture",
    "normal-map",
    "emissive-map",
    "emissive",
    "emissive-intensity",
    "uv-scale",
    "uv-offset",
    "uv-scroll",
    "uv-wrap",
    "normal-strength",
    "alpha-cutoff",
    "smooth",
    "shader",
    "shader-color",
    "shader-color-2",
    "shader-scale",
    "shader-speed",
    "shader-strength",
    "shader-power",
    "shader-steps",
    "shader-progress",
    "uvs",
];

/// Decoded textures of one scene, shared by every mesh that names the same file.
#[derive(Default)]
pub(super) struct Textures {
    loaded: HashMap<(PathBuf, bool), Arc<Texture>>,
    texels: u64,
}

impl Textures {
    fn get(&mut self, base: &Path, src: &str, encoding: Encoding) -> Result<Arc<Texture>> {
        if src.starts_with("data:") || src.contains("://") {
            bail!("only local texture files are supported: {src}");
        }
        let path = base.join(src);
        let key = (path.clone(), encoding == Encoding::Srgb);
        if let Some(texture) = self.loaded.get(&key) {
            return Ok(texture.clone());
        }
        if self.loaded.len() >= MAX_TEXTURES {
            bail!("a data-hf-3d scene may use at most {MAX_TEXTURES} texture files");
        }
        crate::parse::reads::note(&path);
        let texture = Texture::load(&path, encoding)?;
        self.texels += texture.texels();
        if self.texels > MAX_SCENE_TEXELS {
            bail!(
                "the textures of a data-hf-3d scene may hold at most {MAX_SCENE_TEXELS} texels in total; use smaller images"
            );
        }
        let texture = Arc::new(texture);
        self.loaded.insert(key, texture.clone());
        Ok(texture)
    }
}

/// What parsing a mesh's look needs from the scene.
pub(super) struct LookContext<'a> {
    pub base: &'a Path,
    pub vars: Option<&'a Vars>,
    pub textures: &'a mut Textures,
}

fn flag(node: &NodeRef, name: &str) -> Result<bool> {
    match attr(node, name).as_deref().map(str::trim) {
        None => Ok(false),
        Some("" | "true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => bail!("{name} is true or false (got \"{other}\")"),
    }
}

fn pair(node: &NodeRef, name: &str, default: [f64; 2], limit: f64) -> Result<[f64; 2]> {
    let Some(raw) = attr(node, name) else {
        return Ok(default);
    };
    let values = super::numbers(&raw, name)?;
    let [a, b] = values[..] else {
        if let [v] = values[..] {
            if v.abs() <= limit {
                return Ok([v, v]);
            }
        }
        bail!("{name} takes two numbers \"u v\" (got \"{raw}\")");
    };
    if a.abs() > limit || b.abs() > limit {
        bail!("{name} values must be within {limit}");
    }
    Ok([a, b])
}

fn css_color(node: &NodeRef, name: &str, vars: Option<&Vars>) -> Result<Option<Color>> {
    attr(node, name)
        .map(|raw| {
            let value = match vars {
                Some(vars) if raw.contains("var(") => substitute(&raw, vars, 0)?,
                _ => raw.clone(),
            };
            parse_color(value.trim()).with_context(|| format!("{name}=\"{raw}\""))
        })
        .transpose()
}

fn linear(color: Color) -> [f64; 3] {
    [color.r, color.g, color.b].map(to_linear)
}

/// The look of a mesh, or `None` when it has no look attribute.
pub(super) fn parse(
    node: &NodeRef,
    context: &mut LookContext,
    keyed: &Keyed,
) -> Result<Option<Look>> {
    let any = LOOK_ATTRIBUTES
        .iter()
        .any(|name| *name != "uvs" && attr(node, name).is_some())
        || keyed.names("progress");
    if !any {
        return Ok(None);
    }
    let mut look = Look::plain();
    let map = |name: &str, encoding, context: &mut LookContext| -> Result<Option<Arc<Texture>>> {
        attr(node, name)
            .map(|src| {
                context
                    .textures
                    .get(context.base, src.trim(), encoding)
                    .with_context(|| format!("{name}=\"{src}\""))
            })
            .transpose()
    };
    look.albedo = map("texture", Encoding::Srgb, context)?;
    look.normal_map = map("normal-map", Encoding::Linear, context)?;
    look.emissive_map = map("emissive-map", Encoding::Srgb, context)?;

    let intensity = scalar(node, "emissive-intensity", 1.0, (0.0, 16.0))?;
    look.emissive = match css_color(node, "emissive", context.vars)? {
        Some(color) => linear(color).map(|v| v * intensity),
        None if look.emissive_map.is_some() => [intensity; 3],
        None => {
            if attr(node, "emissive-intensity").is_some() {
                bail!("emissive-intensity needs emissive or emissive-map");
            }
            [0.0; 3]
        }
    };
    look.uv_scale = pair(node, "uv-scale", [1.0; 2], 10_000.0)?;
    if look.uv_scale.contains(&0.0) {
        bail!("uv-scale values must not be 0");
    }
    look.uv_offset = pair(node, "uv-offset", [0.0; 2], 10_000.0)?;
    look.uv_scroll = pair(node, "uv-scroll", [0.0; 2], 1000.0)?;
    look.wrap = match attr(node, "uv-wrap").as_deref().map(str::trim) {
        None | Some("repeat") => Wrap::Repeat,
        Some("clamp") => Wrap::Clamp,
        Some(other) => bail!("uv-wrap must be repeat or clamp (got \"{other}\")"),
    };
    look.normal_strength = scalar(node, "normal-strength", 1.0, (0.0, 4.0))?;
    if attr(node, "normal-strength").is_some() && look.normal_map.is_none() {
        bail!("normal-strength needs normal-map");
    }
    look.alpha_cutoff = scalar(node, "alpha-cutoff", 0.0, (0.0, 1.0))?;
    if look.alpha_cutoff > 0.0 && !look.albedo.as_ref().is_some_and(|t| t.has_alpha()) {
        bail!("alpha-cutoff needs a texture with transparent pixels");
    }
    look.smooth = flag(node, "smooth")?;
    look.shader = shader(node, context.vars)?;
    let dissolve = look
        .shader
        .as_ref()
        .is_some_and(|s| s.kind == ShaderKind::Dissolve);
    if (attr(node, "shader-progress").is_some() || keyed.names("progress")) && !dissolve {
        bail!("progress belongs to shader=\"dissolve\"");
    }
    let base = scalar(node, "shader-progress", 0.0, (0.0, 1.0))?;
    look.progress = keyed.channel("progress", base, (0.0, 1.0))?;
    Ok(Some(look))
}

fn shader(node: &NodeRef, vars: Option<&Vars>) -> Result<Option<Shader>> {
    let params = [
        "shader-color",
        "shader-color-2",
        "shader-scale",
        "shader-speed",
        "shader-strength",
        "shader-power",
        "shader-steps",
    ];
    let Some(name) = attr(node, "shader") else {
        if let Some(param) = params.iter().find(|p| attr(node, p).is_some()) {
            bail!("{param} needs shader=\"...\"");
        }
        return Ok(None);
    };
    let name = name.trim();
    let kind = ShaderKind::from_name(name).ok_or_else(|| {
        let all: Vec<&str> = ShaderKind::ALL.iter().map(|k| k.name()).collect();
        anyhow!("unknown shader \"{name}\"; use {}", all.join(", "))
    })?;
    let mut shader = Shader::new(kind);
    if let Some(color) = css_color(node, "shader-color", vars)? {
        shader.color = linear(color);
    }
    if let Some(color) = css_color(node, "shader-color-2", vars)? {
        shader.color2 = linear(color);
    }
    shader.scale = scalar(node, "shader-scale", shader.scale, (0.01, 10_000.0))?;
    shader.speed = scalar(node, "shader-speed", shader.speed, (-10_000.0, 10_000.0))?;
    shader.strength = scalar(node, "shader-strength", shader.strength, (0.0, 16.0))?;
    shader.power = scalar(node, "shader-power", shader.power, (0.1, 16.0))?;
    let steps = scalar(node, "shader-steps", f64::from(shader.steps), (2.0, 16.0))?;
    if steps.fract() != 0.0 {
        bail!("shader-steps must be a whole number from 2 to 16");
    }
    shader.steps = steps as u32;
    if kind == ShaderKind::Dissolve && shader.strength <= 0.0 {
        bail!("shader-strength of a dissolve must be greater than 0");
    }
    Ok(Some(shader))
}

/// Parses `uvs="u v, u v, ..."` of a custom mesh into one pair per vertex.
pub(super) fn uvs(node: &NodeRef) -> Result<Option<Vec<[f32; 2]>>> {
    let Some(raw) = attr(node, "uvs") else {
        return Ok(None);
    };
    let values = super::numbers(&raw, "uvs")?;
    if values.len() % 2 != 0 {
        bail!(
            "uvs must be a list of \"u v\" pairs ({} numbers)",
            values.len()
        );
    }
    Ok(Some(
        values
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| c.map(|v| v as f32))
            .collect(),
    ))
}
