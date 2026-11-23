//! `data-hf-3d` scenes: the host element's options and its `<hf-node>`, `<hf-mesh>`,
//! `<hf-light>`, `<hf-beam>`, `<hf-body>`, `<hf-hinge>`, and `<hf-camera>` descendants. Texture,
//! shader, and smooth-shading attributes of meshes are read by the `look` submodule.
//!
//! The scene's elements are not 2D elements: they take no part in layout or painting and are
//! consumed here, so the walk does not visit them. Every attribute is checked against the tag's
//! list, so typos fail with an error instead of being ignored. Keyframes (`data-hf-keyframes`)
//! use the channel names of 2.5D planes (`x`, `y`, `z`, `rotateX`, `rotateY`, `rotate`) plus
//! `scale`, `scaleX`, `scaleY`, `scaleZ`, light `intensity`, and the camera's `yaw`, `pitch`,
//! `roll`, and `zoom`; omitted channels inherit the previous keyframe (the static attribute value
//! before the first), easings are the `data-hf-keyframes` names plus `smooth`, and the first and
//! last values hold outside the keyed interval. `color` attributes may use `var()` of the host's
//! custom properties; `data-var-*` bindings are applied to the DOM before this parse.

mod beam;
mod look;
mod physics;

use crate::camera::curve::Interp;
use crate::model::Color;
use crate::parse::animation::camera::parse_camera_easing;
use crate::parse::css::vars::substitute;
use crate::parse::css::Vars;
use crate::parse::html::attr;
use crate::parse::values::color::parse_color;
use crate::scene3d::mesh::{Mesh, Primitive, DETAIL, MAX_MESH_TRIANGLES, SEGMENTS};
use crate::scene3d::{
    CameraSpec, Channel, Cull, Light, LightKind, Material, Node, NodeKind, Projection, Scene3d,
    Shading, Target, Transform, ANTIALIAS, DEFAULT_FAR, DEFAULT_FOCAL, DEFAULT_NEAR,
    MAX_DIRECTIONAL_LIGHTS, MAX_NODES, MAX_POINT_LIGHTS, MAX_SHADOW_TRIANGLES, MAX_TRIANGLES,
};
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Default supersampling per axis.
pub(in crate::parse) const DEFAULT_ANTIALIAS: u32 = 2;
/// Largest coordinate (px) a position, size, or vertex may have.
const MAX_COORDINATE: f64 = 1_000_000.0;
/// Largest rotation (degrees).
const MAX_ANGLE: f64 = 100_000.0;
/// Largest scale factor (in magnitude).
const MAX_SCALE: f64 = 1000.0;

const TRANSFORM_ATTRIBUTES: [&str; 5] =
    ["id", "position", "rotation", "scale", "data-hf-keyframes"];
const MESH_ATTRIBUTES: [&str; 14] = [
    "shape",
    "size",
    "radius",
    "height",
    "segments",
    "detail",
    "vertices",
    "indices",
    "color",
    "material",
    "cull",
    "class",
    "specular",
    "shininess",
];
const LIGHT_ATTRIBUTES: [&str; 7] = [
    "type",
    "color",
    "intensity",
    "direction",
    "class",
    "range",
    "shadows",
];
const CAMERA_ATTRIBUTES: [&str; 14] = [
    "id",
    "position",
    "data-hf-keyframes",
    "projection",
    "perspective",
    "fov",
    "zoom",
    "yaw",
    "pitch",
    "roll",
    "near",
    "far",
    "target",
    "class",
];

const NODE_CHANNELS: [&str; 10] = [
    "x", "y", "z", "rotateX", "rotateY", "rotate", "scale", "scaleX", "scaleY", "scaleZ",
];
/// Mesh channels: the node channels plus the dissolve `progress` of the mesh's shader.
const MESH_CHANNELS: [&str; 11] = [
    "x", "y", "z", "rotateX", "rotateY", "rotate", "scale", "scaleX", "scaleY", "scaleZ",
    "progress",
];
const CAMERA_CHANNELS: [&str; 7] = ["x", "y", "z", "yaw", "pitch", "roll", "zoom"];
const KINEMATIC_CHANNELS: [&str; 6] = ["x", "y", "z", "rotateX", "rotateY", "rotate"];

/// Whether `node` hosts a 3D scene.
pub(in crate::parse) fn is_host(node: &NodeRef) -> bool {
    attr(node, "data-hf-3d").is_some()
}

