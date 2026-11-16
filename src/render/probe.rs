//! Layout probe: reports where text is laid out at a given time without changing any pixel.
//!
//! The probe runs the normal draw pass and records each text element's ink box (the union of its
//! line extents, in composition px) as `draw_text` places it. Offscreen layers that only translate
//! (`overflow: hidden`, blur, and partial reveals) are probed on their first drawing pass: the
//! layer origin is added back and the box is cut to the part the layer shows. Text inside rotated
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
