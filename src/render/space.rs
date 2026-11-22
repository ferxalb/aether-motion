//! Drawing of 2.5D spaces: composes each plane's 3D transform down the element tree, orders the
//! planes exactly (splitting intersecting ones), and draws each piece through the normal element
//! pipeline or a projective warp.
//!
//! A stage is a `data-hf-space` container, or an element inside a space that draws its subtree on
//! a layer of its own (`overflow: hidden`, `clip-path`, `mask-image`, `filter`, a blend mode, a
//! reveal, bloom, pixel effects, or `data-hf-motion-blur`) and holds planes with 3D values. Such an
//! element flattens its subtree, as CSS does: its planes form a nested space with the enclosing
//! space's projection, drawn into it, and the element itself stays a plane (or part of one) of the
//! enclosing space. A nested `data-hf-space` is the same, with its own projection and camera.
//!
//! A stage's planes are its direct children and every member with 3D values of its own; other
//! members are drawn flat inside their nearest plane, exactly as in 2D. [`SpaceIndex`] measures
//! the resting layout of each stage once per renderer (the renderer's own placement at scale 1):
//! each plane's offset from its parent, its size, the extent its flat descendants may paint, and
//! the paint order (`data-track-index`, `z-index`, and document order) that breaks ties. Each
//! frame, [`Renderer::draw_space`] composes the transforms from the frame's element states (with
//! `data-hf-parallax` shifts of camera children), orders the visible planes with a BSP tree
//! (`crate::space::bsp`), and draws the fragments back to front: a whole plane facing an unturned
//! camera is a scaled and rolled 2D draw with every effect and the exact 2D sampling; tilted planes,
//! planes seen by a turned camera, and split pieces are warped from one layer per plane
//! (`paint/project.rs`). A plane with `data-hf-motion-blur` is placed again at every shutter
//! sample. Everything depends only on the element states, so frames match in any order, thread,
//! or render part.

use super::geometry::Rect;
use super::layout::in_flow;
use super::paint::{PlaneLayer, PlaneSetup};
use super::Renderer;
use crate::model::{BlendMode, Composition, Content, Element};
use crate::space::bsp::{self, Eye, Polygon};
use crate::space::math::{Affine, View};
use crate::space::Projection;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

/// One element whose transform a stage composes: a plane, or an ancestor of one below the stage.
#[derive(Clone)]
struct Node {
    idx: usize,
    /// Position in [`StageLayout::nodes`] of the parent (`None`: the stage itself).
    parent: Option<usize>,
    /// Resting center relative to the parent's resting center (composition px at scale 1).
    offset: [f64; 2],
    /// Resting border-box size (composition px at scale 1).
    size: [f32; 2],
    plane: bool,
    /// `data-hf-parallax` of this node when its parent is a camera that pans its children.
    parallax: Option<f32>,
    /// What the plane and its flat descendants may paint (local px from its center).
    extent: [f64; 4],
    /// Whether the plane or a flat descendant holds images, video, vector art, or text.
    sized_content: bool,
}

/// Resting layout of one stage.
struct StageLayout {
    projection: Projection,
    near: f64,
    /// Nodes in paint order, so every parent precedes its children.
    nodes: Vec<Node>,
}

/// The planes of every stage, measured once per renderer.
#[derive(Default)]
pub(in crate::render) struct SpaceIndex {
    /// Whether each element is a plane (drawn by its stage rather than by its parent).
    planes: Vec<bool>,
    stages: HashMap<usize, Arc<StageLayout>>,
}

/// Transform of one node this frame.
#[derive(Clone, Copy)]
struct Placed {
    world: Affine,
    /// Whether this node or an ancestor below the stage tilts out of the stage plane.
    tilted: bool,
    /// Sum of the 2D rotations (degrees) down the chain; the plane's roll while it faces the camera.
    roll: f32,
    /// Product of the 2D scales down the chain.
    scale: f64,
}

const ROOT: Placed = Placed {
    world: Affine::IDENTITY,
    tilted: false,
    roll: 0.0,
    scale: 1.0,
};

