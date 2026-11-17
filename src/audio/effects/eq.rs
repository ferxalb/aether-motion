//! Parametric equalizer (`data-hf-eq`): a cascade of up to 16 biquad bands at 48 kHz.
//!
//! Coefficients follow the RBJ Audio EQ Cookbook (high-pass, low-pass, band-pass with 0 dB peak,
//! notch, peaking, low shelf, high shelf). Filtering runs in `f64` in transposed direct form II,
//! with state starting at zero at the clip start and tiny state values flushed to zero every block
//! (`effects::BLOCK` samples) so long silences never fall into denormals. Named presets expand to fixed band lists.
use super::super::SAMPLE_RATE;
use anyhow::{bail, Result};
use std::f64::consts::PI;

/// Most bands in one equalizer.
pub const MAX_BANDS: usize = 16;
/// Accepted center or corner frequencies, in Hz.
pub const MIN_FREQ: f64 = 10.0;
pub const MAX_FREQ: f64 = 20_000.0;
/// Accepted band gain, in dB.
pub const MAX_GAIN_DB: f64 = 24.0;
/// Accepted quality factor.
pub const MIN_Q: f64 = 0.1;
pub const MAX_Q: f64 = 30.0;
/// Butterworth Q, the default of the pass and shelf filters.
pub const BUTTERWORTH_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Named presets accepted by `data-hf-eq`.
pub const PRESETS: [&str; 5] = ["voice", "music-bed", "telephone", "warm", "bright"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandKind {
    HighPass,
    LowPass,
    BandPass,
    Notch,
    Peak,
    LowShelf,
    HighShelf,
}

impl BandKind {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "highpass" => Self::HighPass,
            "lowpass" => Self::LowPass,
            "bandpass" => Self::BandPass,
            "notch" => Self::Notch,
            "peak" => Self::Peak,
            "lowshelf" => Self::LowShelf,
            "highshelf" => Self::HighShelf,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::HighPass => "highpass",
            Self::LowPass => "lowpass",
            Self::BandPass => "bandpass",
            Self::Notch => "notch",
            Self::Peak => "peak",
            Self::LowShelf => "lowshelf",
            Self::HighShelf => "highshelf",
        }
    }

    /// Whether the band takes a `gain` in dB.
    pub fn has_gain(self) -> bool {
        matches!(self, Self::Peak | Self::LowShelf | Self::HighShelf)
    }

    pub fn default_q(self) -> f64 {
        match self {
            Self::BandPass | Self::Notch | Self::Peak => 1.0,
            _ => BUTTERWORTH_Q,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub kind: BandKind,
    pub freq: f64,
    pub gain_db: f64,
    pub q: f64,
}

impl Band {
    pub fn new(kind: BandKind, freq: f64, gain_db: f64, q: f64) -> Result<Self> {
        if !(MIN_FREQ..=MAX_FREQ).contains(&freq) {
            bail!("freq must be between {MIN_FREQ} and {MAX_FREQ} Hz");
        }
        if !(-MAX_GAIN_DB..=MAX_GAIN_DB).contains(&gain_db) {
            bail!("gain must be between -{MAX_GAIN_DB} and {MAX_GAIN_DB} dB");
        }
        if !(MIN_Q..=MAX_Q).contains(&q) {
            bail!("q must be between {MIN_Q} and {MAX_Q}");
        }
        Ok(Self {
            kind,
            freq,
            gain_db,
            q,
        })
    }

    /// Normalized biquad coefficients `[b0, b1, b2, a1, a2]`.
    fn coefficients(&self) -> [f64; 5] {
        let w0 = 2.0 * PI * self.freq / f64::from(SAMPLE_RATE);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * self.q);
        let a = 10f64.powf(self.gain_db / 40.0);
        let shelf = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match self.kind {
            BandKind::LowPass => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            BandKind::HighPass => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            BandKind::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BandKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            BandKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            BandKind::LowShelf => (
                a * ((a + 1.0) - (a - 1.0) * cos + shelf),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - shelf),
                (a + 1.0) + (a - 1.0) * cos + shelf,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - shelf,
            ),
            BandKind::HighShelf => (
                a * ((a + 1.0) + (a - 1.0) * cos + shelf),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - shelf),
                (a + 1.0) - (a - 1.0) * cos + shelf,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - shelf,
            ),
        };
        [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
    }
}

