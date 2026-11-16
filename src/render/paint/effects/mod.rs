//! Element effects drawn through an offscreen layer: `filter` (blur, color functions,
//! `drop-shadow`) and `mix-blend-mode`.
//!
//! The element (box, shadows, content, and children) is drawn twice, on black and on white, to
//! recover premultiplied RGBA. The layer is padded by the reach of the blur and the drop shadow.
//! The stages then run in a fixed order, matching the order `filter` accepts: gaussian blur
//! (premultiplied, the backdrop blur's three-pass box approximation), color functions
//! (`color.rs`, on unpremultiplied values), drop shadow (the blurred, offset alpha under the
//! layer), and finally compositing with the blend function (`blend.rs`) over the frame.
//!
//! Non-normal blend modes need the real backdrop, so they are rejected inside another offscreen
//! layer (`overflow: hidden`, rotation, reveals, or another filter), where only a black or white
//! stand-in is behind the element.

pub(in crate::render) mod blend;
pub(in crate::render) mod color;

use crate::model::{BlendMode, Color, Shadow};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::backdrop::{blur_plane, box_radius};
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::{bail, Result};
use color::ColorChain;

/// Pixels of padding around the element so the blur can spread past its box (three box passes).
fn reach(box_r: usize) -> Result<i32> {
    box_r
        .checked_mul(3)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("filter: blur reach is too large; use --scale"))
}

/// A drop shadow in layer pixels: integer offset, box radius, and color.
struct LayerShadow {
    dx: i32,
    dy: i32,
    box_r: usize,
    color: Color,
}

impl LayerShadow {
    fn new(shadow: Shadow, k: f32) -> Result<Self> {
        let offset =
            |v: f32| crate::render::allocation::pixel_edge((v * k).round(), "filter: drop-shadow");
        // As for `text-shadow`, the blur radius is twice the gaussian's standard deviation.
        Ok(Self {
            dx: offset(shadow.x)?,
            dy: offset(shadow.y)?,
            box_r: box_radius(shadow.blur * k / 2.0),
            color: shadow.color,
        })
    }

    /// Distance in px the shadow can reach past the element's own pixels.
    fn reach(&self) -> Result<i32> {
        let spread = reach(self.box_r)?;
        spread
            .checked_add(self.dx.abs().max(self.dy.abs()))
            .ok_or_else(|| anyhow::anyhow!("filter: drop-shadow reach is too large"))
    }
}

impl<'a> Renderer<'a> {
    /// Whether element `idx` needs [`Self::draw_with_effects`] at the given blur `sigma`.
    pub(in crate::render) fn has_effects(&self, idx: usize, sigma: f32) -> bool {
        let style = &self.composition.elements[idx].style;
        idx != 0
            && (sigma > 0.0
                || !style.filter_colors.is_empty()
                || style.drop_shadow.is_some()
                || style.blend != BlendMode::Normal)
    }