/// Whether an element draws its subtree on an offscreen layer of its own, so planes inside it are
/// flattened into it.
pub(in crate::render) fn flattens(element: &Element) -> bool {
    let style = &element.style;
    let motion = &element.motion;
    style.overflow_hidden
        || style.clip_path.is_some()
        || style.mask.is_some()
        || style.filter_blur > 0.0
        || !style.filter_colors.is_empty()
        || style.drop_shadow.is_some()
        || style.blend != BlendMode::Normal
        || !motion.filter.is_static()
        || !motion.fx.is_static()
        || motion.wipe.is_some()
        || element.effects.bloom.is_some()
        || element.effects.motion_blur.is_some()
}

/// How far (local px) an element may paint outside its box: shadows, blur, and glow, plus (with
/// `moving`) its own translation away from its resting place.
fn reach(element: &Element, moving: bool) -> f64 {
    let style = &element.style;
    let mut reach = 0.0_f32;
    for s in style.shadows.iter().filter(|s| !s.inset) {
        reach = reach.max(s.x.abs().max(s.y.abs()) + s.spread.max(0.0) + s.blur * 1.5);
    }
    for s in style.text_shadow.iter().flatten() {
        reach = reach.max(s.x.abs().max(s.y.abs()) + s.blur * 1.5);
    }
    if let Some(s) = style.drop_shadow {
        reach = reach.max(s.x.abs().max(s.y.abs()) + s.blur * 1.5);
    }
    let motion = &element.motion;
    let blur = style.filter_blur.max(motion.filter.base[0]).max(
        motion.filter.tracks[0]
            .0
            .iter()
            .fold(0.0_f32, |high, p| high.max(p.value)),
    );
    reach += blur * 3.0;
    if let Some(bloom) = element.effects.bloom {
        reach += bloom.radius * 1.5;
    }
    let travel = |track: &crate::model::Track, rest: f32| {
        extremes(track, rest).map_or(0.0, |(low, high)| {
            (low - rest).abs().max((high - rest).abs())
        })
    };
    if !moving {
        return f64::from(reach);
    }
    let mut motion_reach = travel(&motion.x, motion.base.x).max(travel(&motion.y, motion.base.y));
    if let Some(path) = &motion.path {
        motion_reach += path.measure.length();
    }
    f64::from(reach + motion_reach)
}

/// Lowest and highest values a track can reach, including the overshoot of springs and `back`
/// easings (at most 0.6 of a segment's change past either end), or `None` for an empty track.
fn extremes(track: &crate::model::Track, rest: f32) -> Option<(f32, f32)> {
    const OVERSHOOT: f32 = 0.6;
    let mut previous = track.0.first()?.value;
    let (mut low, mut high) = (rest.min(previous), rest.max(previous));
    for point in &track.0 {
        let swing = (point.value - previous).abs() * OVERSHOOT;
        low = low.min(point.value.min(previous) - swing);
        high = high.max(point.value.max(previous) + swing);
        previous = point.value;
    }
    Some((low, high))
}

/// The largest scale an element can reach (its resting scale, or its animated scale with
/// overshoot).
fn largest_scale(element: &Element) -> f32 {
    let motion = &element.motion;
    let rest = motion.base.scale.abs();
    extremes(&motion.scale, motion.base.scale)
        .map_or(rest, |(low, high)| rest.max(low.abs()).max(high.abs()))
}

