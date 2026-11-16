//! Layout probe: reports where text is laid out at a given time without changing any pixel.
//!
//! The probe runs the normal draw pass and records each text element's ink box (the union of its
//! line extents, in composition px) as `draw_text` places it. Text drawn into an offscreen layer
//! (rotated, `overflow: hidden`, blurred, or partially revealed elements) uses layer-local
//! coordinates and is not reported. Hidden elements (alpha 0) never reach `draw_text`.
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

impl<'a> Renderer<'a> {
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
