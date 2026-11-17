//! Sidechain ducking (`data-hf-duck`): lowers a clip (typically music) while other clips (the
//! voice) are sounding.
//!
//! The key is the post-effects output of the trigger clips: a trigger is active at a sample when
//! its louder channel exceeds the threshold. Activity is held for `hold` seconds (bridging the
//! gaps between syllables), the reduction then returns to zero over `release`, and because the
//! whole mix is known in advance the reduction starts `attack` seconds *before* the voice (look
//! ahead), so the music is already down when the first word lands. The 0..1 reduction is shaped
//! with smoothstep and applied in decibels (`gain = 10^(-amount * r / 20)`). Everything is integer
//! sample arithmetic plus two linear scans, so the envelope is exact and deterministic.
use super::super::SAMPLE_RATE;

/// Defaults and limits of the `data-hf-duck-*` options.
pub const DEFAULT_AMOUNT_DB: f64 = 12.0;
pub const MAX_AMOUNT_DB: f64 = 60.0;
pub const DEFAULT_THRESHOLD_DB: f64 = -40.0;
pub const MIN_THRESHOLD_DB: f64 = -90.0;
pub const DEFAULT_ATTACK: f64 = 0.15;
pub const DEFAULT_HOLD: f64 = 0.15;
pub const DEFAULT_RELEASE: f64 = 0.5;
pub const MAX_TIME: f64 = 5.0;
/// Most ducked regions listed in a report.
const MAX_REPORTED_REGIONS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Duck {
    /// Trigger clip ids as authored.
    pub by: Vec<String>,
    /// Trigger clip indices into `Composition::audio`, resolved by the parser.
    pub sources: Vec<usize>,
    pub amount_db: f64,
    pub threshold_db: f64,
    pub attack: f64,
    pub hold: f64,
    pub release: f64,
}

/// Post-effects level (louder channel, absolute) of one trigger clip, from `start` on the timeline.
pub struct Key {
    pub start: usize,
    pub level: Vec<f32>,
}

/// Per-sample gains for a ducked clip, with what `check` reports.
pub struct Envelope {
    pub gains: Vec<f32>,
    /// Timeline sample ranges where the reduction is at least half the amount.
    pub regions: Vec<(usize, usize)>,
}

fn samples(seconds: f64) -> usize {
    (seconds * f64::from(SAMPLE_RATE)).round() as usize
}

impl Duck {
    /// Gains for the clip that covers `start..start + length` of a `total`-sample timeline.
    pub fn envelope(&self, keys: &[&Key], start: usize, length: usize, total: usize) -> Envelope {
        let (attack, hold, release) = (
            samples(self.attack),
            samples(self.hold),
            samples(self.release),
        );
        let threshold = 10f64.powf(self.threshold_db / 20.0) as f32;
        let lo = start.saturating_sub(hold + release);
        let hi = (start + length + attack).min(total);
        let mut active = vec![false; hi - lo];
        for key in keys {
            let from = key.start.max(lo);
            let to = (key.start + key.level.len()).min(hi);
            for n in from..to {
                if key.level[n - key.start] > threshold {
                    active[n - lo] = true;
                }
            }
        }
        // Forward: hold, then release after the last active sample.
        let mut reduction = vec![0.0_f64; hi - lo];
        let mut last = None;
        for (i, r) in reduction.iter_mut().enumerate() {
            if active[i] {
                last = Some(i);
            }
            if let Some(m) = last {
                let since = i - m;
                *r = if since <= hold {
                    1.0
                } else if since < hold + release {
                    1.0 - (since - hold) as f64 / release as f64
                } else {
                    0.0
                };
            }
        }
        // Backward: look ahead by the attack before the next active sample.
        let mut next = None;
        for i in (0..reduction.len()).rev() {
            if active[i] {
                next = Some(i);
            }
            if let Some(m) = next {
                let until = m - i;
                if until < attack.max(1) {
                    let rise = if attack == 0 {
                        1.0
                    } else {
                        1.0 - until as f64 / attack as f64
                    };
                    reduction[i] = reduction[i].max(rise);
                }
            }
        }
        let floor = 10f64.powf(-self.amount_db / 20.0) as f32;
        // 10^(-amount * s / 20) as one exp per ramp sample.
        let slope = -self.amount_db * std::f64::consts::LN_10 / 20.0;
        let mut gains = Vec::with_capacity(length);
        let mut regions = Vec::new();
        let mut open: Option<usize> = None;
        for k in 0..length {
            let n = start + k;
            let r = if n < hi { reduction[n - lo] } else { 0.0 };
            let shaped = r * r * (3.0 - 2.0 * r);
            gains.push(if shaped <= 0.0 {
                1.0
            } else if shaped >= 1.0 {
                floor
            } else {
                (slope * shaped).exp() as f32
            });
            match (shaped >= 0.5, open) {
                (true, None) => open = Some(n),
                (false, Some(from)) => {
                    regions.push((from, n));
                    open = None;
                }
                _ => {}
            }
        }
        if let Some(from) = open {
            regions.push((from, start + length));
        }
        Envelope { gains, regions }
    }
}

