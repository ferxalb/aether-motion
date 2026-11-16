//! Deterministic sample-rate conversion by polyphase Kaiser-windowed sinc interpolation.
//!
//! Output sample `n` sits at input position `n * from / to`. The integer part and the phase are
//! computed with integer arithmetic, so the filter phase never drifts, and a rational ratio needs
//! only `to / gcd(from, to)` distinct filters (160 for 44.1 kHz to 48 kHz).

/// Zero crossings of the sinc kernel on each side at unity ratio (64 taps), with a Kaiser beta of
/// 9, which puts the stopband near 90 dB below the passband.
const HALF_WIDTH: usize = 32;
const BETA: f64 = 9.0;
/// Cutoff as a fraction of the lower Nyquist frequency; keeps the transition band below it.
const CUTOFF: f64 = 0.95;

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Modified Bessel function of the first kind, order 0 (power series).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let quarter = x * x / 4.0;
    for k in 1..60 {
        term *= quarter / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// Converts `input` from `from` Hz to `to` Hz. Samples outside the input count as silence.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let (from64, to64) = (u64::from(from), u64::from(to));
    let g = gcd(from64, to64);
    let phases = (to64 / g) as usize;
    // Filtering at the lower of the two rates: widen the kernel when decimating.
    let stretch = (from as f64 / to as f64).max(1.0);
    let half = (HALF_WIDTH as f64 * stretch).ceil() as usize;
    let taps = 2 * half;
    let cutoff = CUTOFF / stretch;
    let denominator = bessel_i0(BETA);
    // table[phase][tap] weighs input[floor(pos) - half + 1 + tap].
    let mut table = vec![0.0_f64; phases * taps];
    for (phase, row) in table.chunks_exact_mut(taps).enumerate() {
        let fraction = phase as f64 / phases as f64;
        for (tap, weight) in row.iter_mut().enumerate() {
            let distance = (tap as f64 - (half as f64 - 1.0)) - fraction;
            let ratio = distance / half as f64;
            let window = if ratio.abs() >= 1.0 {
                0.0
            } else {
                bessel_i0(BETA * (1.0 - ratio * ratio).sqrt()) / denominator
            };
            *weight = cutoff * sinc(cutoff * distance) * window;
        }
        // Normalize each phase to unity DC gain so levels do not depend on the phase.
        let sum: f64 = row.iter().sum();
        row.iter_mut().for_each(|w| *w /= sum);
    }
    let frames = ((input.len() as u64 * to64 + from64 / 2) / from64) as usize;
    let mut output = Vec::with_capacity(frames);
    for n in 0..frames as u64 {
        let position = n * from64;
        let base = (position / to64) as i64;
        let phase = ((position % to64) / g) as usize;
        let row = &table[phase * taps..(phase + 1) * taps];
        let first = base - half as i64 + 1;
        let mut acc = 0.0_f64;
        if first >= 0 && (first as usize + taps) <= input.len() {
            for (weight, sample) in row.iter().zip(&input[first as usize..]) {
                acc += weight * f64::from(*sample);
            }
        } else {
            for (tap, weight) in row.iter().enumerate() {
                let index = first + tap as i64;
                if (0..input.len() as i64).contains(&index) {
                    acc += weight * f64::from(input[index as usize]);
                }
            }
        }
        output.push(acc as f32);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hz: f64, seconds: f64) -> Vec<f32> {
        (0..(rate as f64 * seconds) as usize)
            .map(|i| {
                (0.5 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()) as f32
            })
            .collect()
    }

    /// RMS error against the ideal tone, ignoring the filter's edge transients.
    fn error_db(output: &[f32], rate: u32, hz: f64) -> f64 {
        let edge = 2000;
        let mut sum = 0.0;
        for (i, sample) in output
            .iter()
            .enumerate()
            .take(output.len() - edge)
            .skip(edge)
        {
            let ideal = 0.5 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin();
            sum += (f64::from(*sample) - ideal).powi(2);
        }
        10.0 * (sum / (output.len() - 2 * edge) as f64).log10()
    }

    #[test]
    fn same_rate_is_a_copy() {
        let input = tone(48_000, 440.0, 0.1);
        assert_eq!(resample(&input, 48_000, 48_000), input);
    }

    #[test]
    fn converts_common_rates_below_minus_80_db_error() {
        for (from, hz) in [(44_100, 1_000.0), (96_000, 5_000.0), (22_050, 3_000.0)] {
            let output = resample(&tone(from, hz, 1.0), from, 48_000);
            assert_eq!(output.len(), 48_000, "length for {from} Hz");
            let error = error_db(&output, 48_000, hz);
            assert!(error < -80.0, "{from} Hz -> 48 kHz error {error:.1} dB");
        }
    }

    #[test]
    fn output_length_follows_the_ratio() {
        assert_eq!(resample(&vec![0.0; 44_100], 44_100, 48_000).len(), 48_000);
        assert_eq!(resample(&vec![0.0; 1000], 8_000, 48_000).len(), 6_000);
    }

    #[test]
    fn attenuates_content_above_the_target_nyquist() {
        // 30 kHz at 96 kHz is above 24 kHz and must not alias into the 48 kHz output.
        let output = resample(&tone(96_000, 30_000.0, 0.5), 96_000, 48_000);
        let peak = output[3000..21_000]
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak < 0.5 * 10f32.powf(-80.0 / 20.0) * 10.0, "peak {peak}");
    }
}