/// Reads the scene hosted by `node` (which has `data-hf-3d`), whose CSS box is `size` px.
pub(in crate::parse) fn parse_scene(
    node: &NodeRef,
    tag: &str,
    vars: &Vars,
    base: &Path,
) -> Result<Scene3d> {
    match attr(node, "data-hf-3d").as_deref().map(str::trim) {
        Some("" | "true") => {}
        Some(other) => {
            bail!("data-hf-3d is a marker attribute: write data-hf-3d (got \"{other}\")")
        }
        None => unreachable!("parse_scene is called on hosts"),
    }
    if !matches!(tag, "div" | "section" | "main" | "article") {
        bail!("data-hf-3d belongs on a <div> (or <section>, <main>, <article>), not <{tag}>");
    }
    for conflict in [
        "data-hf-space",
        "data-hf-camera",
        "data-hf-shots",
        "data-hf-split",
        "data-hf-counter",
        "data-hf-captions",
    ] {
        if attr(node, conflict).is_some() {
            bail!("data-hf-3d cannot be combined with {conflict} on the same element; wrap it");
        }
    }
    let element = node.as_element().expect("element");
    for key in element.attributes.borrow().map.keys() {
        let key = &*key.local;
        if key.starts_with("data-hf-3d-") && key != "data-hf-3d-antialias" {
            bail!("unknown attribute {key}; data-hf-3d takes data-hf-3d-antialias");
        }
    }
    let antialias = attr(node, "data-hf-3d-antialias")
        .map(|raw| {
            raw.trim()
                .parse::<u32>()
                .ok()
                .filter(|v| ANTIALIAS.contains(v))
                .ok_or_else(|| anyhow!("data-hf-3d-antialias must be 1, 2, 3, or 4 (got {raw})"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_ANTIALIAS);
    let mut builder = Builder {
        vars: Some(vars),
        base: Some(base),
        ..Builder::default()
    };
    for child in node.children() {
        builder.child(&child, None)?;
    }
    builder.finish(antialias)
}

#[derive(Default)]
struct Builder<'a> {
    /// Custom properties of the host, for `var()` in `color` attributes.
    vars: Option<&'a Vars>,
    /// Folder of the composition, where texture paths start.
    base: Option<&'a Path>,
    textures: look::Textures,
    looks: Vec<crate::scene3d::look::Look>,
    nodes: Vec<Node>,
    camera: Option<usize>,
    /// Pending camera target node ids.
    target: Option<String>,
    beam: Option<usize>,
    beam_occluder: Option<String>,
    ids: HashMap<String, usize>,
    lights: usize,
    directional: usize,
    points: usize,
    shadows: bool,
    triangles: usize,
}

impl Builder<'_> {
    fn child(&mut self, node: &NodeRef, parent: Option<usize>) -> Result<()> {
        if let Some(text) = node.as_text() {
            if !text.borrow().trim().is_empty() {
                bail!("text inside a data-hf-3d scene is not drawn; put it in a 2D element over the scene");
            }
            return Ok(());
        }
        let Some(el) = node.as_element() else {
            return Ok(());
        };
        let tag = el.name.local.to_string();
        let label = match attr(node, "id") {
            Some(id) => format!("<{tag} id=\"{id}\">"),
            None => format!("<{tag}>"),
        };
        let index = self
            .node(node, &tag, parent)
            .with_context(|| format!("{label} in data-hf-3d"))?;
        for child in node.children() {
            if matches!(tag.as_str(), "hf-hinge" | "hf-beam")
                && (child.as_element().is_some()
                    || child
                        .as_text()
                        .is_some_and(|s| !s.borrow().trim().is_empty()))
            {
                bail!("{label} needs to be empty");
            }
            self.child(&child, Some(index))?;
        }
        Ok(())
    }

    fn node(&mut self, node: &NodeRef, tag: &str, parent: Option<usize>) -> Result<usize> {
        if self.nodes.len() >= MAX_NODES {
            bail!("a data-hf-3d scene may hold at most {MAX_NODES} nodes");
        }
        let (kind, transform) = match tag {
            "hf-node" => {
                allowed(node, &[])?;
                (NodeKind::Group, transform(node, &NODE_CHANNELS)?)
            }
            "hf-body" => {
                let body = physics::body(node, parent)?;
                let channels: &[&str] = if body.kind == crate::scene3d::physics::BodyKind::Kinematic {
                    &KINEMATIC_CHANNELS
                } else { &[] };
                (NodeKind::Body(body), transform(node, channels)?)
            }
            "hf-hinge" => (NodeKind::Hinge(physics::hinge(node, parent)?), Transform::default()),
            "hf-beam" => {
                if self.beam.is_some() {
                    bail!("a data-hf-3d scene has at most one <hf-beam>");
                }
                let (beam, occluder) = beam::parse(node, self.vars)?;
                self.beam = Some(self.nodes.len());
                self.beam_occluder = occluder;
                (NodeKind::Beam(beam), transform(node, &NODE_CHANNELS)?)
            }
            "hf-mesh" => {
                allowed(node, &[&MESH_ATTRIBUTES[..], &look::LOOK_ATTRIBUTES[..]].concat())?;
                let keyed = keyframes(node, &MESH_CHANNELS)?;
                let mut context = look::LookContext {
                    base: self.base.unwrap_or_else(|| Path::new(".")),
                    vars: self.vars,
                    textures: &mut self.textures,
                };
                let (mut kind, look) = mesh(node, &mut self.triangles, self.vars, &mut context, &keyed)?;
                if let (Some(look), NodeKind::Mesh { material, .. }) = (look, &mut kind) {
                    material.look = Some(self.looks.len() as u32);
                    self.looks.push(look);
                }
                (kind, transform(node, &MESH_CHANNELS)?)
            }
            "hf-light" => {
                allowed(node, &LIGHT_ATTRIBUTES)?;
                let mut channels = NODE_CHANNELS.to_vec();
                channels.push("intensity");
                let light = light(node, self.vars)?;
                self.lights += 1;
                if matches!(light.kind, LightKind::Directional { .. }) {
                    self.directional += 1;
                    if self.directional > MAX_DIRECTIONAL_LIGHTS {
                        bail!("a scene may have at most {MAX_DIRECTIONAL_LIGHTS} directional lights");
                    }
                }
                if matches!(light.kind, LightKind::Point { .. }) {
                    self.points += 1;
                    if self.points > MAX_POINT_LIGHTS {
                        bail!("a scene may have at most {MAX_POINT_LIGHTS} point lights");
                    }
                }
                self.shadows |= light.shadows;
                (NodeKind::Light(light), transform(node, &channels)?)
            }
            "hf-camera" => {
                for key in node.as_element().unwrap().attributes.borrow().map.keys() {
                    if !CAMERA_ATTRIBUTES.contains(&&*key.local) && !key.local.starts_with("data-var-") {
                        bail!(
                            "unknown attribute {} on <hf-camera>; use {}",
                            &*key.local,
                            CAMERA_ATTRIBUTES.join(", ")
                        );
                    }
                }
                if self.camera.is_some() {
                    bail!("a data-hf-3d scene has at most one <hf-camera>");
                }
                self.camera = Some(self.nodes.len());
                let (spec, transform, target) = camera(node)?;
                self.target = target;
                (NodeKind::Camera(Box::new(spec)), transform)
            }
            other => bail!(
                "<{other}> cannot be part of a 3D scene; use <hf-node>, <hf-body>, <hf-hinge>, <hf-mesh>, <hf-light>, <hf-beam>, or <hf-camera>"
            ),
        };
        let id = attr(node, "id");
        let index = self.nodes.len();
        if let Some(id) = &id {
            if self.ids.insert(id.clone(), index).is_some() {
                bail!("duplicate id {id} in the 3D scene");
            }
        }
        self.nodes.push(Node {
            id,
            parent,
            kind,
            transform,
        });
        Ok(index)
    }

    fn finish(mut self, antialias: u32) -> Result<Scene3d> {
        if self.triangles > MAX_TRIANGLES {
            bail!(
                "a data-hf-3d scene may hold at most {MAX_TRIANGLES} triangles (this one has {}); lower segments or detail",
                self.triangles
            );
        }
        if self.shadows && self.triangles > MAX_SHADOW_TRIANGLES {
            bail!(
                "a scene with shadows=\"true\" lights may hold at most {MAX_SHADOW_TRIANGLES} triangles (this one has {}); lower segments or detail",
                self.triangles
            );
        }
        if let (Some(camera), Some(id)) = (self.camera, self.target.take()) {
            let index = *self
                .ids
                .get(&id)
                .ok_or_else(|| anyhow!("<hf-camera> target #{id} is not a node of this scene"))?;
            if index == camera || is_ancestor(&self.nodes, camera, index) {
                bail!("<hf-camera> cannot target itself or a node inside it");
            }
            if let NodeKind::Camera(spec) = &mut self.nodes[camera].kind {
                spec.target = Some(Target::Node(index));
            }
        }
        let (transform, spec) = CameraSpec::default_for(DEFAULT_FOCAL);
        if let (Some(index), Some(id)) = (self.beam, self.beam_occluder) {
            let occluder = self
                .ids
                .get(&id)
                .copied()
                .filter(|&i| matches!(self.nodes[i].kind, NodeKind::Mesh { shape: "box", .. }))
                .ok_or_else(|| {
                    anyhow!("<hf-beam> occluder #{id} must be a box mesh in the same viewport")
                })?;
            if let NodeKind::Beam(beam) = &mut self.nodes[index].kind {
                beam.occluder = Some(occluder);
            }
        }
        Ok(Scene3d {
            nodes: self.nodes,
            camera: self.camera,
            default_camera: (transform, spec),
            antialias,
            default_lights: self.lights == 0,
            physics: None,
            looks: self.looks,
        })
    }
}

/// Whether `node` lies inside `ancestor`.
fn is_ancestor(nodes: &[Node], ancestor: usize, mut node: usize) -> bool {
    while let Some(parent) = nodes[node].parent {
        if parent == ancestor {
            return true;
        }
        node = parent;
    }
    false
}

/// Errors on attributes that are neither transform attributes nor in `extra`.
fn allowed(node: &NodeRef, extra: &[&str]) -> Result<()> {
    let tag = node.as_element().unwrap().name.local.to_string();
    for key in node.as_element().unwrap().attributes.borrow().map.keys() {
        let key = &*key.local;
        if !TRANSFORM_ATTRIBUTES.contains(&key)
            && !extra.contains(&key)
            && key != "class"
            && !key.starts_with("data-var-")
        {
            let mut all: Vec<&str> = TRANSFORM_ATTRIBUTES.to_vec();
            all.extend(extra.iter().filter(|k| **k != "class"));
            bail!("unknown attribute {key} on <{tag}>; use {}", all.join(", "));
        }
    }
    Ok(())
}

/// Numbers separated by spaces or commas.
fn numbers(raw: &str, name: &str) -> Result<Vec<f64>> {
    raw.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let v: f64 = s
                .parse()
                .map_err(|_| anyhow!("{name}: invalid number {s}"))?;
            if !v.is_finite() {
                bail!("{name}: numbers must be finite");
            }
            Ok(v)
        })
        .collect()
}