impl SpaceIndex {
    pub(in crate::render) fn build(renderer: &mut Renderer<'_>) -> Result<Self> {
        let composition: &Composition = renderer.composition;
        let elements = &composition.elements;
        let n = elements.len();
        if !elements.iter().any(|e| e.depth.stage.is_some()) {
            return Ok(Self::default());
        }
        // The nearest `data-hf-space` above each element (whose projection nested stages inherit).
        let mut space: Vec<Option<usize>> = vec![None; n];
        for idx in 0..n {
            space[idx] = elements[idx].parent.and_then(|p| {
                if elements[p].depth.stage.is_some() {
                    Some(p)
                } else {
                    space[p]
                }
            });
        }
        // Whether each element has a descendant with 3D values of its own, below any nested space.
        let mut planes_below = vec![false; n];
        for idx in (0..n).rev() {
            if let Some(p) = elements[idx].parent {
                let counts = elements[idx].depth.plane.is_some()
                    || (planes_below[idx] && elements[idx].depth.stage.is_none());
                planes_below[p] |= counts;
            }
        }
        let is_stage: Vec<bool> = (0..n)
            .map(|idx| {
                let element = &elements[idx];
                element.depth.stage.is_some()
                    || (space[idx].is_some() && planes_below[idx] && flattens(element))
            })
            .collect();
        let mut index = Self {
            planes: vec![false; n],
            stages: HashMap::new(),
        };
        for stage in (0..n).filter(|&i| is_stage[i]) {
            let projection_source = if elements[stage].depth.stage.is_some() {
                stage
            } else {
                space[stage].expect("implicit stages are inside a space")
            };
            let definition = elements[projection_source]
                .depth
                .stage
                .as_ref()
                .expect("a data-hf-space");
            let layout = index.measure(renderer, stage, &is_stage)?;
            index.stages.insert(
                stage,
                Arc::new(StageLayout {
                    projection: definition.projection,
                    near: definition.near(),
                    nodes: layout,
                }),
            );
        }
        Ok(index)
    }

    /// Measures the nodes of `stage` and marks its planes.
    fn measure(
        &mut self,
        renderer: &mut Renderer<'_>,
        stage: usize,
        is_stage: &[bool],
    ) -> Result<Vec<Node>> {
        let composition: &Composition = renderer.composition;
        let elements = &composition.elements;
        // Members in paint order with their resting boxes; nested stages are members, but their
        // subtrees are not.
        let stage_rect = renderer.rest_rect(stage)?;
        let mut members: Vec<(usize, Rect)> = Vec::new();
        let mut stack: Vec<(usize, Rect)> = vec![(stage, stage_rect)];
        while let Some((idx, rect)) = stack.pop() {
            if idx != stage {
                members.push((idx, rect));
                if is_stage[idx] {
                    continue;
                }
            }
            let flex = if elements[idx].style.display.as_deref() == Some("flex") {
                let content = renderer.content_box(idx, rect, 1.0);
                renderer.flex_layout(idx, content, 1.0)?
            } else {
                Vec::new()
            };
            let children = renderer.children[idx].clone();
            for &child in children.iter().rev() {
                let slot = in_flow(composition, child)
                    .then(|| flex.iter().find(|(c, _)| *c == child).map(|(_, r)| *r))
                    .flatten();
                stack.push((child, renderer.place(child, rect, 1.0, slot)));
            }
        }
        let rects: HashMap<usize, Rect> = members
            .iter()
            .copied()
            .chain(std::iter::once((stage, stage_rect)))
            .collect();
        let is_plane =
            |idx: usize| elements[idx].parent == Some(stage) || elements[idx].depth.plane.is_some();
        let mut wanted = vec![false; elements.len()];
        for &(idx, _) in &members {
            if is_plane(idx) {
                self.planes[idx] = true;
                let mut node = Some(idx);
                while let Some(n) = node.filter(|&n| n != stage) {
                    wanted[n] = true;
                    node = elements[n].parent;
                }
            }
        }
        // Each member's nearest plane (itself for a plane), for extents and content.
        let mut owner: HashMap<usize, usize> = HashMap::new();
        for &(idx, _) in &members {
            let parent = elements[idx].parent.expect("members have a parent");
            let plane = if is_plane(idx) { idx } else { owner[&parent] };
            owner.insert(idx, plane);
        }
        let center = |r: Rect| {
            [
                f64::from(r.x) + f64::from(r.w) / 2.0,
                f64::from(r.y) + f64::from(r.h) / 2.0,
            ]
        };
        let mut extents: HashMap<usize, [f64; 4]> = HashMap::new();
        let mut sized: HashMap<usize, bool> = HashMap::new();
        for &(idx, rect) in &members {
            let plane = owner[&idx];
            let origin = center(rects[&plane]);
            let element = &elements[idx];
            // A plane's own translation, turn, and scale are part of its transform; its flat
            // descendants move inside its box.
            let own = plane == idx;
            let mut grow = reach(element, !own);
            let scale = largest_scale(element);
            if !own && (element.motion.rotates() || scale != 1.0 || element.motion.rig.is_some()) {
                // A turning or scaled element may sweep its half-diagonal around its center.
                grow += f64::from(rect.w.hypot(rect.h)) * f64::from(scale.max(1.0)) / 2.0;
            }
            let box_extent = [
                f64::from(rect.x) - origin[0] - grow,
                f64::from(rect.y) - origin[1] - grow,
                f64::from(rect.x + rect.w) - origin[0] + grow,
                f64::from(rect.y + rect.h) - origin[1] + grow,
            ];
            let entry = extents.entry(plane).or_insert(box_extent);
            *entry = [
                entry[0].min(box_extent[0]),
                entry[1].min(box_extent[1]),
                entry[2].max(box_extent[2]),
                entry[3].max(box_extent[3]),
            ];
            let holds = !matches!(element.content, Content::Container) || is_stage[idx];
            *sized.entry(plane).or_insert(false) |= holds;
        }
        let mut position: HashMap<usize, usize> = HashMap::new();
        let mut nodes = Vec::new();
        for &(idx, rect) in members.iter().filter(|(idx, _)| wanted[*idx]) {
            let element = &elements[idx];
            let parent_idx = element.parent.expect("members have a parent");
            let (c, p) = (center(rect), center(rects[&parent_idx]));
            let parallax = element
                .layout
                .parallax
                .filter(|&k| k != 1.0 && elements[parent_idx].layout.has_parallax);
            position.insert(idx, nodes.len());
            nodes.push(Node {
                idx,
                parent: position.get(&parent_idx).copied(),
                offset: [c[0] - p[0], c[1] - p[1]],
                size: [rect.w, rect.h],
                plane: self.planes[idx],
                parallax,
                extent: extents.get(&idx).copied().unwrap_or([0.0; 4]),
                sized_content: sized.get(&idx).copied().unwrap_or(false),
            });
        }
        Ok(nodes)
    }
}

