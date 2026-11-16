//! `filter: blur()`: the element (box, shadows, content, and children) renders on an offscreen layer
//! padded by the blur's reach, the layer is gaussian-blurred as premultiplied RGBA, and the result
//! composites over the frame.
//!
//! Reuses the backdrop blur's three-pass box approximation, so the output stays deterministic.
//! Black/white recovery writes directly to blur planes without an intermediate RGBA buffer.

use crate::model::Color;
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::backdrop::{blur_plane, box_radius};
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::Result;

/// Pixels of padding around the element so the blur can spread past its box (three box passes).
fn reach(box_r: usize) -> Result<i32> {
    box_r
        .checked_mul(3)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| anyhow::anyhow!("filter: blur reach is too large; use --scale"))
}

impl<'a> Renderer<'a> {
    /// Draws element `idx` (placed in `rect`) blurred by a gaussian of standard deviation `sigma` px.
    pub(in crate::render) fn draw_filtered(
        &mut self,
        idx: usize,
        rect: Rect,
        cumulative: f32,
        sigma: f32,
    ) -> Result<()> {
        let box_r = box_radius(sigma);
        if box_r == 0 {
            return self.draw_posed(idx, rect, cumulative);
        }
        let k = self.scale;
        let pad = reach(box_r)?;
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
        let edge = |value: f32| {
            crate::render::allocation::pixel_edge(value, "filter: blur").map(i64::from)
        };
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
        let (lw, lh) =
            crate::render::allocation::layer_dimensions([x0, y0, x1, y1], "filter: blur")?;
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
        let mut planes = vec![vec![0.0_f32; lw * lh]; 4];
        for (i, (black, white)) in layers[0]
            .as_chunks::<4>()
            .0
            .iter()
            .zip(layers[1].as_chunks::<4>().0)
            .enumerate()
        {
            let texel = super::clip::premultiplied_pixel(black, white);
            for (plane, value) in planes.iter_mut().zip(texel) {
                plane[i] = f32::from(value);
            }
        }
        for plane in &mut planes {
            blur_plane(plane, lw, lh, box_r);
        }
        let Some(bounds) = clip_pixels((self.surface.width, self.surface.height), x0, y0, x1, y1)
        else {
            return Ok(());
        };
        self.surface.mark_painted(bounds);
        let stride = self.surface.width as usize;
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let i = (y as i32 - y0) as usize * lw + (x as i32 - x0) as usize;
                let alpha = planes[3][i] / 255.0;
                if alpha <= 0.0 {
                    continue;
                }
                let offset = (y * stride + x) * 4;
                let dest = &mut self.surface.pixels[offset..offset + 4];
                for (channel, plane) in dest.iter_mut().zip(&planes).take(3) {
                    *channel = (plane[i] + f32::from(*channel) * (1.0 - alpha))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
                dest[3] = 255;
            }
        }
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
