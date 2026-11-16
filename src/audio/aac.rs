//! AAC-LC encoding of the mixed timeline into raw MP4 access units (pure Rust, `oxideav-aac`).
//!
//! The encoder emits ADTS frames of 1024 samples per channel, primed with a zero hop, so the
//! first `PRIMING` decoded samples are encoder delay. The MP4 edit list (see `mux`) hides them,
//! which is what keeps audio sample-aligned with the first video frame.
use super::SAMPLE_RATE;
use anyhow::{anyhow, bail, Result};
use oxideav_aac::encoder::{EncoderConfig, StreamEncoder, FRAME_LEN};

/// Samples per channel in one AAC access unit.
pub const UNIT_SAMPLES: usize = FRAME_LEN;
/// Decoder output samples that precede the first real sample (one AAC frame of encoder delay).
pub const PRIMING: u32 = UNIT_SAMPLES as u32;
const CHANNELS: usize = 2;

/// Encoded AAC-LC audio: raw access units without ADTS headers.
pub struct AacStream {
    pub units: Vec<Vec<u8>>,
    /// Real (non-priming) samples per channel the stream was built from.
    pub frames: usize,
    pub bitrate: u32,
}

/// Strips the ADTS header (7 bytes, or 9 with a CRC) from one frame.
fn raw_unit(frame: &[u8]) -> Result<Vec<u8>> {
    if frame.len() < 7 || frame[0] != 0xFF || frame[1] & 0xF0 != 0xF0 {
        bail!("AAC encoder produced a frame without an ADTS header");
    }
    let header = if frame[1] & 1 == 1 { 7 } else { 9 };
    let declared = (usize::from(frame[3] & 3) << 11)
        | (usize::from(frame[4]) << 3)
        | usize::from(frame[5] >> 5);
    if declared != frame.len() || declared <= header {
        bail!("AAC encoder produced an inconsistent frame length");
    }
    Ok(frame[header..].to_vec())
}

/// Encodes interleaved stereo 16-bit PCM at 48 kHz into ADTS frames: one per 1024-sample hop
/// (the last may be partial) plus the flush frame.
fn encode_adts(interleaved: &[i16], bitrate: u32) -> Result<Vec<Vec<u8>>> {
    let mut encoder = StreamEncoder::new(EncoderConfig {
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS as u8,
        bitrate,
    })
    .map_err(|e| anyhow!("AAC encoder: {e:?}"))?;
    let hop = UNIT_SAMPLES * CHANNELS;
    let mut chunks = interleaved.chunks(hop);
    // An empty input still yields one silent frame, as `StreamEncoder::encode_all` does.
    let first = chunks.next().unwrap_or(&[]);
    let mut frames = vec![encoder
        .encode_frame(first)
        .map_err(|e| anyhow!("AAC encoder: {e:?}"))?];
    for chunk in chunks {
        frames.push(
            encoder
                .encode_frame(chunk)
                .map_err(|e| anyhow!("AAC encoder: {e:?}"))?,
        );
    }
    frames.push(
        encoder
            .finish()
            .map_err(|e| anyhow!("AAC encoder: {e:?}"))?,
    );
    Ok(frames)
}

/// Encodes interleaved stereo 16-bit PCM at 48 kHz. `bitrate` is in bits per second.
pub fn encode(interleaved: &[i16], bitrate: u32) -> Result<AacStream> {
    let units = encode_adts(interleaved, bitrate)?
        .iter()
        .map(|frame| raw_unit(frame))
        .collect::<Result<Vec<_>>>()?;
    Ok(AacStream {
        units,
        frames: interleaved.len() / CHANNELS,
        bitrate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_aac::decode::StreamDecoder;

    fn tone(frames: usize, hz: f64) -> Vec<i16> {
        (0..frames)
            .flat_map(|i| {
                let v =
                    (0.5 * (2.0 * std::f64::consts::PI * hz * i as f64 / 48_000.0).sin() * 32767.0)
                        .round() as i16;
                [v, v]
            })
            .collect()
    }

    #[test]
    fn emits_one_unit_per_hop_plus_the_flush_frame() {
        let stream = encode(&tone(5000, 440.0), 192_000).unwrap();
        // 5000 samples = 5 hops (the last one partial), plus the trailing flush frame.
        assert_eq!(stream.units.len(), 5 + 1);
        assert_eq!(stream.frames, 5000);
        assert!(stream.units.iter().all(|unit| !unit.is_empty()));
    }

    #[test]
    fn output_is_deterministic() {
        let pcm = tone(10_000, 1000.0);
        let (a, b) = (
            encode(&pcm, 192_000).unwrap(),
            encode(&pcm, 192_000).unwrap(),
        );
        assert_eq!(a.units, b.units);
    }

    /// Decoding the stream and dropping `PRIMING` samples must line up with the source: the
    /// strongest check that the edit list value is right.
    #[test]
    fn decoded_audio_aligns_with_the_source_after_priming() {
        let source = tone(48_000, 1000.0);
        let adts = encode_adts(&source, 192_000).unwrap().concat();
        let decoded = StreamDecoder::new().decode_all(&adts).unwrap();
        let pcm: Vec<i16> = decoded.into_iter().flat_map(|frame| frame.pcm).collect();
        let skip = PRIMING as usize * CHANNELS;
        let mut best = (f64::MAX, 0_i32);
        for lag in -8..=8_i32 {
            let mut error = 0.0;
            for i in 4000..40_000_usize {
                let a = f64::from(source[i * CHANNELS]);
                let b = f64::from(pcm[(skip as i64 + (i as i64 + i64::from(lag)) * 2) as usize]);
                error += (a - b).powi(2);
            }
            if error < best.0 {
                best = (error, lag);
            }
        }
        assert_eq!(best.1, 0, "decoded audio is offset by {} samples", best.1);
        let rms = (best.0 / 36_000.0).sqrt();
        assert!(rms < 600.0, "rms error {rms}");
    }
}