fn vector(node: &NodeRef, name: &str, default: [f64; 3], limit: f64) -> Result<[f64; 3]> {
    let Some(raw) = attr(node, name) else {
        return Ok(default);
    };
    let values = numbers(&raw, name)?;
    let [x, y, z] = values[..] else {
        bail!("{name} takes three numbers \"x y z\" (got \"{raw}\")");
    };
    if [x, y, z].iter().any(|v| v.abs() > limit) {
        bail!("{name} values must be within {limit}");
    }
    Ok([x, y, z])
}

fn scalar(node: &NodeRef, name: &str, default: f64, range: (f64, f64)) -> Result<f64> {
    let Some(raw) = attr(node, name) else {
        return Ok(default);
    };
    let values = numbers(&raw, name)?;
    let [v] = values[..] else {
        bail!("{name} takes one number (got \"{raw}\")");
    };
    if v < range.0 || v > range.1 {
        bail!(
            "{name} must be between {} and {} (got {v})",
            range.0,
            range.1
        );
    }
    Ok(v)
}

fn positive(node: &NodeRef, name: &str, default: f64) -> Result<f64> {
    let v = scalar(node, name, default, (0.0, MAX_COORDINATE))?;
    if v <= 0.0 {
        bail!("{name} must be greater than 0");
    }
    Ok(v)
}

