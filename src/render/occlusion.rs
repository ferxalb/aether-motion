//! Occlusion culling: siblings painted before a scene instance that covers the whole frame with
//! an opaque background are not drawn, because that background overwrites every pixel they
//! would have painted.
//!
//! The test is deliberately narrow so the frame stays byte-identical. It applies only while
//! drawing straight onto the frame (no offscreen layer, layer-blur sample, or layout probe). The
//! covering child must be a scene host whose only child is the scene root, both drawn through the
//! plain path (fully opaque, no rotation, partial reveal, clip path, mask, filter, pixel effect,
//! bloom, blend mode, or motion blur; the host also without `overflow: hidden` or flex layout).
//! The scene root must have a solid background with full alpha and square corners, and its placed
//! box must contain the whole output surface. Such a fill writes the background color exactly
//! into every pixel, so nothing drawn before it, including what backdrop filters and blend modes
//! inside the scene would read, can change the frame.

use super::geometry::Rect;
use super::Renderer;
use crate::model::Background;

impl<'a> Renderer<'a> {
    /// Whether `child` of element `idx` (placed at `rect`, with scale `ps`, and flex `layout`)
    /// covers the frame with an opaque scene background at the current instant.
    pub(in crate::render) fn covers_frame(
        &self,
        idx: usize,
        rect: Rect,
        ps: f32,
        layout: &[(usize, Rect)],
        child: usize,
    ) -> bool {
        if self.isolated > 0
            || self.layer_depth > 0
            || self.probe.is_some()
            || self.layer_blur.sampling
        {
            return false;
        }
        let elements = &self.composition.elements;
        let &[root] = self.children[child].as_slice() else {
            return false;
        };
        let (host, scene) = (&elements[child], &elements[root]);
        let style = &scene.style;
        let opaque = matches!(style.background, Some(Background::Solid(color)) if color.a == 255);
        if !scene.layout.scene_root
            || !opaque
            || style.background_clip_text
            || style.border_radius != 0.0
            || scene.layout.parallax.is_some_and(|factor| factor != 1.0)
            || host.style.overflow_hidden
            || host.style.display.as_deref() == Some("flex")
            || self.current[root].alpha < 1.0
            || !self.plain(child)
            || !self.plain(root)
        {
            return false;
        }
        let (parent, slot) = self.child_frame(idx, rect, ps, layout, child);
        let (host_rect, host_scale) = self.posed_rect(child, parent, ps, slot);
        let (rect, _) = self.posed_rect(root, host_rect, host_scale, None);
        let k = self.scale;
        rect.x * k <= 0.0
            && rect.y * k <= 0.0
            && (rect.x + rect.w) * k >= self.surface.width as f32
            && (rect.y + rect.h) * k >= self.surface.height as f32
    }

    /// Whether element `idx` is drawn straight onto the target, fully opaque, through
    /// `draw_body`.
    fn plain(&self, idx: usize) -> bool {
        let state = self.current[idx];
        let element = &self.composition.elements[idx];
        state.alpha >= 1.0
            && state.rotate == 0.0
            && (state.reveal >= 1.0 || element.motion.wipe.is_none())
            && self.filter_blur(idx) == 0.0
            && !self.is_layer_blurred(idx)
            && !self.needs_mask(idx)
            && !self.has_effects(idx, 0.0)
    }
}
