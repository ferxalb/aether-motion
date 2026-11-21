//! Layout probe: reports where text is laid out at a given time without changing any pixel.
//!
//! The probe runs the normal draw pass and records each text element's ink box (the union of its
//! line extents, in composition px) as `draw_text` places it. Offscreen layers that only translate
//! (`overflow: hidden`, blur, and partial reveals) are probed on their first drawing pass: the
//! layer origin is added back and the box is cut to the part the layer shows. Projected planes of a
//! 2.5D space push a [`ProbeMap`] for their first pass, so their text is reported as the bounding
//! box of its projected corners, cut to what each enclosing layer shows. Text inside rotated
//! elements is not reported. Hidden elements (alpha 0) never reach `draw_text`.
use super::Renderer;
use anyhow::{anyhow, Result};

/// Ink box of one text element at one instant, in composition px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextBox {
    /// Index into `Composition::elements`.
    pub element: usize,
    /// `[left, top, right, bottom]`.
    pub rect: [f32; 4],
}

/// A projective layer being probed: text drawn at `layer_depth == depth` is in that layer's
/// composition px, which `map` takes (homogeneously) to the enclosing frame's, where `clip` is
/// what the enclosing layers show.
pub(in crate::render) struct ProbeMap {
    depth: u32,
    map: [[f64; 3]; 3],
    clip: [f32; 4],
    /// The enclosing `layer_shift`, restored when the layer is done.
    shift: (f32, f32),
}

/// Takes a text box `[left, top, right, bottom]` of the innermost probed surface to frame
/// composition px through every enclosing projective layer; `None` when nothing of it shows.
pub(in crate::render) fn project(maps: &[ProbeMap], rect: [f32; 4]) -> Option<[f32; 4]> {
    let mut rect = rect;
    for entry in maps.iter().rev() {
        let corners = [
            [rect[0], rect[1]],
            [rect[2], rect[1]],
            [rect[2], rect[3]],
            [rect[0], rect[3]],
        ];
        let mut out = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for [x, y] in corners {
            let p = [f64::from(x), f64::from(y), 1.0];
            let h = entry.map.map(|row| row[0] * p[0] + row[1] * p[1] + row[2]);
            if h[2] <= 0.0 || !h.iter().all(|v| v.is_finite()) {
                return None;
            }
            let (px, py) = ((h[0] / h[2]) as f32, (h[1] / h[2]) as f32);
            out = [
                out[0].min(px),
                out[1].min(py),
                out[2].max(px),
                out[3].max(py),
            ];
        }
        let clip = entry.clip;
        rect = [
            out[0].max(clip[0]),
            out[1].max(clip[1]),
            out[2].min(clip[2]),
            out[3].min(clip[3]),
        ];
        if rect[0] >= rect[2] || rect[1] >= rect[3] {
            return None;
        }
    }
    Some(rect)
}

/// Whether text drawn at `layer_depth` can be reported (see [`Renderer::probe_recordable`]).
pub(in crate::render) fn recordable(maps: &[ProbeMap], layer_depth: u32) -> bool {
    match maps.last() {
        None => layer_depth == 0,
        Some(map) => layer_depth == map.depth,
    }
}

/// Probe state saved while the first pass of a translated layer is drawn.
pub(in crate::render) struct SavedLayer {
    shift: (f32, f32),
    clip: [f32; 4],
}

impl<'a> Renderer<'a> {
    /// Called before drawing pass `pass` (0 on black, 1 on white) of an offscreen layer whose
    /// origin is `origin` and which shows only `window` (`[left, top, right, bottom]`, both in the
    /// parent's composition px). Only the first pass is probed; the second is hidden from it.
    pub(in crate::render) fn begin_layer_pass(
        &mut self,
        pass: usize,
        origin: (f32, f32),
        window: Option<[f32; 4]>,
    ) -> Option<SavedLayer> {
        self.isolated += 1;
        if pass > 0 {
            self.layer_depth += 1;
            return None;
        }
        let saved = SavedLayer {
            shift: self.layer_shift,
            clip: self.layer_clip,
        };
        let (sx, sy) = self.layer_shift;
        if let Some([l, t, r, b]) = window {
            let clip = &mut self.layer_clip;
            *clip = [
                clip[0].max(l + sx),
                clip[1].max(t + sy),
                clip[2].min(r + sx),
                clip[3].min(b + sy),
            ];
        }
        self.layer_shift = (sx + origin.0, sy + origin.1);
        Some(saved)
    }

    /// Undoes [`Self::begin_layer_pass`].
    pub(in crate::render) fn end_layer_pass(&mut self, saved: Option<SavedLayer>) {
        self.isolated -= 1;
        match saved {
            Some(saved) => {
                self.layer_shift = saved.shift;
                self.layer_clip = saved.clip;
            }
            None => self.layer_depth -= 1,
        }
    }

    /// Whether text drawn now can be reported: directly on the frame (or a translated layer of
    /// it), or on the innermost probed projective layer.
    pub(in crate::render) fn probe_recordable(&self) -> bool {
        recordable(&self.probe_maps, self.layer_depth)
    }

    /// Starts probing a projective layer whose composition px `to_target` maps to the current
    /// surface's; returns whether it is probed (call [`Self::leave_probe_map`] after it then).
    pub(in crate::render) fn enter_probe_map(&mut self, to_target: &[[f64; 3]; 3]) -> bool {
        if self.probe.is_none() || !self.probe_recordable() {
            return false;
        }
        let (sx, sy) = self.layer_shift;
        let shift = [f64::from(sx), f64::from(sy)];
        let map = std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                to_target[r][c]
                    + if r < 2 {
                        shift[r] * to_target[2][c]
                    } else {
                        0.0
                    }
            })
        });
        self.probe_maps.push(ProbeMap {
            depth: self.layer_depth + 1,
            map,
            clip: self.layer_clip,
            shift: self.layer_shift,
        });
        self.layer_shift = (0.0, 0.0);
        self.layer_clip = [
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::INFINITY,
        ];
        true
    }

    /// Ends [`Self::enter_probe_map`].
    pub(in crate::render) fn leave_probe_map(&mut self) {
        if let Some(map) = self.probe_maps.pop() {
            self.layer_shift = map.shift;
            self.layer_clip = map.clip;
        }
    }

    /// Draws the frame at `time` and returns the text boxes it painted, in draw order.
    pub fn probe_text_at(&mut self, time: f64) -> Result<Vec<TextBox>> {
        if !time.is_finite() || time < 0.0 || time >= self.composition.duration {
            return Err(anyhow!("time is outside the composition duration"));
        }
        // Unchanged element states would reuse the previous frame and skip drawing.
        self.previous.clear();
        self.probe = Some(Vec::new());
        let drawn = self.render_time(time).map(|_| ());
        let boxes = self.probe.take().unwrap_or_default();
        drawn?;
        Ok(boxes)
    }
}