/// The `color` attribute: any supported CSS color, `var(--name)` of the host's custom properties
/// (composition variables and themes) included.
fn color(node: &NodeRef, default: Color, vars: Option<&Vars>) -> Result<Color> {
    attr(node, "color").map_or(Ok(default), |raw| {
        let value = match vars {
            Some(vars) if raw.contains("var(") => substitute(&raw, vars, 0)?,
            _ => raw.clone(),
        };
        parse_color(value.trim()).with_context(|| format!("color=\"{raw}\""))
    })
}

fn mesh(
    node: &NodeRef,
    total: &mut usize,
    vars: Option<&Vars>,
    context: &mut look::LookContext,
    keyed: &Keyed,
) -> Result<(NodeKind, Option<crate::scene3d::look::Look>)> {
    let shape = attr(node, "shape").ok_or_else(|| {
        anyhow!("<hf-mesh> needs shape: plane, box, pyramid, cone, cylinder, sphere, or custom")
    })?;
    let shape = shape.trim();
    let used = |names: &[&str]| -> Result<()> {
        for name in [
            "size", "radius", "height", "segments", "detail", "vertices", "indices", "uvs",
        ] {
            if !names.contains(&name) && attr(node, name).is_some() {
                bail!("{name} does not apply to shape=\"{shape}\"");
            }
        }
        Ok(())
    };
    let size3 = |default: [f64; 3]| -> Result<[f64; 3]> {
        let v = vector(node, "size", default, MAX_COORDINATE)?;
        if v.iter().any(|&s| s <= 0.0) {
            bail!("size values must be greater than 0");
        }
        Ok(v)
    };
    let segments = || -> Result<u32> {
        let v = scalar(node, "segments", 16.0, (3.0, 128.0))?;
        if v.fract() != 0.0 || !SEGMENTS.contains(&(v as u32)) {
            bail!("segments must be a whole number from 3 to 128");
        }
        Ok(v as u32)
    };
    let (mesh, name) = match shape {
        "plane" => {
            used(&["size"])?;
            let raw = attr(node, "size");
            let [width, depth] = match &raw {
                None => [200.0, 200.0],
                Some(raw) => {
                    let values = numbers(raw, "size")?;
                    let [w, d] = values[..] else {
                        bail!("a plane's size takes two numbers \"width depth\" (got \"{raw}\")");
                    };
                    if w <= 0.0 || d <= 0.0 || w > MAX_COORDINATE || d > MAX_COORDINATE {
                        bail!("size values must be between 0 and {MAX_COORDINATE}");
                    }
                    [w, d]
                }
            };
            (Primitive::Plane { width, depth }.mesh(), "plane")
        }
        "box" => {
            used(&["size"])?;
            (
                Primitive::Box {
                    size: size3([100.0; 3])?,
                }
                .mesh(),
                "box",
            )
        }
        "pyramid" => {
            used(&["size"])?;
            (
                Primitive::Pyramid {
                    size: size3([100.0; 3])?,
                }
                .mesh(),
                "pyramid",
            )
        }
        "cone" | "cylinder" => {
            used(&["radius", "height", "segments"])?;
            let (radius, height, segments) = (
                positive(node, "radius", 50.0)?,
                positive(node, "height", 100.0)?,
                segments()?,
            );
            if shape == "cone" {
                let p = Primitive::Cone {
                    radius,
                    height,
                    segments,
                };
                (p.mesh(), "cone")
            } else {
                let p = Primitive::Cylinder {
                    radius,
                    height,
                    segments,
                };
                (p.mesh(), "cylinder")
            }
        }
        "sphere" => {
            used(&["radius", "detail"])?;
            let detail = scalar(node, "detail", 1.0, (0.0, 4.0))?;
            if detail.fract() != 0.0 || !DETAIL.contains(&(detail as u32)) {
                bail!("detail must be a whole number from 0 to 4");
            }
            let p = Primitive::Sphere {
                radius: positive(node, "radius", 50.0)?,
                detail: detail as u32,
            };
            (p.mesh(), "sphere")
        }
        "custom" => {
            used(&["vertices", "indices", "uvs"])?;
            let vertices = numbers(
                &attr(node, "vertices")
                    .ok_or_else(|| anyhow!("shape=\"custom\" needs vertices"))?,
                "vertices",
            )?;
            let indices = numbers(
                &attr(node, "indices").ok_or_else(|| anyhow!("shape=\"custom\" needs indices"))?,
                "indices",
            )?;
            if vertices.len() % 3 != 0 {
                bail!(
                    "vertices must be a list of x y z triples ({} numbers)",
                    vertices.len()
                );
            }
            if indices.len() % 3 != 0 {
                bail!(
                    "indices must be a list of triangle triples ({} numbers)",
                    indices.len()
                );
            }
            if indices
                .iter()
                .any(|&i| i < 0.0 || i.fract() != 0.0 || i > f64::from(u32::MAX))
            {
                bail!("indices must be whole numbers from 0");
            }
            if indices.len() / 3 > MAX_MESH_TRIANGLES {
                bail!("a mesh may have at most {MAX_MESH_TRIANGLES} triangles");
            }
            let positions = vertices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| c.map(|v| v as f32))
                .collect();
            let triangles = indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| c.map(|v| v as u32))
                .collect();
            let mut mesh = Mesh::custom(positions, triangles)?;
            if let Some(uvs) = look::uvs(node)? {
                mesh = mesh.with_uvs(uvs)?;
            }
            (mesh, "custom")
        }
        other => bail!(
            "unknown shape \"{other}\"; use plane, box, pyramid, cone, cylinder, sphere, or custom"
        ),
    };
    *total += mesh.triangles.len();
    if *total > MAX_TRIANGLES {
        bail!("a data-hf-3d scene may hold at most {MAX_TRIANGLES} triangles; lower segments or detail");
    }
    let shading = match attr(node, "material").as_deref().map(str::trim) {
        None | Some("lambert") => Shading::Lambert,
        Some("unlit") => Shading::Unlit,
        Some(other) => {
            bail!("material must be lambert or unlit (got \"{other}\"); PBR is not supported")
        }
    };
    let cull = match attr(node, "cull").as_deref().map(str::trim) {
        None | Some("back") => Cull::Back,
        Some("none") => Cull::None,
        Some(other) => bail!("cull must be back or none (got \"{other}\")"),
    };
    let material = Material {
        shading,
        color: color(
            node,
            Color {
                r: 200,
                g: 200,
                b: 200,
                a: 255,
            },
            vars,
        )?,
        cull,
        specular: scalar(node, "specular", 0.0, (0.0, 1.0))?,
        shininess: scalar(node, "shininess", 32.0, (1.0, 512.0))?,
        look: None,
    };
    if shading == Shading::Unlit && attr(node, "specular").is_some() {
        bail!("specular does not apply to material=\"unlit\"");
    }
    let look = look::parse(node, context, keyed)?;
    let mut mesh = mesh;
    if let Some(look) = &look {
        if look.needs_uvs() && mesh.uvs.is_empty() {
            bail!("textures and the {} shader read UVs: give the custom mesh uvs=\"u v, ...\", one pair per vertex", look.shader.as_ref().map_or("chosen", |s| s.kind.name()));
        }
        if shading == Shading::Unlit
            && look
                .shader
                .as_ref()
                .is_some_and(|s| s.kind == crate::scene3d::look::ShaderKind::Toon)
        {
            bail!("shader=\"toon\" bands the lighting, so it needs material=\"lambert\"");
        }
        if look.smooth {
            mesh.smooth_normals();
        }
    }
    Ok((
        NodeKind::Mesh {
            mesh: Arc::new(mesh),
            material,
            shape: name,
        },
        look,
    ))
}

