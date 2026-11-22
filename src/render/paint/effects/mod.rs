//! Element effects drawn through an offscreen layer: `filter` (blur, color functions,
//! `drop-shadow`), the tint, channel split, and glitch of `model::FxMotion` (`fx.rs`),
//! `mix-blend-mode`, and the `data-hf-bloom` halo (`bloom.rs`).
//!
//! The element (box, shadows, content, and children) is drawn twice, on black and on white, to
//! recover premultiplied RGBA. The layer is padded by the reach of the blur and the drop shadow.
//! The stages then run in a fixed order, matching the order `filter` accepts: gaussian blur
//! (premultiplied, the backdrop blur's three-pass box approximation), color functions
//! (`color.rs`, on unpremultiplied values), tint, channel split, glitch, drop shadow (the blurred, offset alpha under the
//! layer), and finally compositing with the blend function (`blend.rs`) over the frame. A bloom
//! halo, computed from the final texels, is then added over the composited element.
//!
//! Animated filters read their amounts from the element's `FrameState`. An animated blur uses the
//! continuous kernel of `exact_blur.rs`; a static blur keeps the whole-radius box kernel. Animated
//! color functions replace the amounts of the same functions in the static `filter` and the others
//! follow in `FilterChannel::ALL` order, so a chain that is neutral at some instant skips the layer.
//!
//! Non-normal blend modes need the real backdrop, so they are rejected inside another offscreen
//! layer (`overflow: hidden`, rotation, reveals, or another filter), where only a black or white
//! stand-in is behind the element.

pub(in crate::render) mod blend;
pub(in crate::render) mod bloom;
pub(in crate::render) mod color;
pub(in crate::render) mod exact_blur;
mod fx;

use crate::model::{BlendMode, Color, ColorFilter, Content, FilterChannel, Shadow};
use crate::render::geometry::{clip_pixels, Rect};
use crate::render::paint::backdrop::{blur_plane, box_radius};
use crate::render::paint::Surface;
use crate::render::Renderer;
use anyhow::{bail, Result};
use color::ColorChain;
use exact_blur::{blur_plane_exact, ExactBox};

/// Blur kernel of one draw: the static whole-radius box, or the continuous one when animated.
#[derive(Clone, Copy)]
enum Kernel {
    Boxes(usize),
    Exact(ExactBox),
}

impl Kernel {
    fn is_none(self) -> bool {
        matches!(self, Self::Boxes(0))
    }

    fn reach(self) -> Result<i32> {
        match self {
            Self::Boxes(0) => Ok(0),
            Self::Boxes(r) => reach(r),
            Self::Exact(pass) => i32::try_from(pass.reach() + 1)
                .map_err(|_| anyhow::anyhow!("filter: blur reach is too large; use --scale")),
        }
    }

