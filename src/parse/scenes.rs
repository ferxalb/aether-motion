//! Loads, validates, and flattens external scene instances into the native composition model.

use super::html::{attr, direct_text};
use super::split::SplitPlan;
use super::tree::Pending;
use super::{parse_pass, values_from_json, VariableValues};
use crate::model::{AudioClip, Composition, Element, Length};
use anyhow::{anyhow, bail, Context, Result};
use kuchiki::NodeRef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_DEPTH: usize = 8;
const MAX_INSTANCES: usize = 256;

#[derive(Default)]
pub(super) struct SceneStack {
    active: Vec<PathBuf>,
    instances: usize,
}

impl SceneStack {
    pub(super) fn parse(
        &mut self,
        path: &Path,
        values: &VariableValues,
        splits: &SplitPlan,
    ) -> Result<Composition> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("could not read {}", path.display()))?;
        if self.active.contains(&canonical) {
            bail!(
                "cycle in data-composition-src: {} is already active",
                path.display()
            );
        }
        if self.active.len() > MAX_DEPTH {
            bail!("data-composition-src supports at most {MAX_DEPTH} nested levels");
        }
        if !self.active.is_empty() {
            self.instances += 1;
            if self.instances > MAX_INSTANCES {
                bail!("data-composition-src supports at most {MAX_INSTANCES} instances per composition");
            }
        }
        let imported = !self.active.is_empty();
        self.active.push(canonical);
        let result = parse_pass(path, values, splits, self, imported);
        self.active.pop();
        result
    }
}

pub(super) struct Instance {
    host: usize,
    scene: Composition,
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
                if path.is_absolute()
                    || src.contains([':', '\\', '?', '#'])
                    || !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
                    })
                {
                    bail!("use a relative local .html or .htm path for data-composition-src");
                }
                let values = attr(&node, "data-variable-values")
                    .map(|raw| values_from_json(&raw).context("data-variable-values"))
                    .transpose()?
                    .unwrap_or_default();
                let scene = stack.parse(&base.join(path), &values, splits)?;
                if (scene.width, scene.height) != size {
                    bail!("scene dimensions must match the containing composition ({}x{}); found {}x{}", size.0, size.1, scene.width, scene.height);
                }
                let duration = item.duration.unwrap_or(scene.duration);
                if duration <= 0.0 || duration > scene.duration {
                    bail!("set data-duration above 0 and at most the scene's duration ({}s)", scene.duration);
                }
                item.duration = Some(duration);
                fill_canvas(&mut item.element, size);
                Ok(Instance { host, scene })
            })()
            .with_context(|| format!("data-composition-src on #{label} ({src:?})"))
        })
        .collect()
}

fn fill_canvas(element: &mut Element, size: (u32, u32)) {
    let style = &mut element.style;
    style.position = Some("absolute".into());
    style.left = None;
    style.top = None;
    style.width = Some(Length::Px(size.0 as f32));
    style.height = Some(Length::Px(size.1 as f32));
    style.max_width = None;
    style.margin = [0.0; 4];
}

pub(super) fn append(
    instances: Vec<Instance>,
    elements: &mut Vec<Element>,
    ids: &mut HashMap<String, usize>,
    audio: &mut Vec<AudioClip>,
) -> Result<()> {
    for Instance { host, mut scene } in instances {
        let offset = elements.len();
        let instance = &elements[host];
        let (start, end) = (instance.start, instance.end);
        let prefix = format!(
            "{}::",
            instance.id.as_deref().expect("validated instance id")
        );
        fill_canvas(&mut scene.elements[0], (scene.width, scene.height));
        scene.elements[0].layout.scene_root = true;
        elements[host].children.push(offset);
        for mut element in scene.elements {
            element.start += start;
            element.end += start;
            element.parent = Some(element.parent.map_or(host, |index| index + offset));
            for child in &mut element.children {
                *child += offset;
            }
            if let Some(rig) = &mut element.motion.rig {
                if rig.subject().is_some() {
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