fn light(node: &NodeRef, vars: Option<&Vars>) -> Result<Light> {
    let kind = match attr(node, "type").as_deref().map(str::trim) {
        Some("ambient") => {
            if attr(node, "direction").is_some() {
                bail!("direction applies to type=\"directional\" lights only");
            }
            LightKind::Ambient
        }
        Some("directional") => {
            let direction = vector(node, "direction", [0.0, 1.0, 0.0], MAX_COORDINATE)?;
            if direction.iter().all(|&v| v == 0.0) {
                bail!("direction must not be 0 0 0");
            }
            LightKind::Directional { direction }
        }
        Some("point") => {
            if attr(node, "direction").is_some() {
                bail!("direction applies to type=\"directional\" lights only");
            }
            LightKind::Point {
                range: scalar(node, "range", 1000.0, (1.0, MAX_COORDINATE))?,
            }
        }
        Some(other) => bail!("unknown light type \"{other}\"; use ambient, directional, or point (spot is not supported)"),
        None => bail!("<hf-light> needs type=\"ambient\", \"directional\", or \"point\""),
    };
    if !matches!(kind, LightKind::Point { .. }) && attr(node, "range").is_some() {
        bail!("range applies to type=\"point\" lights only");
    }
    let shadows = match attr(node, "shadows").as_deref().map(str::trim) {
        None | Some("false") => false,
        Some("" | "true") => true,
        Some(other) => bail!("shadows must be true or false (got \"{other}\")"),
    };
    if shadows && matches!(kind, LightKind::Ambient) {
        bail!("shadows does not apply to ambient lights");
    }
    let intensity = scalar(node, "intensity", 1.0, (0.0, 16.0))?;
    let keyed = keyframes(node, &[NODE_CHANNELS.as_slice(), &["intensity"]].concat())?;
    Ok(Light {
        kind,
        color: color(node, Color::WHITE, vars)?,
        intensity: keyed.channel("intensity", intensity, (0.0, 16.0))?,
        shadows,
    })
}

