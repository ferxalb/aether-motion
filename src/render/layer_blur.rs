//! Per-element motion blur (`data-hf-motion-blur` on a non-root element): the element and its
//! subtree are averaged over a shutter window while everything else is drawn once.
//!
//! The window is `shutter` degrees of one frame at the frame rate being rendered (the export's
//! `--fps`, or 30 for `render_at` snapshots), centered on the frame time, exactly as for the root's
//! `data-motion-blur`, so a still rendered with `--frame N --fps F` equals frame N of an export at
//! `F`. Each frame first plans every blurred element ([`Renderer::plan_layer_blur`]): its subtree's
//! states at the sample times, and whether any of them moves inside the window (fades, filters, and
//! video frames are not movement, as for the root shutter). An element that does not move is drawn
//! once, sharp.
//!
//! A moving element is drawn where it sits in the scene, over the static background already
//! painted: before the first sample the frame is copied once; each sample restores the pixels the
//! previous sample touched (`Surface::painted`), draws the subtree at its sample states, and adds
//! the difference to the copy into an integer accumulator that grows to cover what the samples
//! touched. The average replaces those pixels at the end, rounded exactly like the root shutter.
//! Drawing over the real background (instead of an offscreen layer) means `backdrop-filter` and
//! `mix-blend-mode` inside the subtree see what is really behind them at every sample, and the
//! average equals compositing the averaged layer for normal blending. Elements drawn after it see
//! the averaged result.
//!
//! The root's `data-motion-blur` and nested `data-hf-motion-blur` are rejected at parse time
//! (see `parse/effects.rs`); layout probes draw every element once.

use super::frame_state::FrameState;
use super::geometry::{PixelRect, Rect};
use super::shutter::{moves, travel};
use super::{states_at, Renderer};
use crate::model::{Composition, MotionBlur};
use anyhow::Result;

/// One element with `data-hf-motion-blur`.
struct Blurred {
    idx: usize,
    shutter: MotionBlur,
    /// The element and its descendants, in ascending (parent-first) order.
    subtree: Vec<usize>,
    /// Sample times of the current frame; empty when the subtree is still inside the window.
    times: Vec<f64>,
    /// Subtree states at each sample time (`samples[s][k]` is `subtree[k]` at `times[s]`).
    samples: Vec<Vec<FrameState>>,
}

/// Per-renderer state of every blurred element, plus scratch buffers reused across frames.
#[derive(Default)]
pub(in crate::render) struct LayerBlur {
    elements: Vec<Blurred>,
    /// Set while a sample is being drawn, so the blurred element is drawn directly.
    pub(in crate::render) sampling: bool,
    /// Copy of the frame before the first sample.
    backdrop: Vec<u8>,
    /// Sums of `sample - backdrop` per channel over `accum_rect`.
    accum: Vec<i32>,
    states: Vec<FrameState>,
}

impl LayerBlur {
    pub(in crate::render) fn new(composition: &Composition) -> Self {
        let elements = composition
            .elements
            .iter()
            .enumerate()
            .filter_map(|(idx, element)| {
                let shutter = element.effects.motion_blur?;
                let mut subtree = vec![idx];
                let mut next = 0;
                while next < subtree.len() {
                    subtree.extend_from_slice(&composition.elements[subtree[next]].children);
                    next += 1;
                }
                subtree.sort_unstable();
                Some(Blurred {
                    idx,
                    shutter,
                    subtree,
                    times: Vec::new(),
                    samples: Vec::new(),
                })
            })
            .collect();
        Self {
            elements,
            ..Self::default()
        }
    }
}

/// `rect` grown to include `other`.
fn union(rect: Option<PixelRect>, other: PixelRect) -> PixelRect {
    rect.map_or(other, |r| r.union(other))
}