impl<'a> Renderer<'a> {
    /// Whether `idx` is a plane of a stage, drawn by [`Self::draw_space`] instead of its parent.
    pub(in crate::render) fn is_space_plane(&self, idx: usize) -> bool {
        self.spaces.planes.get(idx).copied().unwrap_or(false)
    }

    /// Whether `idx` draws its children as a stage (a `data-hf-space`, or an element that
    /// flattens planes).
    pub(in crate::render) fn is_stage(&self, idx: usize) -> bool {
        !self.spaces.stages.is_empty() && self.spaces.stages.contains_key(&idx)
    }

    /// Whether `idx` is a space whose camera keyframes move the 3D camera (its own box then stays
    /// at its resting pose).
    pub(in crate::render) fn is_space_camera(&self, idx: usize) -> bool {
        self.composition.elements[idx]
            .depth
            .stage
            .as_ref()
            .is_some_and(|stage| stage.camera)
    }

    /// The camera of stage `stage`, whose box is drawn at `rect` with scale `cumulative`.
    fn stage_view(&self, stage: usize, layout: &StageLayout, rect: Rect, cumulative: f32) -> View {
        let element = &self.composition.elements[stage];
        let state = self.current[stage];
        let base = element.motion.base;
        let camera = element.depth.stage.as_ref().filter(|s| s.camera);
        let (focus, zoom) = match camera {
            Some(_) if state.scale != 0.0 && base.scale != 0.0 => {
                let scale = f64::from(state.scale);
                (
                    [
                        -f64::from(state.x - base.x) / scale,
                        -f64::from(state.y - base.y) / scale,
                    ],
                    scale / f64::from(base.scale),
                )
            }
            _ => ([0.0; 2], 1.0),
        };
        let [dolly, tilt, pan, roll] = state.camera_3d.map(f64::from);
        View::new(
            layout.projection,
            [
                f64::from(rect.x) + f64::from(rect.w) / 2.0,
                f64::from(rect.y) + f64::from(rect.h) / 2.0,
            ],
            f64::from(cumulative) * zoom,
            [focus[0], focus[1], dolly],
            [tilt, pan, roll],
            layout.near,
        )
    }