impl Envelope {
    pub fn to_json(&self, duck: &Duck) -> serde_json::Value {
        let rate = f64::from(SAMPLE_RATE);
        let ducked: usize = self.regions.iter().map(|(a, b)| b - a).sum();
        let regions: Vec<_> = self
            .regions
            .iter()
            .take(MAX_REPORTED_REGIONS)
            .map(|&(a, b)| serde_json::json!([a as f64 / rate, b as f64 / rate]))
            .collect();
        let deepest = self.gains.iter().copied().fold(1.0_f32, f32::min);
        serde_json::json!({
            "by": duck.by,
            "amountDb": duck.amount_db,
            "thresholdDb": duck.threshold_db,
            "attack": duck.attack,
            "hold": duck.hold,
            "release": duck.release,
            "duckedSeconds": ducked as f64 / rate,
            "regions": regions,
            "deepestDb": super::gain_db(f64::from(deepest)),
        })
    }
}

/// Clip indices in mixing order: every trigger before the clips it ducks, otherwise document
/// order. `ducks[i]` lists the triggers of clip `i`. Fails on a ducking cycle.
pub fn processing_order(ducks: &[&[usize]]) -> Result<Vec<usize>, usize> {
    let n = ducks.len();
    let mut waiting: Vec<usize> = ducks.iter().map(|d| d.len()).collect();
    let mut order = Vec::with_capacity(n);
    let mut done = vec![false; n];
    while order.len() < n {
        let ready = (0..n).find(|&i| !done[i] && waiting[i] == 0);
        let Some(i) = ready else {
            return Err((0..n).find(|&i| !done[i]).unwrap_or(0));
        };
        done[i] = true;
        order.push(i);
        for (j, triggers) in ducks.iter().enumerate() {
            waiting[j] -= triggers.iter().filter(|&&t| t == i).count();
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duck(attack: f64, hold: f64, release: f64) -> Duck {
        Duck {
            by: vec!["voice".into()],
            sources: vec![0],
            amount_db: 20.0,
            threshold_db: -40.0,
            attack,
            hold,
            release,
        }
    }

    #[test]
    fn ducks_ahead_of_the_voice_and_releases_after_the_hold() {
        // Voice active from 1.0 s to 2.0 s; a 3 s clip from 0.
        let mut level = vec![0.0_f32; 3 * 48_000];
        level[48_000..96_000].fill(0.5);
        let key = Key { start: 0, level };
        let env = duck(0.1, 0.2, 0.5).envelope(&[&key], 0, 3 * 48_000, 3 * 48_000);
        let floor = 0.1_f32; // -20 dB
        assert_eq!(env.gains[0], 1.0);
        assert_eq!(
            env.gains[48_000 - 4800 - 1],
            1.0,
            "nothing before the look-ahead"
        );
        assert!(
            (env.gains[48_000] - floor).abs() < 1e-6,
            "fully down when the voice starts"
        );
        assert!((env.gains[96_000 + 9600] - floor).abs() < 1e-6, "held");
        assert!(env.gains[96_000 + 9600 + 12_000] > floor);
        assert_eq!(env.gains[96_000 + 9600 + 24_000 + 1], 1.0, "released");
        assert_eq!(env.regions.len(), 1);
    }

    #[test]
    fn orders_triggers_first_and_rejects_cycles() {
        assert_eq!(processing_order(&[&[1], &[], &[]]), Ok(vec![1, 0, 2]));
        assert_eq!(processing_order(&[&[1], &[0]]), Err(0));
    }

    #[test]
    fn quiet_keys_below_the_threshold_do_not_duck() {
        let key = Key {
            start: 0,
            level: vec![0.005; 48_000],
        };
        let env = duck(0.1, 0.1, 0.1).envelope(&[&key], 0, 48_000, 48_000);
        assert!(env.gains.iter().all(|g| *g == 1.0));
        assert!(env.regions.is_empty());
    }

    #[test]
    fn a_key_before_the_clip_still_releases_into_it() {
        let mut level = vec![0.0_f32; 48_000];
        level[..24_000].fill(1.0);
        let key = Key { start: 0, level };
        let env = duck(0.0, 0.0, 0.5).envelope(&[&key], 24_000, 24_000, 48_000);
        assert!(
            (env.gains[0] - 0.1).abs() < 1e-4,
            "still down right after the key"
        );
        assert!(
            env.gains[12_000] > 0.1 && env.gains[12_000] < 1.0,
            "releasing"
        );
        assert_eq!(*env.gains.last().unwrap(), 1.0);
    }
}