    /// Draws element `idx` (placed in `rect`) through its `filter` stages and `mix-blend-mode`;
    /// `sigma` is the element blur's standard deviation in output px (0 for none).
    pub(in crate::render) fn draw_with_effects(
        &mut self,
        idx: usize,
        rect: Rect,
        cumulative: f32,
        sigma: f32,
    ) -> Result<()> {
        let style = &self.composition.elements[idx].style;
        let blend = style.blend;
        if blend != BlendMode::Normal && self.isolated > 0 {
            bail!(
                "mix-blend-mode on {} cannot be drawn inside overflow: hidden, rotation, a reveal, a wipe, or a filter; move it out of that element",
                self.label(idx)
            );
        }
        let colors = ColorChain::new(&style.filter_colors);
        let k = self.scale;
        let box_r = box_radius(sigma);
        if box_r == 0
            && colors.is_identity()
            && style.drop_shadow.is_none()
            && blend == BlendMode::Normal
        {
            return self.draw_posed(idx, rect, cumulative);
        }
        let shadow = style
            .drop_shadow
            .map(|s| LayerShadow::new(s, k * cumulative))
            .transpose()?;
        let shadow_pad = shadow.as_ref().map_or(Ok(0), LayerShadow::reach)?;
        let pad = reach(box_r)?
            .checked_add(shadow_pad)
            .ok_or_else(|| anyhow::anyhow!("filter: reach is too large; use --scale"))?;
        // Shadows can extend past the box, so pad generously; stay near the frame to bound memory.
        let own = self.shadow_reach(idx, cumulative)?;
        let margin = pad
            .checked_add(own)
            .ok_or_else(|| anyhow::anyhow!("filter: blur margin is too large; use --scale"))?;
        let (sw, sh) = (self.surface.width as i32, self.surface.height as i32);
        let max_x = sw
            .checked_add(pad)
            .ok_or_else(|| anyhow::anyhow!("filter layer width is too large"))?;
        let max_y = sh
            .checked_add(pad)
            .ok_or_else(|| anyhow::anyhow!("filter layer height is too large"))?;
        let edge =
            |value: f32| crate::render::allocation::pixel_edge(value, "filter").map(i64::from);
        let wanted = [
            (edge((rect.x * k).floor())? - i64::from(margin)).max(-i64::from(pad)) as i32,
            (edge((rect.y * k).floor())? - i64::from(margin)).max(-i64::from(pad)) as i32,
            (edge(((rect.x + rect.w) * k).ceil())? + i64::from(margin)).min(i64::from(max_x))
                as i32,
            (edge(((rect.y + rect.h) * k).ceil())? + i64::from(margin)).min(i64::from(max_y))
                as i32,
        ];
        let [x0, y0, x1, y1] = wanted;
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        let (lw, lh) = crate::render::allocation::layer_dimensions([x0, y0, x1, y1], "filter")?;
        let (ox, oy) = (x0 as f32 / k, y0 as f32 / k);
        let shifted = Rect {
            x: rect.x - ox,
            y: rect.y - oy,
            ..rect
        };
        let mut layers: Vec<Vec<u8>> = Vec::with_capacity(2);
        let mut outcome = Ok(());
        for (pass, backdrop) in [Color::BLACK, Color::WHITE].into_iter().enumerate() {
            let layer = Surface::filled(lw as u32, lh as u32, k, backdrop);
            let outer = std::mem::replace(&mut self.surface, layer);
            let saved = self.begin_layer_pass(pass, (ox, oy), None);
            outcome = outcome.and_then(|()| self.draw_posed(idx, shifted, cumulative));
            self.end_layer_pass(saved);
            layers.push(std::mem::replace(&mut self.surface, outer).pixels);
        }
        outcome?;
        // Zeroed allocations come from the allocator already cleared; zips avoid bounds checks.
        let mut planes = vec![vec![0.0_f32; lw * lh]; 4];
        if let [r, g, b, a] = &mut planes[..] {
            let texels = layers[0]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(layers[1].as_chunks::<4>().0)
                .map(|(black, white)| super::clip::premultiplied_pixel(black, white));
            let outputs = r
                .iter_mut()
                .zip(g.iter_mut())
                .zip(b.iter_mut())
                .zip(a.iter_mut());
            for ([tr, tg, tb, ta], (((r, g), b), a)) in texels.zip(outputs) {
                (*r, *g, *b, *a) = (f32::from(tr), f32::from(tg), f32::from(tb), f32::from(ta));
            }
        }
        drop(layers);
        if box_r > 0 {
            for plane in &mut planes {
                blur_plane(plane, lw, lh, box_r);
            }
        }
        if !colors.is_identity() {
            apply_colors(&mut planes, &colors);
        }
        if let Some(shadow) = &shadow {
            apply_drop_shadow(&mut planes, lw, lh, shadow);
        }
        let Some(bounds) = clip_pixels((self.surface.width, self.surface.height), x0, y0, x1, y1)
        else {
            return Ok(());
        };
        self.surface.mark_painted(bounds);
        let layer = blend::Layer {
            planes: &planes,
            width: lw,
            origin: (x0, y0),
        };
        let stride = self.surface.width as usize;
        blend::composite_layer(blend, &mut self.surface.pixels, stride, bounds, &layer);
        Ok(())
    }