/// An equalizer: its bands (as authored or from a preset) and their coefficients.
#[derive(Clone, Debug, PartialEq)]
pub struct Equalizer {
    /// Preset name when the bands come from one.
    pub preset: Option<&'static str>,
    /// Private so the bands can never drift from `coefficients`; build a new `Equalizer` to change them.
    bands: Vec<Band>,
    coefficients: Vec<[f64; 5]>,
}

impl Equalizer {
    pub fn new(bands: Vec<Band>, preset: Option<&'static str>) -> Result<Self> {
        if bands.is_empty() || bands.len() > MAX_BANDS {
            bail!("needs 1 to {MAX_BANDS} bands");
        }
        let coefficients = bands.iter().map(Band::coefficients).collect();
        Ok(Self {
            preset,
            bands,
            coefficients,
        })
    }

    /// The bands, as authored or expanded from the preset.
    pub fn bands(&self) -> &[Band] {
        &self.bands
    }

    /// The bands of a named preset, or `None` for an unknown name.
    pub fn preset(name: &str) -> Option<Self> {
        use BandKind::*;
        let (name, bands): (&'static str, &[(BandKind, f64, f64, f64)]) = match name {
            // Rumble cut, less boxiness, presence and air: speech that sits on top of a mix.
            "voice" => (
                "voice",
                &[
                    (HighPass, 80.0, 0.0, BUTTERWORTH_Q),
                    (Peak, 250.0, -2.0, 1.0),
                    (Peak, 3500.0, 3.0, 0.9),
                    (HighShelf, 10_000.0, 2.0, BUTTERWORTH_Q),
                ],
            ),
            // A music bed that leaves room for speech: sub cut and a wide dip at speech presence.
            "music-bed" => (
                "music-bed",
                &[
                    (HighPass, 40.0, 0.0, BUTTERWORTH_Q),
                    (Peak, 2500.0, -5.0, 0.7),
                    (HighShelf, 12_000.0, -1.0, BUTTERWORTH_Q),
                ],
            ),
            "telephone" => (
                "telephone",
                &[
                    (HighPass, 300.0, 0.0, BUTTERWORTH_Q),
                    (LowPass, 3400.0, 0.0, BUTTERWORTH_Q),
                    (Peak, 1500.0, 3.0, 1.0),
                ],
            ),
            "warm" => (
                "warm",
                &[
                    (LowShelf, 200.0, 3.0, BUTTERWORTH_Q),
                    (HighShelf, 8000.0, -2.0, BUTTERWORTH_Q),
                ],
            ),
            "bright" => (
                "bright",
                &[
                    (LowShelf, 120.0, -2.0, BUTTERWORTH_Q),
                    (HighShelf, 6000.0, 4.0, BUTTERWORTH_Q),
                ],
            ),
            _ => return None,
        };
        let bands = bands
            .iter()
            .map(|&(kind, freq, gain_db, q)| Band {
                kind,
                freq,
                gain_db,
                q,
            })
            .collect();
        Some(Self::new(bands, Some(name)).expect("presets are valid"))
    }

    /// Magnitude response of the whole cascade at `freq` Hz, in dB.
    pub fn response_db(&self, freq: f64) -> f64 {
        let w = 2.0 * PI * freq / f64::from(SAMPLE_RATE);
        // |H(e^jw)|^2 for each biquad, from z^-1 = e^-jw.
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let mut db = 0.0;
        for [b0, b1, b2, a1, a2] in &self.coefficients {
            let (nr, ni) = (b0 + b1 * c1 + b2 * c2, -(b1 * s1 + b2 * s2));
            let (dr, di) = (1.0 + a1 * c1 + a2 * c2, -(a1 * s1 + a2 * s2));
            db += 10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10();
        }
        db
    }

