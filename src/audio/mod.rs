//! Audio for compositions: `<audio>` clips are decoded (`decode`), converted to 48 kHz
//! (`resample`), read at variable speed when a clip sets one (`varispeed`), shaped by per-clip
//! effects (volume automation, equalizer, ducking: `effects`), mixed sample-accurately onto one stereo timeline (`mix`), encoded to AAC-LC
//! (`aac`) after optional master limiting and dither (`master`), and attached to the MP4 with the edit list that trims encoder delay (`mux`).
//!
//! `build_track` is the facade used by the encoder and by `check`.

pub mod aac;
pub mod decode;
pub mod effects;
pub mod master;
pub mod mix;
pub mod mux;
pub mod resample;
mod varispeed;

use crate::model::Composition;
use anyhow::Result;

/// Sample rate of the mix and of the MP4 audio track.
pub const SAMPLE_RATE: u32 = 48_000;
/// Default AAC bitrate in bits per second (stereo).
pub const DEFAULT_BITRATE: u32 = 192_000;

/// Samples per channel that span `frame_count` video frames at `fps`, rounded to nearest.
pub fn frames_for_video(frame_count: usize, fps: u32) -> usize {
    ((frame_count as u128 * u128::from(SAMPLE_RATE) + u128::from(fps) / 2) / u128::from(fps))
        as usize
}

/// Mixes and encodes the composition's audio for a video of `frame_count` frames at `fps`.
/// Returns `None` when the composition has no `<audio>` clips.
pub fn build_track(
    composition: &Composition,
    frame_count: usize,
    fps: u32,
    bitrate: u32,
) -> Result<Option<aac::AacStream>> {
    if composition.audio.is_empty() {
        return Ok(None);
    }
    let mix = mix::mix(composition, frames_for_video(frame_count, fps))?;
    aac::encode(
        &mix.to_i16_interleaved(composition.audio_master.dither),
        bitrate,
    )
    .map(Some)
}