    /// Largest distance in px that an outer `box-shadow` of `idx` reaches past its box.
    fn shadow_reach(&self, idx: usize, cumulative: f32) -> Result<i32> {
        let k = self.scale * cumulative;
        let mut reach = 0;
        for s in self.composition.elements[idx]
            .style
            .shadows
            .iter()
            .filter(|s| !s.inset)
        {
            let offset = s.x.abs().max(s.y.abs());
            let value = crate::render::allocation::pixel_edge(
                ((offset + s.spread.max(0.0) + s.blur * 1.5) * k).ceil(),
                "box-shadow",
            )?;
            let value = value
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("box-shadow reach is too large"))?;
            reach = reach.max(value);
        }
        Ok(reach)
    }

    /// `#id` or `element N`, for error messages.
    fn label(&self, idx: usize) -> String {
        self.composition.elements[idx]
            .id
            .as_ref()
            .map_or_else(|| format!("element {idx}"), |id| format!("#{id}"))
    }
}

/// Runs the color functions on premultiplied planes (values 0..=255), in place.
fn apply_colors(planes: &mut [Vec<f32>], colors: &ColorChain) {
    let [r, g, b, a] = planes else {
        return;
    };
    for (((r, g), b), a) in r.iter_mut().zip(g.iter_mut()).zip(b.iter_mut()).zip(a) {
        if *a <= 0.0 {
            continue;
        }
        let alpha = *a / 255.0;
        let unpremultiply = |v: f32| (v / *a).clamp(0.0, 1.0);
        let ([cr, cg, cb], alpha) = colors.apply(
            [unpremultiply(*r), unpremultiply(*g), unpremultiply(*b)],
            alpha,
        );
        let scale = alpha * 255.0;
        (*r, *g, *b, *a) = (cr * scale, cg * scale, cb * scale, scale);
    }
}

/// Puts the offset, blurred, tinted alpha of the layer under it (premultiplied `over`).
fn apply_drop_shadow(planes: &mut [Vec<f32>], width: usize, height: usize, shadow: &LayerShadow) {
    let tint = f32::from(shadow.color.a) / 255.0;
    let mut cast = vec![0.0_f32; width * height];
    for y in 0..height {
        let sy = y as i64 - i64::from(shadow.dy);
        if sy < 0 || sy >= height as i64 {
            continue;
        }
        for x in 0..width {
            let sx = x as i64 - i64::from(shadow.dx);
            if sx < 0 || sx >= width as i64 {
                continue;
            }
            cast[y * width + x] = planes[3][sy as usize * width + sx as usize] * tint;
        }
    }
    if shadow.box_r > 0 {
        blur_plane(&mut cast, width, height, shadow.box_r);
    }
    let rgb = [shadow.color.r, shadow.color.g, shadow.color.b].map(|c| f32::from(c) / 255.0);
    for (i, &s) in cast.iter().enumerate() {
        if s <= 0.0 {
            continue;
        }
        let under = 1.0 - planes[3][i] / 255.0;
        if under <= 0.0 {
            continue;
        }
        for (plane, c) in planes.iter_mut().zip(rgb) {
            plane[i] += c * s * under;
        }
        planes[3][i] += s * under;
    }
}

#[cfg(test)]
mod tests {
    use super::reach;

    #[test]
    fn reach_covers_three_box_passes() {
        assert_eq!(reach(0).unwrap(), 1);
        assert_eq!(reach(5).unwrap(), 16);
        assert!(reach(usize::MAX).is_err());
    }
}
