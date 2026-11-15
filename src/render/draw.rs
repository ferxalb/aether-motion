//! Scene traversal: places each element, applies its pose, and draws its box, content, and children.

use super::geometry::Rect;
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
                x: parent.x + style.left.map_or(0.0, |n| n.resolve_scaled(parent.w, ps)),
                y: parent.y + style.top.map_or(0.0, |n| n.resolve_scaled(parent.h, ps)),
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
        if let Some(shadow) = element.style.shadow {
            self.surface
                .draw_shadow(rect, radius, shadow, alpha, cumulative);
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
            None => {}
        }
        let border = element.style.border_width * cumulative;
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
        match &element.content {
            Content::Image(path) => self.draw_image(
                path,
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
