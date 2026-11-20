//! Loads, validates, and flattens external scene instances into the native composition model.
//!
//! `cache` compiles each imported document once per set of variable values (and keeps it between
//! `preview --watch` compiles, see `SceneCache`); `fingerprint` identifies the files a kept scene
//! read. `load` validates the instance hosts of one document and `append` flattens their scenes
//! into its element tree, timeline, ids, audio, and dependency list: each instance gets its own
//! ids, tree links, and clip times, and shares the element bodies of the compiled scene. A scene
//! whose canvas differs from its host's is placed by `data-composition-fit` (`contain`, `cover`,
//! or `none`), which gives that instance its own copy of the scene root.

mod cache;
mod fingerprint;

pub use cache::SceneCache;
pub(super) use cache::SceneStack;
pub(in crate::parse) use fingerprint::Fingerprint;

use super::html::{attr, direct_text};
use super::split::SplitPlan;
use super::tree::Pending;
use super::values_from_json;
use crate::model::{AudioClip, Composition, Content, Element, Length};
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Flattened elements a document may hold with its imported scenes; reuse makes scenes cheap to
/// compile, so this bounds the size of the expanded tree instead.
pub(super) const MAX_ELEMENTS: usize = 200_000;

/// Where a scene root sits in its host when the canvases differ: its top-left offset at rest and
/// its uniform scale around its center.
#[derive(Clone, Copy, Debug)]
struct Placement {
    left: f32,
    top: f32,
    scale: f32,
}

pub(super) struct Instance {
    host: usize,
    scene: Composition,
    placement: Option<Placement>,
}

/// The containing document a scene is imported into: its parsed tree, composition root, the
/// directory its relative paths resolve from, and its canvas size.
pub(super) struct Host<'a> {
    pub document: &'a NodeRef,
    pub root: &'a NodeRef,
    pub base: &'a Path,
    pub size: (u32, u32),
}

pub(super) fn load(
    host: &Host<'_>,
    pending: &mut [Pending],
    ids: &HashMap<String, usize>,
    splits: &SplitPlan,
    stack: &mut SceneStack,
) -> Result<Vec<Instance>> {
    let Host {
        document,
        root,
        base,
        size,
    } = *host;
    let mut flattened = pending.len();
    document
        .descendants()
        .filter_map(|node| attr(&node, "data-composition-src").map(|src| (node, src)))
        .map(|(node, src)| {
            let id = attr(&node, "id").filter(|id| !id.trim().is_empty());
            let label = id.as_deref().unwrap_or("no id");
            (|| {
                let tag = &*node.as_element().expect("source is an attribute").name.local;
                if node.parent().as_ref() != Some(root)
                    || !matches!(tag, "div" | "section" | "article" | "main")
                    || node.children().any(|child| child.as_element().is_some())
                    || !direct_text(&node).is_empty()
                {
                    bail!("use an empty div, section, article, or main directly inside the composition root");
                }
                let host = *ids
                    .get(label)
                    .filter(|_| id.is_some())
                    .ok_or_else(|| anyhow!("give the scene instance a unique id"))?;
                let item = &mut pending[host];
                if item.start_expr.is_none() || attr(&node, "data-track-index").is_none() {
                    bail!("set data-start and data-track-index on the scene instance");
                }
                let src = src.trim();
                let path = Path::new(src);
                // `has_root` also rejects `/x.html` on Windows, where it is not absolute (no drive).
                if path.is_absolute()
                    || path.has_root()
                    || src.contains([':', '\\', '?', '#'])
                    || !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
                    })
                {
                    bail!("use a relative local .html or .htm path for data-composition-src");
                }
                let fit = attr(&node, "data-composition-fit");
                let values = attr(&node, "data-variable-values")
                    .map(|raw| values_from_json(&raw).context("data-variable-values"))
                    .transpose()?
                    .unwrap_or_default();
                let scene = stack.parse(&base.join(path), &values, splits)?;
                let placement = placement(fit.as_deref(), (scene.width, scene.height), size)?;
                let duration = item.duration.unwrap_or(scene.duration);
                if duration <= 0.0 || duration > scene.duration {
                    bail!("set data-duration above 0 and at most the scene's duration ({}s)", scene.duration);
                }
                flattened += scene.elements.len();
                if flattened > MAX_ELEMENTS {
                    bail!("the composition and its scenes flatten to more than {MAX_ELEMENTS} elements; reuse fewer instances or split the film into separate renders");
                }
                item.duration = Some(duration);
                fill_canvas(&mut item.element, size);
                Ok(Instance { host, scene, placement })
            })()
            .with_context(|| format!("data-composition-src on #{label} ({src:?})"))
        })
        .collect()
}