    fn apply(self, plane: &mut [f32], width: usize, height: usize) {
        match self {
            Self::Boxes(0) => {}
            Self::Boxes(r) => blur_plane(plane, width, height, r),
            Self::Exact(pass) => blur_plane_exact(plane, width, height, pass),
        }
    }
}

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
        // Unlike `box-shadow` and `text-shadow`, the `drop-shadow()` length is the gaussian's
        // standard deviation itself (Filter Effects), so it is not halved.
        Ok(Self {
            dx: offset(shadow.x)?,
            dy: offset(shadow.y)?,
            box_r: box_radius(shadow.blur * k),
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
        let element = &self.composition.elements[idx];
        let style = &element.style;
        idx != 0
            && (sigma > 0.0
                || !style.filter_colors.is_empty()
                || style.drop_shadow.is_some()
                || style.blend != BlendMode::Normal
                || !element.motion.filter.is_static()
                || !element.motion.fx.is_static()
                || element.effects.bloom.is_some())
    }

    /// The element's bloom at the current instant, in output pixels; `None` when it casts none.
    fn bloom_pass(&self, idx: usize, cumulative: f32) -> Option<bloom::BloomPass> {
        let element = &self.composition.elements[idx];
        let bloom = element.effects.bloom?;
        let intensity = if element.motion.fx.bloom.0.is_empty() {
            bloom.intensity
        } else {
            self.current[idx].fx[3]
        };
        bloom::BloomPass::new(bloom, intensity, self.scale * cumulative)
    }

    /// The element's `filter: blur()` radius (composition px) at the current instant.
    pub(in crate::render) fn filter_blur(&self, idx: usize) -> f32 {
        let element = &self.composition.elements[idx];
        if element.motion.filter.is_animated(FilterChannel::Blur) {
            self.current[idx].filters[FilterChannel::Blur.index()]
        } else {
            element.style.filter_blur
        }
    }

    /// The element's color functions at the current instant (see the module notes for the order).
    fn color_filters(&self, idx: usize) -> Vec<ColorFilter> {
        let element = &self.composition.elements[idx];
        let motion = &element.motion.filter;
        if motion.is_static() {
            return element.style.filter_colors.clone();
        }
        let values = self.current[idx].filters;
        let animated = |channel: FilterChannel| {
            motion
                .is_animated(channel)
                .then(|| channel.color(values[channel.index()]))
                .flatten()
        };
        element
            .style
            .filter_colors
            .iter()
            .map(|&f| FilterChannel::of(f).and_then(animated).unwrap_or(f))
            .chain(motion.appended.iter().filter_map(|&c| animated(c)))
            .collect()
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
        if blend != BlendMode::Normal && self.projecting > 0 {
            bail!(
                "mix-blend-mode on {} cannot be drawn on a projected plane of a 2.5D space (tilted, seen by a camera that turns, or split where planes intersect); keep blended elements on planes that face the camera and do not intersect others",
                self.label(idx)
            );
        }
        if blend != BlendMode::Normal && self.isolated > 0 {
            bail!(
                "mix-blend-mode on {} cannot be drawn inside overflow: hidden, rotation, a reveal, a wipe, or a filter; move it out of that element",
                self.label(idx)
            );
        }
        let colors = ColorChain::new(&self.color_filters(idx));
        let k = self.scale;
        let kernel = if self.composition.elements[idx]
            .motion
            .filter
            .is_animated(FilterChannel::Blur)
        {
            ExactBox::new(sigma).map_or(Kernel::Boxes(0), Kernel::Exact)
        } else {
            Kernel::Boxes(box_radius(sigma))
        };
        let fx = self.current[idx].fx;
        let fx_seed = self.current[idx].fx_seed;
        let tint_color = self.composition.elements[idx].motion.fx.tint_color;
        let tint = tint_color.filter(|_| fx[0] > 0.0).map(|c| (c, fx[0]));
        let split = (fx[2] * cumulative * k).round();
        let has_fx = tint.is_some() || fx[1] > 0.0 || split != 0.0;
        let bloom = self.bloom_pass(idx, cumulative);
        if kernel.is_none()
            && colors.is_identity()
            && style.drop_shadow.is_none()
            && blend == BlendMode::Normal
            && !has_fx
            && bloom.is_none()
        {
            return self.draw_posed(idx, rect, cumulative);
        }
        let shadow = style
            .drop_shadow
            .map(|s| LayerShadow::new(s, k * cumulative))
            .transpose()?;
        let shadow_pad = shadow.as_ref().map_or(Ok(0), LayerShadow::reach)?;
        let pad = kernel
            .reach()?
            .max(1)
            .checked_add(shadow_pad)
            .and_then(|n| n.checked_add(split.abs() as i32))
            .and_then(|n| n.checked_add(bloom.map_or(0, bloom::BloomPass::reach)))
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
        // A rotated element is drawn around its center, so the layer must hold its rotated bounds.
        let (sin, cos) = self.current[idx].rotate.to_radians().sin_cos();
        let (half_w, half_h) = (
            (rect.w * cos.abs() + rect.h * sin.abs()) / 2.0,
            (rect.w * sin.abs() + rect.h * cos.abs()) / 2.0,
        );
        let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        let wanted = [
            (edge(((cx - half_w) * k).floor())? - i64::from(margin)).max(-i64::from(pad)) as i32,
            (edge(((cy - half_h) * k).floor())? - i64::from(margin)).max(-i64::from(pad)) as i32,
            (edge(((cx + half_w) * k).ceil())? + i64::from(margin)).min(i64::from(max_x)) as i32,
            (edge(((cy + half_h) * k).ceil())? + i64::from(margin)).min(i64::from(max_y)) as i32,
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
        // Layers of the static blur alone keep their own empty caches, so their output stays as it
        // was; the other effects share the frame's caches, frozen so both passes agree.
        let share = !(colors.is_identity()
            && !has_fx
            && shadow.is_none()
            && blend == BlendMode::Normal
            && bloom.is_none()
            && matches!(kernel, Kernel::Boxes(_)));
        if share {
            self.surface.gradient_cache.freeze();
            self.surface.shadow_cache.freeze();
        }
        let mut layers: Vec<Vec<u8>> = Vec::with_capacity(2);
        let mut outcome = Ok(());
        for (pass, backdrop) in [Color::BLACK, Color::WHITE].into_iter().enumerate() {
            let mut layer = Surface::filled(lw as u32, lh as u32, k, backdrop);
            if share {
                layer.take_caches(&mut self.surface);
            }
            let outer = std::mem::replace(&mut self.surface, layer);
            let saved = self.begin_layer_pass(pass, (ox, oy), None);
            outcome = outcome.and_then(|()| self.draw_posed(idx, shifted, cumulative));
            self.end_layer_pass(saved);
            let mut layer = std::mem::replace(&mut self.surface, outer);
            if share {
                self.surface.take_caches(&mut layer);
            }
            layers.push(layer.pixels);
        }
        if share {
            self.surface.gradient_cache.thaw();
            self.surface.shadow_cache.thaw();
        }
        outcome?;
        let Some(bounds) = clip_pixels((self.surface.width, self.surface.height), x0, y0, x1, y1)
        else {
            return Ok(());
        };
        let stride = self.surface.width as usize;
        if kernel.is_none() && shadow.is_none() && !has_fx {
            // Color functions and blending only: recover, recolor, and composite in one pass.
            self.surface.mark_painted(bounds);
            let layer = blend::Layer {
                texels: blend::Texels::Passes {
                    black: &layers[0],
                    white: &layers[1],
                    colors: &colors,
                },
                width: lw,
                origin: (x0, y0),
            };
            blend::composite_layer(blend, &mut self.surface.pixels, stride, bounds, &layer);
            if let Some(bloom) = bloom {
                let pixels = &mut self.surface.pixels;
                let scratch = &mut self.bloom_scratch;
                crate::profile::time(crate::profile::Phase::Bloom, || {
                    bloom.apply(&layer, lh, pixels, stride, bounds, scratch)
                });
            }
            return Ok(());
        }
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
        if !kernel.is_none() {
            for plane in &mut planes {
                kernel.apply(plane, lw, lh);
            }
        }
        if !colors.is_identity() {
            apply_colors(&mut planes, &colors);
        }
        if let Some((color, amount)) = tint {
            fx::apply_tint(&mut planes, color, amount);
        }
        if split != 0.0 {
            fx::apply_rgb_split(&mut planes, lw, split);
        }
        if fx[1] > 0.0 {
            // The element's box in layer pixels, so slices read from inside it only.
            let edge = |v: f32, hi: usize| ((v * k).round().max(0.0) as usize).min(hi);
            let span = fx::Span {
                x0: edge(shifted.x, lw),
                y0: edge(shifted.y, lh),
                x1: edge(shifted.x + shifted.w, lw),
                y1: edge(shifted.y + shifted.h, lh),
            };
            fx::apply_glitch(&mut planes, lw, span, fx[1], fx_seed);
        }
        if let Some(shadow) = &shadow {
            apply_drop_shadow(&mut planes, lw, lh, shadow);
        }
        self.surface.mark_painted(bounds);
        let layer = blend::Layer {
            texels: blend::Texels::Planes(&planes),
            width: lw,
            origin: (x0, y0),
        };
        blend::composite_layer(blend, &mut self.surface.pixels, stride, bounds, &layer);
        if let Some(bloom) = bloom {
            let pixels = &mut self.surface.pixels;
            let scratch = &mut self.bloom_scratch;
            crate::profile::time(crate::profile::Phase::Bloom, || {
                bloom.apply(&layer, lh, pixels, stride, bounds, scratch)
            });
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
        // The element's own `text-shadow` can also fall outside its box.
        if matches!(self.composition.elements[idx].content, Content::Text(_)) {
            for s in self.text_props(idx).shadow {
                let value = crate::render::allocation::pixel_edge(
                    ((s.x.abs().max(s.y.abs()) + s.blur * 1.5) * k).ceil(),
                    "text-shadow",
                )?
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("text-shadow reach is too large"))?;
                reach = reach.max(value);
            }
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
        if *a > 0.0 {
            [*r, *g, *b, *a] = colors.recolor([*r, *g, *b, *a]);
        }
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