    /// A fresh filter state for one channel.
    pub fn state(&self) -> EqState {
        EqState {
            coefficients: self.coefficients.clone(),
            z: vec![[0.0; 2]; self.coefficients.len()],
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        let bands: Vec<_> = self
            .bands
            .iter()
            .map(|b| {
                let mut band = serde_json::json!({"type": b.kind.name(), "freq": b.freq, "q": b.q});
                if b.kind.has_gain() {
                    band["gain"] = serde_json::json!(b.gain_db);
                }
                band
            })
            .collect();
        let response: serde_json::Map<String, serde_json::Value> = [
            50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10_000.0, 16_000.0,
        ]
        .iter()
        .map(|&f| {
            let db = (self.response_db(f) * 100.0).round() / 100.0;
            (format!("{f}"), serde_json::json!(db))
        })
        .collect();
        serde_json::json!({"preset": self.preset, "bands": bands, "responseDb": response})
    }
}

/// Running state of one channel through the cascade.
pub struct EqState {
    coefficients: Vec<[f64; 5]>,
    z: Vec<[f64; 2]>,
}

/// States below this magnitude are flushed to zero (about -500 dBFS).
const FLUSH: f64 = 1e-25;

impl EqState {
    #[inline]
    pub fn process(&mut self, mut x: f64) -> f64 {
        for ([b0, b1, b2, a1, a2], z) in self.coefficients.iter().zip(self.z.iter_mut()) {
            let y = b0 * x + z[0];
            z[0] = b1 * x - a1 * y + z[1];
            z[1] = b2 * x - a2 * y;
            x = y;
        }
        x
    }

    /// Flushes tiny states to zero; the mixer calls it once per block, which keeps decaying
    /// tails out of denormals without a branch per sample.
    pub fn flush(&mut self) {
        for z in self.z.iter_mut().flatten() {
            if z.abs() < FLUSH {
                *z = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq(kind: BandKind, freq: f64, gain: f64) -> Equalizer {
        Equalizer::new(
            vec![Band::new(kind, freq, gain, kind.default_q()).unwrap()],
            None,
        )
        .unwrap()
    }

    /// RMS gain in dB of a steady sine at `freq` through the filter (after settling).
    fn measured_db(eq: &Equalizer, freq: f64) -> f64 {
        let mut state = eq.state();
        let n = 48_000;
        let (mut input, mut output) = (0.0, 0.0);
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f64 / 48_000.0).sin();
            let y = state.process(x);
            if i >= n / 2 {
                input += x * x;
                output += y * y;
            }
        }
        10.0 * (output / input).log10()
    }

    #[test]
    fn peak_band_reaches_its_gain_at_the_center() {
        let e = eq(BandKind::Peak, 1000.0, 6.0);
        assert!((e.response_db(1000.0) - 6.0).abs() < 1e-9);
        assert!((measured_db(&e, 1000.0) - 6.0).abs() < 0.05);
        assert!(measured_db(&e, 50.0).abs() < 0.1);
    }

    #[test]
    fn pass_filters_cut_three_db_at_the_corner_and_more_beyond() {
        let hp = eq(BandKind::HighPass, 200.0, 0.0);
        assert!((hp.response_db(200.0) + 3.0103).abs() < 0.01);
        assert!(measured_db(&hp, 50.0) < -23.0);
        assert!(measured_db(&hp, 5000.0).abs() < 0.05);
        let lp = eq(BandKind::LowPass, 2000.0, 0.0);
        assert!(measured_db(&lp, 10_000.0) < -25.0);
    }

    #[test]
    fn shelves_settle_at_their_gain() {
        let low = eq(BandKind::LowShelf, 200.0, -6.0);
        assert!((low.response_db(20.0) + 6.0).abs() < 0.1);
        assert!(low.response_db(10_000.0).abs() < 0.05);
        let high = eq(BandKind::HighShelf, 4000.0, 4.0);
        assert!((high.response_db(20_000.0) - 4.0).abs() < 0.2);
    }

    #[test]
    fn presets_exist_and_telephone_band_limits() {
        for name in PRESETS {
            assert!(Equalizer::preset(name).is_some(), "{name}");
        }
        let phone = Equalizer::preset("telephone").unwrap();
        assert!(phone.response_db(100.0) < -15.0);
        assert!(phone.response_db(10_000.0) < -15.0);
        assert!(phone.response_db(1500.0) > 0.0);
    }

    #[test]
    fn rejects_out_of_range_bands() {
        assert!(Band::new(BandKind::Peak, 5.0, 0.0, 1.0).is_err());
        assert!(Band::new(BandKind::Peak, 1000.0, 30.0, 1.0).is_err());
        assert!(Band::new(BandKind::Peak, 1000.0, 0.0, 0.0).is_err());
    }
}
