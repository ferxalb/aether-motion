//! `data-hf-draw`: an inline `<svg>` whose strokes draw themselves. Each shape's stroke is revealed
//! along its length with a dash pattern (`stroke-dasharray: L L+1; stroke-dashoffset: L·(1-p)`),
//! injected into the markup for the current instant; shapes start one after another by `stagger`.
//!
//! The markup is prepared once at parse time with a placeholder in every shape's `style`, and the
//! stroke lengths are measured from the parsed SVG (in each shape's own user space, where dash
//! lengths apply). Once every stroke is complete the original markup is drawn through the normal
//! image cache.

use super::measure::{visit_paths, PathMeasure};
use crate::model::Easing;
use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

/// Placeholder written into a shape's `style` at parse time; `{index}` is the shape's number.
pub fn token(index: usize) -> String {
    format!("__aether_draw_{index}__")
}

/// Timing of a stroke reveal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawTiming {
    /// Seconds from the clip start at which the first shape starts drawing.
    pub at: f64,
    /// Seconds each shape takes to draw.
    pub duration: f64,
    /// Seconds between the starts of consecutive shapes (document order).
    pub stagger: f64,
    pub easing: Easing,
}

/// A prepared inline `<svg>` with self-drawing strokes.
#[derive(Debug)]
pub struct DrawOnSvg {
    /// The finished drawing (placeholders removed).
    pub markup: Arc<str>,
    /// Markup around the placeholders: `parts[i]` comes before shape `i`'s style, and the last
    /// part closes the document.
    parts: Vec<String>,
    /// Stroke length of each shape in its user space; 0 when it draws nothing.
    lengths: Vec<f32>,
    pub timing: DrawTiming,
}

impl DrawOnSvg {
    /// Builds the reveal from markup holding `ids.len()` placeholders (in order) for shapes with
    /// those ids.
    pub fn new(markup: &str, ids: &[String], timing: DrawTiming) -> Result<Self> {
        let mut parts = Vec::with_capacity(ids.len() + 1);
        let mut rest = markup;
        for index in 0..ids.len() {
            let mark = token(index);
            let (before, after) = rest.split_once(&mark).ok_or_else(|| {
                anyhow::anyhow!("data-hf-draw: lost a shape while preparing the SVG")
            })?;
            parts.push(before.to_string());
            rest = after;
        }
        parts.push(rest.to_string());
        let finished: String = parts.concat();
        let tree = crate::render::svg::parse_svg(finished.as_bytes())?;
        // usvg can split a shape into a fill and a stroke path with the same id; keep the longest.
        let mut by_id: HashMap<String, f32> = HashMap::new();
        let mut failure = None;
        visit_paths(tree.root(), &mut |path| {
            if path.id().is_empty() {
                return;
            }
            match PathMeasure::new(path.data()) {
                Ok(measure) => {
                    let length = by_id.entry(path.id().to_string()).or_default();
                    *length = length.max(measure.length());
                }
                Err(error) => failure = Some(error),
            }
        });
        if let Some(error) = failure {
            return Err(error.context("data-hf-draw"));
        }
        let lengths = ids
            .iter()
            .map(|id| by_id.get(id).copied().unwrap_or(0.0))
            .collect();
        Ok(Self {
            markup: finished.into(),
            parts,
            lengths,
            timing,
        })
    }

    /// Number of prepared shapes.
    pub fn shapes(&self) -> usize {
        self.lengths.len()
    }

    /// Seconds from the clip start at which the last shape finishes.
    pub fn end(&self) -> f64 {
        let t = self.timing;
        t.at + t.stagger * self.shapes().saturating_sub(1) as f64 + t.duration
    }

    /// Drawn share (0..=1) of shape `index`, `elapsed` seconds after the clip start.
    fn progress(&self, index: usize, elapsed: f64) -> f32 {
        let t = self.timing;
        let start = t.at + t.stagger * index as f64;
        if t.duration <= 0.0 {
            return if elapsed >= start { 1.0 } else { 0.0 };
        }
        let linear = ((elapsed - start) / t.duration).clamp(0.0, 1.0) as f32;
        t.easing.apply(linear).clamp(0.0, 1.0)
    }

    /// A key that changes whenever the drawing does: 0 before any stroke starts, 1 once all are
    /// complete, and a time-based value in between.
    pub fn key(&self, elapsed: f64) -> u32 {
        if elapsed < self.timing.at {
            0
        } else if elapsed >= self.end() {
            1
        } else {
            2 + ((elapsed - self.timing.at) * 1e5) as u32
        }
    }

    /// Markup for the instant `elapsed`, or `None` when every stroke is complete (draw
    /// [`Self::markup`] then).
    pub fn markup_at(&self, elapsed: f64) -> Option<String> {
        if elapsed >= self.end() {
            return None;
        }
        let mut out = String::with_capacity(self.markup.len() + self.shapes() * 64);
        for (index, part) in self.parts.iter().enumerate() {
            out.push_str(part);
            let Some(&length) = self.lengths.get(index) else {
                continue;
            };
            let p = self.progress(index, elapsed);
            if length <= 0.0 || p >= 1.0 {
                continue;
            }
            if p <= 0.0 {
                out.push_str("stroke:none");
                continue;
            }
            let _ = write!(
                out,
                "stroke-dasharray:{:.4} {:.4};stroke-dashoffset:{:.4}",
                length,
                length + 1.0,
                length * (1.0 - p)
            );
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DrawOnSvg {
        let markup = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path id="a" d="M0 0 H100" stroke="red" style="{}"/><circle id="b" cx="50" cy="50" r="10" stroke="red" fill="none" style="{}"/></svg>"#,
            token(0),
            token(1)
        );
        DrawOnSvg::new(
            &markup,
            &["a".into(), "b".into()],
            DrawTiming {
                at: 0.5,
                duration: 1.0,
                stagger: 0.5,
                easing: Easing::Linear,
            },
        )
        .unwrap()
    }

    #[test]
    fn measures_each_shape() {
        let d = sample();
        assert_eq!(d.lengths[0], 100.0);
        assert!((d.lengths[1] - std::f32::consts::TAU * 10.0).abs() < 0.2);
        assert!(!d.markup.contains("__aether_draw"));
        assert_eq!(d.end(), 2.0);
    }

    #[test]
    fn reveals_strokes_in_order() {
        let d = sample();
        let start = d.markup_at(0.0).unwrap();
        assert_eq!(start.matches("stroke:none").count(), 2);
        let middle = d.markup_at(1.0).unwrap();
        assert!(middle.contains("stroke-dasharray:100.0000 101.0000;stroke-dashoffset:50.0000"));
        assert!(
            middle.contains("stroke:none"),
            "the second shape has not started"
        );
        assert!(d.markup_at(2.0).is_none());
        assert_eq!((d.key(0.2), d.key(2.5)), (0, 1));
        assert_ne!(d.key(1.0), d.key(1.01));
    }
}