fn camera(node: &NodeRef) -> Result<(CameraSpec, Transform, Option<String>)> {
    let projection = match attr(node, "projection").as_deref().map(str::trim) {
        None | Some("perspective") => {
            let focal = match (attr(node, "perspective"), attr(node, "fov")) {
                (Some(_), Some(_)) => bail!("use perspective or fov, not both"),
                (Some(_), None) => scalar(node, "perspective", DEFAULT_FOCAL, (50.0, 100_000.0))?,
                (None, Some(_)) => {
                    return camera_rest(
                        node,
                        Projection::FieldOfView {
                            degrees: scalar(node, "fov", 50.0, (1.0, 170.0))?,
                        },
                    )
                }
                (None, None) => DEFAULT_FOCAL,
            };
            Projection::Perspective { focal }
        }
        Some("orthographic") => {
            if attr(node, "perspective").is_some() || attr(node, "fov").is_some() {
                bail!("perspective and fov only apply to projection=\"perspective\"");
            }
            Projection::Orthographic
        }
        Some(other) => bail!("projection must be perspective or orthographic (got \"{other}\")"),
    };
    camera_rest(node, projection)
}

fn camera_rest(
    node: &NodeRef,
    projection: Projection,
) -> Result<(CameraSpec, Transform, Option<String>)> {
    let near = scalar(node, "near", DEFAULT_NEAR, (0.01, MAX_COORDINATE))?;
    let far = scalar(
        node,
        "far",
        DEFAULT_FAR.max(near * 2.0),
        (0.02, 10.0 * MAX_COORDINATE),
    )?;
    if far <= near * 1.0001 {
        bail!("far ({far}) must be greater than near ({near})");
    }
    let keyed = keyframes(node, &CAMERA_CHANNELS)?;
    let default_z = match projection {
        Projection::Perspective { focal } => focal,
        _ => DEFAULT_FOCAL,
    };
    let position = vector(node, "position", [0.0, 0.0, default_z], MAX_COORDINATE)?;
    let mut transform = Transform::default();
    for (i, name) in ["x", "y", "z"].iter().enumerate() {
        transform.position[i] =
            keyed.channel(name, position[i], (-MAX_COORDINATE, MAX_COORDINATE))?;
    }
    let mut orientation = std::array::from_fn(|_| Channel::constant(0.0));
    for (i, name) in ["yaw", "pitch", "roll"].iter().enumerate() {
        let value = scalar(node, name, 0.0, (-MAX_ANGLE, MAX_ANGLE))?;
        orientation[i] = keyed.channel(name, value, (-MAX_ANGLE, MAX_ANGLE))?;
    }
    let zoom = scalar(node, "zoom", 1.0, (0.05, 50.0))?;
    let zoom = keyed.channel("zoom", zoom, (0.05, 50.0))?;
    let (target, target_node) = match attr(node, "target") {
        None => (None, None),
        Some(raw) if raw.trim().starts_with('#') => (None, Some(raw.trim()[1..].to_string())),
        Some(raw) => {
            let values = numbers(&raw, "target")?;
            let [x, y, z] = values[..] else {
                bail!("target takes \"#node-id\" or three numbers \"x y z\" (got \"{raw}\")");
            };
            (Some(Target::Point([x, y, z])), None)
        }
    };
    Ok((
        CameraSpec {
            projection,
            zoom,
            orientation,
            near,
            far,
            target,
        },
        transform,
        target_node,
    ))
}

