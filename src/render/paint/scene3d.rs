//! `data-hf-3d` painting: evaluates the scene for this instant, rasterizes it into the element's
//! padding box at the surface's resolution (camera zoom and `--scale` included), and composites the
//! straight-alpha rows over what is already painted, multiplied by the element's opacity. The
//! backend (CPU rasterizer or GPU) is chosen by `crate::scene3d::backend`.

use super::surface::Surface;
use crate::model::{Color, Content, Length};
use crate::render::frame_state::FrameState;
use crate::render::geometry::{PixelRect, Rect};
use crate::render::Renderer;
use crate::scene3d::camera::Placement;
use crate::scene3d::Scene3d;
use anyhow::Result;

impl<'a> Renderer<'a> {
    /// Draws the scene of element `idx` into `rect` (its padding box, surface px / `self.scale`).
    pub(in crate::render) fn draw_scene3d(
        &mut self,
        idx: usize,
        scene: &Scene3d,
        rect: Rect,
        cumulative: f32,
        alpha: f32,
    ) -> Result<()> {
        if alpha <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 || cumulative <= 0.0 {
            return Ok(());
        }
        let elapsed = self.time - self.composition.elements[idx].start;
        let k = f64::from(self.scale);
        let placement = Placement {
            origin: [f64::from(rect.x) * k, f64::from(rect.y) * k],
            scale: f64::from(cumulative) * k,
            size: [
                f64::from(rect.w / cumulative),
                f64::from(rect.h / cumulative),
            ],
        };
        let surface = &mut self.surface;
        let target = [surface.width as usize, surface.height as usize];
        let stride = target[0] * 4;
        let drawn = crate::profile::time(crate::profile::Phase::Scene3d, || {
            self.scene3d
                .draw(scene, elapsed, &placement, target, &mut |x, y, row| {
                    let start = y * stride + x * 4;
                    let dest = &mut surface.pixels[start..start + row.len() * 4];
                    for (dest, &[r, g, b, a]) in dest.as_chunks_mut::<4>().0.iter_mut().zip(row) {
                        if a == 255 && alpha >= 1.0 {
                            // `blend_pixel` at full coverage writes the color itself.
                            *dest = [r, g, b, 255];
                        } else if a != 0 {
                            let coverage = alpha * f32::from(a) / 255.0;
                            Surface::blend_pixel(dest, Color { r, g, b, a: 255 }, coverage);
                        }
                    }
                })
        })?;
        if let Some(region) = drawn {
            self.surface.mark_painted(PixelRect {
                x0: region.rect[0],
                y0: region.rect[1],
                x1: region.rect[2],
                y1: region.rect[3],
            });
        }
        Ok(())
    }

    /// Largest on-screen travel (composition px) of any visible 3D scene between composition times
    /// `first` and `last` (only the elements in `subset`, when given); `states` tells which
    /// elements are visible. Used to size motion-blur sample counts.
    pub(in crate::render) fn scene3d_travel(
        &self,
        first: f64,
        last: f64,
        states: &[FrameState],
        subset: Option<&[usize]>,
    ) -> f32 {
        let elements = &self.composition.elements;
        let all: Vec<usize>;
        let indices = match subset {
            Some(subset) => subset,
            None => {
                all = (0..elements.len()).collect();
                &all
            }
        };
        let px = |length: Option<Length>| match length {
            Some(Length::Px(v)) if v > 0.0 => Some(f64::from(v)),
            _ => None,
        };
        let mut travel: f64 = 0.0;
        for &idx in indices {
            let element = &elements[idx];
            let Content::Scene3d(scene) = &element.content else {
                continue;
            };
            if !states.get(idx).is_some_and(|s| s.alpha > 0.0) || !scene.is_animated() {
                continue;
            }
            let size = [
                px(element.style.width).unwrap_or(f64::from(self.composition.width)),
                px(element.style.height).unwrap_or(f64::from(self.composition.height)),
            ];
            let start = element.start;
            travel = travel.max(scene.screen_travel(first - start, last - start, size));
        }
        travel as f32
    }
}