impl<'a> Renderer<'a> {
    /// Plans every blurred element for the frame at `time` (with `current` already sampled there)
    /// and returns whether any of them moves inside its window.
    pub(in crate::render) fn plan_layer_blur(&mut self, time: f64) -> bool {
        if self.layer_blur.elements.is_empty() {
            return false;
        }
        let composition = self.composition;
        let (fps, duration) = (self.fps, composition.duration);
        let short_side = composition.width.min(composition.height);
        let mut states = std::mem::take(&mut self.layer_blur.states);
        let mut any = false;
        for i in 0..self.layer_blur.elements.len() {
            let blurred = &self.layer_blur.elements[i];
            let (shutter, idx) = (blurred.shutter, blurred.idx);
            let subset = |states: &[FrameState], subtree: &[usize]| -> Vec<FrameState> {
                subtree.iter().map(|&k| states[k]).collect()
            };
            let subtree = blurred.subtree.clone();
            let center = subset(&self.current, &subtree);
            let n = match shutter.samples {
                Some(n) => n,
                None => {
                    let (first, last) = shutter.window(time, fps, duration);
                    states_at(composition, first, &mut states);
                    let from = subset(&states, &subtree);
                    states_at(composition, last, &mut states);
                    let to = subset(&states, &subtree);
                    let (low, high) = MotionBlur::AUTO_SAMPLES;
                    let travel = travel(&from, &to, short_side);
                    ((travel * self.scale / 2.0).ceil() as u32).clamp(low, high)
                }
            };
            let times = shutter.sample_times(time, fps, duration, n);
            let samples: Vec<Vec<FrameState>> = times
                .iter()
                .map(|&t| {
                    states_at(composition, t, &mut states);
                    subset(&states, &subtree)
                })
                .collect();
            let moving = center[0].alpha > 0.0 || samples.iter().any(|s| s[0].alpha > 0.0);
            let moving = moving && moves(&samples, &center);
            debug_assert_eq!(subtree[0], idx);
            let blurred = &mut self.layer_blur.elements[i];
            if moving {
                blurred.times = times;
                blurred.samples = samples;
                any = true;
            } else {
                blurred.times.clear();
                blurred.samples.clear();
            }
        }
        self.layer_blur.states = states;
        any
    }

    /// Whether `idx` must be drawn through [`Self::draw_layer_blurred`].
    pub(in crate::render) fn is_layer_blurred(&self, idx: usize) -> bool {
        !self.layer_blur.sampling
            && self.probe.is_none()
            && self.composition.elements[idx].effects.motion_blur.is_some()
    }

    /// Draws blurred element `idx` (placed like any child: `parent`, scale, and flex slot), averaged
    /// over its planned samples, or once when it does not move.
    pub(in crate::render) fn draw_layer_blurred(
        &mut self,
        idx: usize,
        parent: Rect,
        parent_scale: f32,
        assigned: Option<Rect>,
    ) -> Result<()> {
        let Some(slot) = self.layer_blur.elements.iter().position(|b| b.idx == idx) else {
            return self.draw_sample(idx, parent, parent_scale, assigned);
        };
        if self.layer_blur.elements[slot].times.is_empty() {
            return self.draw_sample(idx, parent, parent_scale, assigned);
        }
        let times = std::mem::take(&mut self.layer_blur.elements[slot].times);
        let samples = std::mem::take(&mut self.layer_blur.elements[slot].samples);
        let subtree = std::mem::take(&mut self.layer_blur.elements[slot].subtree);
        let center: Vec<FrameState> = subtree.iter().map(|&k| self.current[k]).collect();
        let center_time = self.time;
        let mut backdrop = std::mem::take(&mut self.layer_blur.backdrop);
        backdrop.clear();
        backdrop.extend_from_slice(&self.surface.pixels);
        let mut accum = std::mem::take(&mut self.layer_blur.accum);
        let before = self.surface.painted.take();
        let mut area: Option<PixelRect> = None;
        let mut previous: Option<PixelRect> = None;
        let mut outcome = Ok(());
        for (states, &t) in samples.iter().zip(&times) {
            if let Some(rect) = previous {
                self.restore(&backdrop, rect);
            }
            for (&k, &state) in subtree.iter().zip(states) {
                self.current[k] = state;
            }
            self.time = t;
            outcome = self.draw_sample(idx, parent, parent_scale, assigned);
            if outcome.is_err() {
                break;
            }
            let Some(touched) = self.surface.painted.take() else {
                previous = None;
                continue;
            };
            let grown = union(area, touched);
            if area != Some(grown) {
                accum = regrow(accum, area, grown);
                area = Some(grown);
            }
            self.accumulate(&backdrop, &mut accum, grown, touched);
            previous = Some(touched);
        }
        if let Some(rect) = previous {
            self.restore(&backdrop, rect);
        }
        for (&k, &state) in subtree.iter().zip(&center) {
            self.current[k] = state;
        }
        self.time = center_time;
        if outcome.is_ok() {
            if let Some(area) = area {
                self.average(&backdrop, &accum, area, times.len() as i32);
            }
        }
        self.surface.painted = match (before, area) {
            (Some(b), Some(a)) => Some(b.union(a)),
            (b, a) => b.or(a),
        };
        let blurred = &mut self.layer_blur.elements[slot];
        blurred.times = times;
        blurred.samples = samples;
        blurred.subtree = subtree;
        self.layer_blur.backdrop = backdrop;
        self.layer_blur.accum = accum;
        outcome
    }

