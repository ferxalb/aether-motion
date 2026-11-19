//! Motion blur (`data-motion-blur` on the root): a virtual shutter that averages sub-frame samples.
//!
//! Each output frame draws the scene at several instants spread evenly over a window of
//! `shutter` degrees of one frame interval, centered on the frame time, and averages them per
//! channel with rounding. Without `data-motion-blur-samples`, the count follows the largest travel
//! of any element across the window: about one sample per 2 output px, from 8 to 64. Sample times depend only on the frame time and fps, so output stays
//! deterministic. The root's color grade, vignette, and grain run once on the averaged frame.
//! When nothing moves inside the window (no visible element changes position, scale, rotation, or
//! reveal; fades, filters, and video frames do not count as movement) the frame is drawn once at the
//! frame time, and the identical-frame shortcut still applies.
//!
//! Sums are kept in 16-bit channels (at most 64 samples of 255), so a 4K frame needs 66 MiB.

use super::frame_state::FrameState;
use super::Renderer;
use crate::model::MotionBlur;
use anyhow::Result;

/// Whether any element visible in two samples (the `center` state counts as one) has a different
/// geometry in them, or is visible at the center but in none of the samples. Drawing once at the
/// center is only equivalent to the average when this is false.
pub(in crate::render) fn moves(samples: &[Vec<FrameState>], center: &[FrameState]) -> bool {
    let Some(first) = samples.first() else {
        return false;
    };
    (0..first.len()).any(|i| {
        if center[i].alpha > 0.0 && samples.iter().all(|s| s[i].alpha <= 0.0) {
            return true;
        }
        let mut visible = samples
            .iter()
            .map(|s| s[i])
            .chain(std::iter::once(center[i]))
            .filter(|s| s.alpha > 0.0);
        let Some(reference) = visible.next() else {
            return false;
        };
        visible.any(|s| {
            (s.x, s.y, s.scale, s.rotate, s.reveal)
                != (
                    reference.x,
                    reference.y,
                    reference.scale,
                    reference.rotate,
                    reference.reveal,
                )
        })
    })
}

/// Largest distance (composition px) any visible element travels between two state sets.
/// Rotation and scale count as the movement of a point a quarter of the frame's short side away.
pub(in crate::render) fn travel(from: &[FrameState], to: &[FrameState], short_side: u32) -> f32 {
    let reach = short_side as f32 / 4.0;
    from.iter()
        .zip(to)
        .filter(|(a, b)| a.alpha > 0.0 && b.alpha > 0.0)
        .map(|(a, b)| {
            let shift = (a.x - b.x).hypot(a.y - b.y);
            let turn = (a.rotate - b.rotate).to_radians().abs() * reach;
            let zoom = (a.scale - b.scale).abs() * reach;
            shift + turn + zoom
        })
        .fold(0.0, f32::max)
}

/// Frame rate assumed for the shutter window of a single snapshot (`render_at`), which has no fps.
pub(in crate::render) const SNAPSHOT_FPS: u32 = 30;

impl<'a> Renderer<'a> {
    /// Draws the frame at `time`, motion-blurred over the shutter window at `fps` when enabled.
    pub(in crate::render) fn render_shuttered(&mut self, time: f64, fps: u32) -> Result<&[u8]> {
        self.fps = fps;
        let Some(blur) = self.composition.motion_blur else {
            return self.render_time(time);
        };
        let duration = self.composition.duration;
        let n = match blur.samples {
            Some(n) => n,
            None => {
                let (first, last) = blur.window(time, fps, duration);
                self.sample_states(first);
                let start = self.current.clone();
                self.sample_states(last);
                let travel = travel(
                    &start,
                    &self.current,
                    self.composition.width.min(self.composition.height),
                );
                let (low, high) = MotionBlur::AUTO_SAMPLES;
                ((travel * self.scale / 2.0).ceil() as u32).clamp(low, high)
            }
        };
        let times = blur.sample_times(time, fps, duration, n);
        let mut samples: Vec<Vec<FrameState>> = Vec::with_capacity(times.len());
        for &t in &times {
            self.sample_states(t);
            samples.push(self.current.clone());
        }
        self.sample_states(time);
        if !moves(&samples, &self.current) {
            return self.render_time(time);
        }
        let len = self.surface.pixels.len();
        self.shutter_sums.clear();
        self.shutter_sums.resize(len, 0);
        let mut drawn = Ok(());
        for states in samples {
            self.current = states;
            drawn = self.draw_scene();
            if drawn.is_err() {
                break;
            }
            for (sum, &value) in self.shutter_sums.iter_mut().zip(&self.surface.pixels) {
                *sum += u16::from(value);
            }
        }
        // The surface no longer matches any single state, so the next frame must redraw.
        self.previous.clear();
        drawn?;
        let n = times.len() as u32;
        for (pixel, &sum) in self.surface.pixels.iter_mut().zip(&self.shutter_sums) {
            *pixel = ((u32::from(sum) + n / 2) / n) as u8;
        }
        self.surface.mark_all_painted();
        self.finish_frame(time);
        self.stats.rasterized += 1;
        Ok(&self.surface.pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, rotate: f32) -> FrameState {
        FrameState {
            alpha: 1.0,
            x,
            scale: 1.0,
            rotate,
            ..FrameState::default()
        }
    }

    #[test]
    fn only_geometry_counts_as_movement() {
        let faded = FrameState {
            alpha: 0.5,
            filters: [3.0; 8],
            ..at(10.0, 0.0)
        };
        let hidden = FrameState::default();
        let rest = [at(10.0, 0.0)];
        assert!(!moves(
            &[vec![at(10.0, 0.0)], vec![faded], vec![hidden]],
            &rest
        ));
        assert!(moves(&[vec![at(10.0, 0.0)], vec![at(10.0, 5.0)]], &rest));
    }

    #[test]
    fn center_state_must_match_the_samples() {
        let samples = [vec![at(10.0, 0.0)], vec![at(10.0, 0.0)]];
        assert!(moves(&samples, &[at(20.0, 0.0)]));
        let hidden = [vec![FrameState::default()], vec![FrameState::default()]];
        assert!(moves(&hidden, &[at(10.0, 0.0)]));
        assert!(!moves(&hidden, &[FrameState::default()]));
    }

    #[test]
    fn travel_takes_the_fastest_visible_element() {
        let from = [at(0.0, 0.0), at(10.0, 0.0), FrameState::default()];
        let to = [at(30.0, 0.0), at(10.0, 90.0), at(500.0, 0.0)];
        // 90 degrees at a quarter of a 400 px side is pi/2 * 100 px; the hidden element is ignored.
        let expected = std::f32::consts::FRAC_PI_2 * 100.0;
        assert!((travel(&from, &to, 400) - expected).abs() < 1e-3);
    }

    #[test]
    fn samples_are_centered_on_the_frame_and_clamped() {
        let blur = MotionBlur {
            shutter: 180.0,
            samples: None,
        };
        let times = blur.sample_times(1.0, 10, 5.0, 4);
        let expected = [0.98125, 0.99375, 1.00625, 1.01875];
        for (t, e) in times.iter().zip(expected) {
            assert!((t - e).abs() < 1e-9, "{times:?}");
        }
        assert_eq!(blur.window(0.0, 10, 5.0).0, 0.0);
        assert!(blur.window(4.99, 10, 5.0).1 < 5.0);
    }
}