/// Static attributes and keyframed channels of a group, mesh, or light.
fn transform(node: &NodeRef, channels: &[&str]) -> Result<Transform> {
    let keyed = keyframes(node, channels)?;
    let position = vector(node, "position", [0.0; 3], MAX_COORDINATE)?;
    let rotation = vector(node, "rotation", [0.0; 3], MAX_ANGLE)?;
    let scale = match attr(node, "scale") {
        None => [1.0; 3],
        Some(raw) => {
            let values = numbers(&raw, "scale")?;
            match values[..] {
                [s] => [s; 3],
                [x, y, z] => [x, y, z],
                _ => bail!("scale takes one number or three \"x y z\" (got \"{raw}\")"),
            }
        }
    };
    if scale.iter().any(|s| s.abs() > MAX_SCALE) {
        bail!("scale values must be within {MAX_SCALE}");
    }
    let mut t = Transform::default();
    for (i, name) in ["x", "y", "z"].iter().enumerate() {
        t.position[i] = keyed.channel(name, position[i], (-MAX_COORDINATE, MAX_COORDINATE))?;
    }
    for (i, name) in ["rotateX", "rotateY", "rotate"].iter().enumerate() {
        t.rotation[i] = keyed.channel(name, rotation[i], (-MAX_ANGLE, MAX_ANGLE))?;
    }
    for (i, name) in ["scaleX", "scaleY", "scaleZ"].iter().enumerate() {
        t.scale[i] = keyed.channel(name, scale[i], (-MAX_SCALE, MAX_SCALE))?;
    }
    Ok(t)
}