/// `data-composition-fit` for a scene of canvas `scene` inside a host canvas `host`: `None` when
/// the canvases match (the scene fills its host exactly, as in v1).
fn placement(fit: Option<&str>, scene: (u32, u32), host: (u32, u32)) -> Result<Option<Placement>> {
    let [w, h, hw, hh] = [scene.0, scene.1, host.0, host.1].map(|v| v as f32);
    let (sx, sy) = (hw / w, hh / h);
    let scale = match fit.map(str::trim) {
        Some("contain") => sx.min(sy),
        Some("cover") => sx.max(sy),
        Some("none") => 1.0,
        Some("fill") => bail!("data-composition-fit=\"fill\" would scale unevenly, which is unsupported; use contain, cover, or none"),
        Some(other) => bail!("data-composition-fit must be contain, cover, or none, not {other:?}"),
        None if scene == host => return Ok(None),
        None => bail!(
            "the scene is {}x{} but the containing composition is {}x{}; set data-composition-fit to contain, cover, or none",
            scene.0, scene.1, host.0, host.1
        ),
    };
    if scene == host {
        return Ok(None);
    }
    Ok(Some(Placement {
        left: (hw - w) / 2.0,
        top: (hh - h) / 2.0,
        scale,
    }))
}

pub(super) fn fill_canvas(element: &mut Element, size: (u32, u32)) {
    let style = &mut element.style;
    style.position = Some("absolute".into());
    style.left = None;
    style.top = None;
    style.width = Some(Length::Px(size.0 as f32));
    style.height = Some(Length::Px(size.1 as f32));
    style.max_width = None;
    style.margin = [0.0; 4];
}

/// Files the elements and audio of one document read (its own document and transcripts are added
/// by the caller).
pub(super) fn asset_dependencies(elements: &[Element], audio: &[AudioClip]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for element in elements {
        match &element.content {
            Content::Image(path) => files.push(path.clone()),
            Content::Video(video) => files.push(video.path.clone()),
            Content::Sequence(sequence) => {
                files.extend(sequence.files.iter().cloned());
                if let Some(folder) = sequence.files.first().and_then(|f| f.parent()) {
                    files.push(folder.to_path_buf());
                }
            }
            _ => {}
        }
    }
    files.extend(audio.iter().map(|clip| clip.src.clone()));
    files
}

pub(super) fn append(
    instances: Vec<Instance>,
    elements: &mut Vec<Element>,
    ids: &mut HashMap<String, usize>,
    audio: &mut Vec<AudioClip>,
    dependencies: &mut Vec<PathBuf>,
) -> Result<()> {
    for Instance {
        host,
        mut scene,
        placement,
    } in instances
    {
        let offset = elements.len();
        let instance = &elements[host];
        let (start, end) = (instance.start, instance.end);
        let prefix = format!(
            "{}::",
            instance.id.as_deref().expect("validated instance id")
        );
        dependencies.append(&mut scene.dependencies);
        // The compiled root already fills its own canvas; a fitted instance copies and places it.
        if let Some(Placement { left, top, scale }) = placement {
            let root = &mut scene.elements[0];
            root.style.left = Some(Length::Px(left));
            root.style.top = Some(Length::Px(top));
            root.motion.base.scale = scale;
        }
        elements[host].children.push(offset);
        for mut element in scene.elements {
            element.start += start;
            element.end += start;
            element.parent = Some(element.parent.map_or(host, |index| index + offset));
            for child in &mut element.children {
                *child += offset;
            }
            // A camera that follows a subject copies its body to point at this instance's subject.
            if element
                .motion
                .rig
                .as_ref()
                .is_some_and(|rig| rig.subject().is_some())
            {
                if let Some(rig) = &mut element.motion.rig {
                    Arc::make_mut(rig).offset_subject(offset);
                }
            }
            if let Some(id) = &mut element.id {
                *id = format!("{prefix}{id}");
                if ids.insert(id.clone(), elements.len()).is_some() {
                    bail!("duplicate scoped scene id: {id}");
                }
            }
            elements.push(element);
        }
        let audio_offset = audio.len();
        for mut clip in scene.audio {
            clip.start += start;
            clip.scene_end = Some(clip.scene_end.map_or(end, |local| (start + local).min(end)));
            if let Some(id) = &mut clip.id {
                *id = format!("{prefix}{id}");
            }
            if let Some(duck) = &mut clip.effects.duck {
                for id in &mut duck.by {
                    *id = format!("{prefix}{id}");
                }
                for source in &mut duck.sources {
                    *source += audio_offset;
                }
            }
            audio.push(clip);
        }
    }
    Ok(())
}
