//! Rotated subtrees: render on an offscreen layer, then composite around the box center.

use crate::model::{Color, Content};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::Renderer;
use anyhow::{anyhow, Result};

impl<'a> Renderer<'a> {
    /// Renders the subtree on its own surface and composites it rotated around the box center.
    /// Descendants are clipped to the box plus its shadow reach.
    pub(in crate::render) fn draw_rotated(
        &mut self,
        idx: usize,
        rect: Rect,
        cumulative: f32,
        degrees: f32,
    ) -> Result<()> {
        let k = self.scale;
        let element = &self.composition.elements[idx];
        let box_reach = element.style.shadow.map_or(0.0, |s| {
            (s.x.abs().max(s.y.abs()) + s.spread.max(0.0) + s.blur * 1.5) * cumulative * k
        });
        // The element's own `text-shadow` can also fall outside its box.
        let text_reach = match element.content {
            Content::Text(_) => self.text_props(idx).shadow.map_or(0.0, |s| {
                (s.x.abs().max(s.y.abs()) + s.blur * 1.5) * cumulative * k
            }),
            _ => 0.0,
        };
        let reach = box_reach.max(text_reach) + 2.0;
        let x0 = (rect.x * k - reach).floor() as i32;
        let y0 = (rect.y * k - reach).floor() as i32;
        let x1 = ((rect.x + rect.w) * k + reach).ceil() as i32;
        let y1 = ((rect.y + rect.h) * k + reach).ceil() as i32;
        let (lw, lh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        if lw.saturating_mul(lh).saturating_mul(4) > 128 * 1024 * 1024 {
            return Err(anyhow!(
                "rotated element needs a layer over 128 MiB; use --scale"
            ));
        }
        let shifted = Rect {
            x: rect.x - x0 as f32 / k,
            y: rect.y - y0 as f32 / k,
            ..rect
        };
        // Drawing on black and on white recovers color and alpha without alpha-aware blending.
        let saved = (self.width, self.height, self.painted, self.backdrop);
        (self.width, self.height) = (lw as u32, lh as u32);
        let mut layers: Vec<Vec<u8>> = Vec::with_capacity(2);
        let mut outcome = Ok(());
        for backdrop in [Color::BLACK, Color::WHITE] {
            let mut surface = [backdrop.r, backdrop.g, backdrop.b, 255].repeat(lw * lh);
            std::mem::swap(&mut self.pixels, &mut surface);
            (self.painted, self.backdrop) = (None, backdrop);
            outcome = outcome.and_then(|()| self.draw_body(idx, shifted, cumulative));
            std::mem::swap(&mut self.pixels, &mut surface);
            layers.push(surface);
        }
        (self.width, self.height, self.painted, self.backdrop) = saved;
        outcome?;
        // Premultiplied color from the black pass; alpha from how much white shows through.
        let premultiplied: Vec<[u8; 4]> = layers[0]
            .as_chunks::<4>()
            .0
            .iter()
            .zip(layers[1].as_chunks::<4>().0)
            .map(|(b, w)| {
                let gap = (0..3)
                    .map(|c| i32::from(w[c]) - i32::from(b[c]))
                    .sum::<i32>()
                    / 3;
                [b[0], b[1], b[2], (255 - gap).clamp(0, 255) as u8]
            })
            .collect();
        let (sin, cos) = degrees.to_radians().sin_cos();
        let center = ((rect.x + rect.w / 2.0) * k, (rect.y + rect.h / 2.0) * k);
        let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(x, y)| {
            let (dx, dy) = (x as f32 - center.0, y as f32 - center.1);
            (
                center.0 + dx * cos - dy * sin,
                center.1 + dx * sin + dy * cos,
            )
        });
        let low = |f: fn(&(f32, f32)) -> f32| corners.iter().map(f).fold(f32::INFINITY, f32::min);
        let high =
            |f: fn(&(f32, f32)) -> f32| corners.iter().map(f).fold(f32::NEG_INFINITY, f32::max);
        let Some(bounds) = clip_pixels(
            (self.width, self.height),
            low(|c| c.0).floor() as i32,
            low(|c| c.1).floor() as i32,
            high(|c| c.0).ceil() as i32,
            high(|c| c.1).ceil() as i32,
        ) else {
            return Ok(());
        };
        self.mark_painted(bounds);
        let texel = |x: i32, y: i32| -> [f32; 4] {
            if x < 0 || y < 0 || x >= lw as i32 || y >= lh as i32 {
                return [0.0; 4];
            }
            premultiplied[y as usize * lw + x as usize].map(f32::from)
        };
        let stride = self.width as usize;
        // Layer coordinates are linear in x on each row: sx = ax + x*cos, sy = ay - x*sin.
        let (limit_x, limit_y) = (lw as f32, lh as f32);
        let span = |origin: f32, slope: f32, limit: f32| -> (f32, f32) {
            if slope.abs() < 1e-6 {
                return if origin > -1.0 && origin < limit {
                    (f32::NEG_INFINITY, f32::INFINITY)
                } else {
                    (f32::INFINITY, f32::NEG_INFINITY)
                };
            }
            let (a, b) = ((-1.0 - origin) / slope, (limit - origin) / slope);
            (a.min(b), a.max(b))
        };
        for y in bounds.y0..bounds.y1 {
            let dy = y as f32 + 0.5 - center.1;
            let ax = center.0 + (0.5 - center.0) * cos + dy * sin - x0 as f32 - 0.5;
            let ay = center.1 - (0.5 - center.0) * sin + dy * cos - y0 as f32 - 0.5;
            let ((x_lo, x_hi), (y_lo, y_hi)) = (span(ax, cos, limit_x), span(ay, -sin, limit_y));
            let first = (x_lo.max(y_lo).max(bounds.x0 as f32 - 1.0).floor() as i64 - 1)
                .max(bounds.x0 as i64) as usize;
            let last = ((x_hi.min(y_hi).min(bounds.x1 as f32).ceil() as i64) + 1)
                .min(bounds.x1 as i64)
                .max(0) as usize;
            for x in first..last {
                let sx = ax + x as f32 * cos;
                let sy = ay - x as f32 * sin;
                let (ix, iy) = (sx.floor(), sy.floor());
                let (fx, fy) = (sx - ix, sy - iy);
                let (ix, iy) = (ix as i32, iy as i32);
                let taps = [
                    (texel(ix, iy), (1.0 - fx) * (1.0 - fy)),
                    (texel(ix + 1, iy), fx * (1.0 - fy)),
                    (texel(ix, iy + 1), (1.0 - fx) * fy),
                    (texel(ix + 1, iy + 1), fx * fy),
                ];
                let mut sample = [0.0_f32; 4];
                for (tap, weight) in taps {
                    for (total, value) in sample.iter_mut().zip(tap) {
                        *total += value * weight;
                    }
                }
                if sample[3] < 0.5 {
                    continue;
                }
                let keep = 1.0 - sample[3] / 255.0;
                let offset = (y * stride + x) * 4;
                let dest = &mut self.pixels[offset..offset + 4];
                for (channel, value) in dest.iter_mut().zip(sample).take(3) {
                    *channel = (value + f32::from(*channel) * keep).round().min(255.0) as u8;
                }
                dest[3] = 255;
            }
        }
        Ok(())
    }
}