/// Parsed `data-hf-keyframes` of one scene node.
struct Keyed {
    frames: Vec<(f64, Interp, HashMap<String, f64>)>,
}

fn keyframes(node: &NodeRef, channels: &[&str]) -> Result<Keyed> {
    let Some(raw) = attr(node, "data-hf-keyframes") else {
        return Ok(Keyed { frames: Vec::new() });
    };
    let parse = || -> Result<Keyed> {
        let value: serde_json::Value = serde_json::from_str(&raw).context("invalid JSON")?;
        let list = value
            .as_array()
            .ok_or_else(|| anyhow!("must be a JSON array of keyframes"))?;
        if list.is_empty() {
            bail!("needs at least one keyframe");
        }
        let mut frames = Vec::with_capacity(list.len());
        let mut last = f64::NEG_INFINITY;
        for entry in list {
            let object = entry
                .as_object()
                .ok_or_else(|| anyhow!("each keyframe must be an object"))?;
            let at = object
                .get("at")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| anyhow!("each keyframe needs a numeric at"))?;
            if !(0.0..=3600.0).contains(&at) {
                bail!("at must be between 0 and 3600 seconds");
            }
            if at <= last {
                bail!("keyframe times must be strictly increasing");
            }
            last = at;
            let mut easing = Interp::Eased(crate::model::Easing::Linear);
            let mut values = HashMap::new();
            for (key, value) in object {
                match key.as_str() {
                    "at" => {}
                    "easing" => {
                        let name = value
                            .as_str()
                            .ok_or_else(|| anyhow!("easing must be a string"))?;
                        if channels == KINEMATIC_CHANNELS && !matches!(name, "linear" | "smooth") {
                            bail!("kinematic body easing must be linear or smooth");
                        }
                        easing = parse_camera_easing(name)?;
                    }
                    name if channels.contains(&name) => {
                        let v = value
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .ok_or_else(|| anyhow!("{name} must be a number"))?;
                        values.insert(name.to_string(), v);
                    }
                    other => bail!(
                        "unknown keyframe channel {other}; this node animates {}",
                        channels.join(", ")
                    ),
                }
            }
            if let Some(&s) = values.get("scale") {
                for axis in ["scaleX", "scaleY", "scaleZ"] {
                    values.entry(axis.to_string()).or_insert(s);
                }
            }
            frames.push((at, easing, values));
        }
        Ok(Keyed { frames })
    };
    parse().context("data-hf-keyframes")
}

impl Keyed {
    /// Whether any keyframe animates `name`.
    fn names(&self, name: &str) -> bool {
        self.frames.iter().any(|f| f.2.contains_key(name))
    }

    /// The channel `name`: constant `base` when no keyframe names it, otherwise a key at every
    /// keyframe (omitted values inherit the previous one, `base` before the first).
    fn channel(&self, name: &str, base: f64, range: (f64, f64)) -> Result<Channel> {
        if !self.frames.iter().any(|f| f.2.contains_key(name)) {
            return Ok(Channel::constant(base));
        }
        let mut value = base;
        let mut points = Vec::with_capacity(self.frames.len());
        for (at, easing, values) in &self.frames {
            if let Some(&v) = values.get(name) {
                if v < range.0 || v > range.1 {
                    bail!(
                        "keyframe {name} must be between {} and {} (got {v})",
                        range.0,
                        range.1
                    );
                }
                value = v;
            }
            points.push((*at, value, *easing));
        }
        Ok(Channel::keyed(&points))
    }
}
