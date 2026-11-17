//! Per-clip audio effects: volume automation (`automation`), a parametric equalizer (`eq`), and
//! sidechain ducking (`duck`).
//!
//! A clip's signal chain is: source (and varispeed) -> equalizer -> `data-volume` x fades x
//! automation x ducking -> mix. `Chain` applies it sample by sample inside the mixer; a clip
//! without effects skips the chain, so its output is byte-identical to the plain mix.
pub mod automation;
pub mod duck;
pub mod eq;

use std::sync::Arc;

/// The effects authored on one `<audio>` or `<video>` sound.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioEffects {
    /// `data-hf-volume-ramp`.
    pub volume: Option<Arc<automation::VolumeCurve>>,
    /// `data-hf-eq`.
    pub eq: Option<Arc<eq::Equalizer>>,
    /// `data-hf-duck` and its options.
    pub duck: Option<duck::Duck>,
}

impl AudioEffects {
    pub fn is_empty(&self) -> bool {
        self.volume.is_none() && self.eq.is_none() && self.duck.is_none()
    }
}

/// Linear gain in dB, rounded to 0.01 dB; `null` for silence.
pub fn gain_db(gain: f64) -> serde_json::Value {
    if gain <= 0.0 {
        serde_json::Value::Null
    } else {
        serde_json::json!((20.0 * gain.log10() * 100.0).round() / 100.0)
    }
}

/// Control block in samples: volume automation is evaluated at block edges (1.5 kHz at 48 kHz)
/// and interpolated linearly inside, and equalizer states are flushed once per block.
pub const BLOCK: usize = 32;

/// Volume automation read at control rate.
struct VolumeCursor<'a> {
    curve: &'a automation::VolumeCurve,
    /// First clip sample of the current block, and the gains at its two edges.
    block: usize,
    from: f64,
    to: f64,
}

impl VolumeCursor<'_> {
    #[inline]
    fn gain(&mut self, k: usize) -> f64 {
        if k < self.block || k >= self.block + BLOCK {
            let rate = f64::from(super::SAMPLE_RATE);
            self.block = k - k % BLOCK;
            self.from = self.curve.gain(self.block as f64 / rate);
            self.to = self.curve.gain((self.block + BLOCK) as f64 / rate);
        }
        if self.from == self.to {
            return self.from;
        }
        self.from + (self.to - self.from) * ((k - self.block) as f64 / BLOCK as f64)
    }
}

/// Running effect chain of one clip in the mixer.
pub(crate) struct Chain<'a> {
    eq: Option<[eq::EqState; 2]>,
    /// Mono sources run the equalizer once and copy the result to both channels.
    mono: bool,
    volume: Option<VolumeCursor<'a>>,
    duck: Option<Vec<f32>>,
}

impl<'a> Chain<'a> {
    pub(crate) fn new(effects: &'a AudioEffects, mono: bool, duck: Option<Vec<f32>>) -> Self {
        Self {
            eq: effects.eq.as_ref().map(|e| [e.state(), e.state()]),
            mono,
            volume: effects.volume.as_deref().map(|curve| VolumeCursor {
                curve,
                block: usize::MAX / 2,
                from: 1.0,
                to: 1.0,
            }),
            duck,
        }
    }

    /// Processes clip sample `k` with base gain `gain` (volume and fades); returns the samples
    /// to add to the mix. `k` only increases within a clip.
    #[inline]
    pub(crate) fn apply(&mut self, k: usize, l: f32, r: f32, mut gain: f64) -> (f32, f32) {
        let (mut l, mut r) = (f64::from(l), f64::from(r));
        if let Some([left, right]) = &mut self.eq {
            if k.is_multiple_of(BLOCK) {
                left.flush();
                right.flush();
            }
            l = left.process(l);
            r = if self.mono { l } else { right.process(r) };
        }
        if let Some(volume) = &mut self.volume {
            gain *= volume.gain(k);
        }
        if let Some(duck) = &self.duck {
            gain *= f64::from(duck[k]);
        }
        ((l * gain) as f32, (r * gain) as f32)
    }
}
