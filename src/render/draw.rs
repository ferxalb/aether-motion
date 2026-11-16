//! Scene traversal: places each element, applies its pose, and draws its box, content, and children.

use super::geometry::Rect;
use super::paint::ImageSrc;
use super::Renderer;
use crate::model::{Background, Color, Content};
use anyhow::Result;

impl<'a> Renderer<'a> {
    pub(in crate::render) fn draw_node(
        &mut self,
        idx: usize,
        parent: Rect,
        parent_scale: f32,
        assigned: Option<Rect>,
    ) -> Result<()> {
        let state = self.current[idx];
        let alpha = state.alpha;
        if alpha <= 0.0 {
            return Ok(());
        }
        let element = &self.composition.elements[idx];
        let ps = parent_scale;
        let mut rect = if idx == 0 {
            parent
        } else if let Some(slot) = assigned {
            // Static items ignore left/top; `position: relative` shifts them.
            let relative = element.style.position.as_deref() == Some("relative");
            let offset = |length: Option<crate::model::Length>, basis: f32| {
                length
                    .filter(|_| relative)
                    .map_or(0.0, |n| n.resolve_scaled(basis, ps))
            };
            Rect {
                x: slot.x + offset(element.style.left, parent.w),
                y: slot.y + offset(element.style.top, parent.h),
                ..slot
            }
        } else {
            let style = &element.style;
            let width = style
                .width
                .map_or(parent.w, |n| n.resolve_scaled(parent.w, ps));
            Rect {
                x: parent.x
                    + style.left.map_or(0.0, |n| n.resolve_scaled(parent.w, ps))
                    + style.margin[3] * ps,
                y: parent.y
                    + style.top.map_or(0.0, |n| n.resolve_scaled(parent.h, ps))
                    + style.margin[0] * ps,
                w: style
                    .max_width
                    .map_or(width, |n| width.min(n.resolve_scaled(parent.w, ps))),
                h: style
                    .height
                    .map_or(parent.h, |n| n.resolve_scaled(parent.h, ps)),
            }
        };
        rect.x += state.x * ps;
        rect.y += state.y * ps;
        // CSS scale grows around the element center; descendants inherit it.
        let mut cumulative = ps;
        if idx != 0 && state.scale != 1.0 {
            let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            rect.w *= state.scale;
            rect.h *= state.scale;
            rect.x = cx - rect.w / 2.0;
            rect.y = cy - rect.h / 2.0;
            cumulative *= state.scale;
        }
        let sigma = self.composition.elements[idx].style.filter_blur * cumulative * self.scale;
        if self.has_effects(idx, sigma) {
            return self.draw_with_effects(idx, rect, cumulative, sigma);
        }
        self.draw_posed(idx, rect, cumulative)
    }

    /// Draws a placed element through its reveal, rotation, or plain box path.
    pub(in crate::render) fn draw_posed(
        &mut self,
        idx: usize,
        rect: Rect,
        cumulative: f32,
    ) -> Result<()> {
        let state = self.current[idx];
        if idx != 0 && state.reveal < 1.0 {
            if let Some(from) = self.composition.elements[idx].motion.wipe {
                return self.draw_revealed(idx, rect, cumulative, state.reveal, from);
            }
        }
        if idx != 0 && state.rotate != 0.0 {
            return self.draw_rotated(idx, rect, cumulative, state.rotate);
        }
        self.draw_body(idx, rect, cumulative)
    }

    /// Draws one element and its subtree into `rect` with no rotation.
    pub(in crate::render) fn draw_body(
        &mut self,
        idx: usize,
        rect: Rect,
        cumulative: f32,
    ) -> Result<()> {
        let alpha = self.current[idx].alpha;
        let element = &self.composition.elements[idx];
        let radius = element.style.border_radius * cumulative;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return Ok(());
        }
        // The first shadow of the list sits on top, so paint from the last one backwards.
        for &shadow in element.style.shadows.iter().rev().filter(|s| !s.inset) {
            self.surface
                .draw_shadow(rect, radius, shadow, alpha, cumulative)?;
        }
        if element.style.backdrop_blur > 0.0 {
            let sigma = element.style.backdrop_blur * cumulative * self.surface.scale;
            self.surface.backdrop_blur(rect, radius, sigma, alpha);
        }
        match element.style.background {
            Some(Background::Solid(color)) => {
                let base = self.composition.background;
                if idx != 0 || (color.r, color.g, color.b) != (base.r, base.g, base.b) {
                    self.surface.fill(rect, color, alpha, radius);
                }
            }
            Some(Background::Linear {
                from,
                to,
                horizontal,
            }) => {
                self.surface
                    .fill_gradient(rect, [from, to], horizontal, alpha, radius);
            }
            Some(Background::Gradient(ref gradient)) => {
                self.surface
                    .fill_gradient_general(rect, gradient, alpha, radius)?;
            }
            None => {}
        }
        let border = element.style.border_width * cumulative;
        // Inset shadows live in the padding box: above the background, below border and content.
        let padding_box = Rect {
            x: rect.x + border,
            y: rect.y + border,
            w: (rect.w - 2.0 * border).max(0.0),
            h: (rect.h - 2.0 * border).max(0.0),
        };
        let inner_radius = (radius - border).max(0.0);
        for &shadow in element.style.shadows.iter().rev().filter(|s| s.inset) {
            self.surface
                .draw_inset_shadow(padding_box, inner_radius, shadow, alpha, cumulative)?;
        }
        if border > 0.0 {
            let color = element.style.border_color.unwrap_or(Color::WHITE);
            self.surface.draw_border(rect, radius, border, color, alpha);
        }
        let [top, right, bottom, left] = element
            .style
            .padding
            .map(|padding| padding * cumulative + border);
        let content = Rect {
            x: rect.x + left,
            y: rect.y + top,
            w: (rect.w - left - right).max(0.0),
            h: (rect.h - top - bottom).max(0.0),
        };
        if element.style.overflow_hidden {
            return self.draw_clipped(idx, rect, content, cumulative);
        }
        self.draw_contents(idx, rect, content, cumulative)
    }

    /// Draws the content of an element (image, text, and children); its own box is already painted.
    pub(in crate::render) fn draw_contents(
        &mut self,
        idx: usize,
        rect: Rect,
        content: Rect,
        cumulative: f32,
    ) -> Result<()> {
        let alpha = self.current[idx].alpha;
        let element = &self.composition.elements[idx];
        match &element.content {
            Content::Image(path) => self.draw_image(
                ImageSrc::File(path),
                rect,
                element.style.object_fit.as_deref().unwrap_or("fill"),
                alpha,
            )?,
            Content::InlineSvg(markup) => self.draw_image(
                ImageSrc::Inline(markup),
                rect,
                element.style.object_fit.as_deref().unwrap_or("fill"),
                alpha,
            )?,
            Content::Video(source) => self.draw_video(
                idx,
                source,
                rect,
                element.style.object_fit.as_deref().unwrap_or("fill"),
                alpha,
            )?,
            Content::Text(_) => self.draw_text(idx, content, alpha, cumulative)?,
            Content::Container => {}
        }
        let layout = if element.style.display.as_deref() == Some("flex") {
            self.flex_layout(idx, content, cumulative)?
        } else {
            Vec::new()
        };
        for i in 0..self.children[idx].len() {
            let child = self.children[idx][i];
            let slot = layout.iter().find(|(c, _)| *c == child).map(|(_, r)| *r);
            self.draw_node(child, rect, cumulative, slot)?;
        }
        Ok(())
    }
}