    /// The local transform of `node` from its current state, with the parallax shift of a camera
    /// child (`focus` is the stage camera's).
    fn local_transform(
        &self,
        node: &Node,
        stage: usize,
        focus: [f64; 2],
    ) -> (Affine, [f32; 3], f32, f32) {
        let mut s = self.current[node.idx];
        if self.is_space_camera(node.idx) {
            // A nested space's camera moves its own planes, not its box.
            let base = self.composition.elements[node.idx].motion.base;
            (s.x, s.y, s.scale) = (base.x, base.y, base.scale);
        }
        let mut shift = [0.0_f64; 2];
        if let Some(k) = node.parallax {
            let k = f64::from(k) - 1.0;
            let parent = self.composition.elements[node.idx]
                .parent
                .expect("parallax nodes have a parent");
            if parent == stage {
                // The classic camera puts a plane of parallax k at zoom * (p - k * focus).
                shift = [-k * focus[0], -k * focus[1]];
            } else {
                let camera = self.current[parent];
                if camera.scale != 0.0 {
                    shift = camera
                        .camera_pan
                        .map(|pan| f64::from(pan) * k / f64::from(camera.scale));
                }
            }
        }
        let local = Affine::pose(
            [
                node.offset[0] + f64::from(s.x) + shift[0],
                node.offset[1] + f64::from(s.y) + shift[1],
                f64::from(s.depth[0]),
            ],
            f64::from(s.depth[1]),
            f64::from(s.depth[2]),
            f64::from(s.rotate),
            f64::from(s.scale),
        );
        (local, s.depth, s.rotate, s.scale)
    }

    fn place_node(&self, node: &Node, parent: Placed, stage: usize, focus: [f64; 2]) -> Placed {
        let (local, depth, rotate, scale) = self.local_transform(node, stage, focus);
        Placed {
            world: parent.world.then(&local),
            tilted: parent.tilted || depth[1] != 0.0 || depth[2] != 0.0,
            roll: parent.roll + rotate,
            scale: parent.scale * f64::from(scale),
        }
    }

    /// Draws the planes of stage `stage`, whose box is drawn at `rect` with scale `cumulative`.
    pub(in crate::render) fn draw_space(
        &mut self,
        stage: usize,
        rect: Rect,
        cumulative: f32,
    ) -> Result<()> {
        let Some(layout) = self.spaces.stages.get(&stage).cloned() else {
            return Ok(());
        };
        let view = self.stage_view(stage, &layout, rect, cumulative);
        let focus = [view.camera[0], view.camera[1]];
        let mut placed: Vec<Placed> = Vec::with_capacity(layout.nodes.len());
        for node in &layout.nodes {
            let parent = node.parent.map_or(ROOT, |p| placed[p]);
            placed.push(self.place_node(node, parent, stage, focus));
        }
        // Visible planes, in paint order. A plane blurred over a moving shutter window is kept when
        // any sample shows it, and its visibility and facing are decided per sample.
        let mut polygons = Vec::new();
        for (slot, node) in layout.nodes.iter().enumerate() {
            if !node.plane {
                continue;
            }
            let sampled = self.is_layer_blurred(node.idx) && self.blur_moves(node.idx);
            let visible = if sampled {
                self.blur_visible(node.idx)
            } else {
                self.current[node.idx].alpha > 0.0
            };
            if !visible || (!sampled && self.faces_away(node.idx, &placed[slot], &view)) {
                continue;
            }
            polygons.push(Polygon {
                id: slot,
                plane: placed[slot].world,
                bounds: node.extent,
            });
        }
        let eye = match view.eye_point() {
            (p, w) if w > 0.0 => Eye::Point(p),
            (d, _) => Eye::Direction(d),
        };
        let fragments = bsp::order(&polygons, eye).unwrap_or_else(|| {
            // Too many cuts: fall back to the distance of each plane's center.
            let mut sorted: Vec<&Polygon> = polygons.iter().collect();
            sorted.sort_by(|a, b| {
                view.distance(b.plane.t)
                    .total_cmp(&view.distance(a.plane.t))
                    .then(a.id.cmp(&b.id))
            });
            sorted
                .into_iter()
                .map(|p| bsp::Fragment {
                    id: p.id,
                    clips: Vec::new(),
                })
                .collect()
        });
        let mut remaining: HashMap<usize, usize> = HashMap::new();
        for fragment in &fragments {
            *remaining.entry(fragment.id).or_default() += 1;
        }
        let mut layers: HashMap<usize, Option<PlaneLayer>> = HashMap::new();
        for fragment in fragments {
            let slot = fragment.id;
            let node = &layout.nodes[slot];
            let left = remaining.get_mut(&slot).expect("counted");
            *left -= 1;
            let last = *left == 0;
            if self.is_layer_blurred(node.idx) && self.blur_moves(node.idx) {
                // A plane blurred over a moving window is drawn whole, once, placed again at
                // every sample; samples where it is hidden or faces away draw nothing.
                if !fragment.clips.is_empty() && !last {
                    continue;
                }
                let parent = node.parent.map_or(ROOT, |p| placed[p]);
                let node = node.clone();
                let idx = node.idx;
                self.blur_samples(idx, &mut |r: &mut Self| {
                    let at = r.place_node(&node, parent, stage, focus);
                    if r.current[idx].alpha <= 0.0 || r.faces_away(idx, &at, &view) {
                        return Ok(());
                    }
                    r.draw_whole_plane(&node, at, &view)
                })?;
                continue;
            }
            let at = placed[slot];
            if fragment.clips.is_empty() {
                self.draw_whole_plane(node, at, &view)?;
                continue;
            }
            if let std::collections::hash_map::Entry::Vacant(entry) = layers.entry(slot) {
                let setup = self.plane_setup(node, at, &view);
                entry.insert(self.plane_layer(node.idx, setup)?);
            }
            if let Some(Some(layer)) = layers.get_mut(&slot) {
                self.composite_plane(layer, &fragment.clips);
            }
            if last {
                layers.remove(&slot);
            }
        }
        Ok(())
    }