    /// Draws `idx` once at its `current` state, bypassing the blur.
    fn draw_sample(
        &mut self,
        idx: usize,
        parent: Rect,
        parent_scale: f32,
        assigned: Option<Rect>,
    ) -> Result<()> {
        self.layer_blur.sampling = true;
        let outcome = self.draw_node(idx, parent, parent_scale, assigned);
        self.layer_blur.sampling = false;
        outcome
    }

    /// Copies `rect` of `backdrop` back onto the surface.
    fn restore(&mut self, backdrop: &[u8], rect: PixelRect) {
        let stride = self.surface.width as usize * 4;
        for y in rect.y0..rect.y1 {
            let span = y * stride + rect.x0 * 4..y * stride + rect.x1 * 4;
            self.surface.pixels[span.clone()].copy_from_slice(&backdrop[span]);
        }
    }

    /// Adds `surface - backdrop` inside `touched` to `accum`, which covers `area`.
    fn accumulate(&self, backdrop: &[u8], accum: &mut [i32], area: PixelRect, touched: PixelRect) {
        let stride = self.surface.width as usize * 4;
        let aw = (area.x1 - area.x0) * 4;
        for y in touched.y0..touched.y1 {
            let span = y * stride + touched.x0 * 4..y * stride + touched.x1 * 4;
            let row = (y - area.y0) * aw + (touched.x0 - area.x0) * 4;
            let sums = &mut accum[row..row + span.len()];
            for ((sum, &now), &was) in sums
                .iter_mut()
                .zip(&self.surface.pixels[span.clone()])
                .zip(&backdrop[span])
            {
                *sum += i32::from(now) - i32::from(was);
            }
        }
    }

    /// Writes the average of `n` samples over `area`: `(n * backdrop + sum + n / 2) / n`, which is
    /// the rounded mean of the samples' pixels.
    fn average(&mut self, backdrop: &[u8], accum: &[i32], area: PixelRect, n: i32) {
        let stride = self.surface.width as usize * 4;
        let aw = (area.x1 - area.x0) * 4;
        for y in area.y0..area.y1 {
            let span = y * stride + area.x0 * 4..y * stride + area.x1 * 4;
            let sums = &accum[(y - area.y0) * aw..(y - area.y0 + 1) * aw];
            for ((pixel, &was), &sum) in self.surface.pixels[span.clone()]
                .iter_mut()
                .zip(&backdrop[span])
                .zip(sums)
            {
                *pixel = ((n * i32::from(was) + sum + n / 2) / n) as u8;
            }
        }
    }
}

/// `accum` (covering `old`, or nothing) re-laid out to cover `new`, with zeros where it grew.
fn regrow(accum: Vec<i32>, old: Option<PixelRect>, new: PixelRect) -> Vec<i32> {
    let nw = (new.x1 - new.x0) * 4;
    let mut grown = accum;
    let Some(old) = old else {
        grown.clear();
        grown.resize(nw * (new.y1 - new.y0), 0);
        return grown;
    };
    let mut out = vec![0_i32; nw * (new.y1 - new.y0)];
    let ow = (old.x1 - old.x0) * 4;
    for y in old.y0..old.y1 {
        let from = (y - old.y0) * ow;
        let to = (y - new.y0) * nw + (old.x0 - new.x0) * 4;
        out[to..to + ow].copy_from_slice(&grown[from..from + ow]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regrowing_keeps_sums_in_place() {
        let old = PixelRect {
            x0: 2,
            y0: 2,
            x1: 3,
            y1: 3,
        };
        let new = PixelRect {
            x0: 1,
            y0: 1,
            x1: 4,
            y1: 3,
        };
        let out = regrow(vec![1, 2, 3, 4], Some(old), new);
        assert_eq!(out.len(), 3 * 2 * 4);
        // The old pixel lands at column 1, row 1 of the new area.
        assert_eq!(&out[(3 + 1) * 4..(3 + 1) * 4 + 4], &[1, 2, 3, 4]);
        assert_eq!(out.iter().sum::<i32>(), 10);
    }
}
