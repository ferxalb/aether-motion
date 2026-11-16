//! Audio file decoding: WAV, FLAC, Ogg Vorbis, and MP3 into planar `f32` PCM at the file's own
//! sample rate. Encoder delay and padding are trimmed by the container readers, so the samples
//! start where playback starts.
use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Decoded audio: one or two planar channels at `rate` Hz.
pub struct Pcm {
    pub rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

/// Upper bound on one decoded source (frames x channels), about 512 MiB of `f32`.
const MAX_SAMPLES: usize = 128 * 1024 * 1024;

pub fn decode_file(path: &Path) -> Result<Pcm> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("could not read audio file {}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| {
            anyhow!(
                "unsupported or unreadable audio file {}: {e}; use WAV, FLAC, Ogg Vorbis, or MP3",
                path.display()
            )
        })?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| anyhow!("{} has no audio track", path.display()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("{} has no decodable audio track", path.display()))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| anyhow!("unsupported audio codec in {}: {e}", path.display()))?;

    let mut pcm = Pcm {
        rate: 0,
        channels: Vec::new(),
    };
    let mut planes: Vec<Vec<f32>> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(e) => bail!("could not read {}: {e}", path.display()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(e)) => {
                bail!("corrupt audio data in {}: {e}", path.display())
            }
            Err(e) => bail!("could not decode {}: {e}", path.display()),
        };
        let (rate, count) = (decoded.spec().rate(), decoded.spec().channels().count());
        if pcm.channels.is_empty() {
            if !(1..=2).contains(&count) {
                bail!(
                    "{} has {count} channels; only mono and stereo are supported",
                    path.display()
                );
            }
            pcm.rate = rate;
            pcm.channels = vec![Vec::new(); count];
        } else if rate != pcm.rate || count != pcm.channels.len() {
            bail!("{} changes sample rate or channels midway", path.display());
        }
        decoded.copy_to_vecs_planar::<f32>(&mut planes);
        for (out, plane) in pcm.channels.iter_mut().zip(&planes) {
            out.extend_from_slice(plane);
        }
        if pcm.frames() * pcm.channels.len() > MAX_SAMPLES {
            bail!(
                "{} is too long to decode (limit {} million samples)",
                path.display(),
                MAX_SAMPLES / 1_000_000
            );
        }
    }
    if pcm.frames() == 0 {
        bail!("{} contains no audio samples", path.display());
    }
    Ok(pcm)
}