    /// Whether plane `idx`, placed at `at`, hides its back from the camera.
    fn faces_away(&self, idx: usize, at: &Placed, view: &View) -> bool {
        let plane = self.composition.elements[idx].depth.plane.as_ref();
        plane.is_some_and(|plane| plane.backface_hidden)
            && view.facing(at.world.t, at.world.axis(2)) <= 0.0
    }

    fn plane_setup<'v>(&self, node: &Node, at: Placed, view: &'v View) -> PlaneSetup<'v> {
        PlaneSetup {
            plane: at.world,
            size: node.size,
            extent: node.extent,
            sized_content: node.sized_content,
            view,
        }
    }

    /// Draws a whole plane placed at `at`: a 2D draw when it faces an unturned camera, otherwise a
    /// projective warp.
    fn draw_whole_plane(&mut self, node: &Node, at: Placed, view: &View) -> Result<()> {
        if at.tilted || view.is_tilted() {
            let setup = self.plane_setup(node, at, view);
            if let Some(mut layer) = self.plane_layer(node.idx, setup)? {
                self.composite_plane(&mut layer, &[]);
            }
            return Ok(());
        }
        if !view.visible(at.world.t) {
            return Ok(());
        }
        let center = view.project(at.world.t);
        let scale = view.magnification(at.world.t) * at.scale;
        let (w, h) = (
            (f64::from(node.size[0]) * scale) as f32,
            (f64::from(node.size[1]) * scale) as f32,
        );
        let rect = Rect {
            x: (center[0] - f64::from(w) / 2.0) as f32,
            y: (center[1] - f64::from(h) / 2.0) as f32,
            w,
            h,
        };
        let roll = at.roll - view.roll() as f32;
        self.draw_plane(node.idx, rect, scale as f32, roll)
    }

    /// Draws plane `idx` through the normal element pipeline at a placement computed by its space:
    /// border box `rect`, scale `cumulative`, and on-screen roll `roll` (degrees).
    pub(in crate::render) fn draw_plane(
        &mut self,
        idx: usize,
        rect: Rect,
        cumulative: f32,
        roll: f32,
    ) -> Result<()> {
        let rotate = self.current[idx].rotate;
        self.current[idx].rotate = roll;
        let previous = self.plane_pose.replace((idx, rect, cumulative));
        let outcome = self.draw_node(idx, rect, cumulative, None);
        self.plane_pose = previous;
        self.current[idx].rotate = rotate;
        outcome
    }
}
