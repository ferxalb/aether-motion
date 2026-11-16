//! MP4 side of audio: the AAC track configuration, interleaving of access units with video
//! frames, and the post-write edit-list patch that aligns audio with the first video frame.
//!
//! The `mp4` crate writes tracks but no edit list, and its track durations count the encoder's
//! priming samples. `patch_moov` rewrites the `moov` box (always the last box the crate writes) so
//! the audio track starts `PRIMING` samples into the media and both durations equal the video's.
use super::aac::{AacStream, PRIMING, UNIT_SAMPLES};
use super::SAMPLE_RATE;
use anyhow::{anyhow, ensure, Context, Result};
use mp4::{
    AacConfig, AudioObjectType, Bytes, ChannelConfig, MediaConfig, Mp4Sample, SampleFreqIndex,
    TrackConfig,
};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// MP4 audio track configuration: AAC-LC, 48 kHz, stereo, timescale of one tick per sample.
pub fn track_config(stream: &AacStream) -> TrackConfig {
    let mut config = TrackConfig::from(MediaConfig::AacConfig(AacConfig {
        bitrate: stream.bitrate,
        profile: AudioObjectType::AacLowComplexity,
        freq_index: SampleFreqIndex::Freq48000,
        chan_conf: ChannelConfig::Stereo,
    }));
    config.timescale = SAMPLE_RATE;
    config
}

/// The MP4 sample for access unit `index`.
pub fn sample(stream: &AacStream, index: usize) -> Mp4Sample {
    Mp4Sample {
        start_time: (index * UNIT_SAMPLES) as u64,
        duration: UNIT_SAMPLES as u32,
        rendering_offset: 0,
        is_sync: true,
        bytes: Bytes::copy_from_slice(&stream.units[index]),
    }
}

/// Number of access units whose decode time falls before the end of video frame `frame`,
/// so writing that many after each frame keeps both tracks interleaved in time.
pub fn units_due(frame: usize, fps: u32) -> usize {
    let end = (frame as u128 + 1) * u128::from(SAMPLE_RATE);
    end.div_ceil(UNIT_SAMPLES as u128 * u128::from(fps)) as usize
}

/// Size of the `edts` box with one `elst` entry.
const EDTS_LEN: usize = 36;

fn be32(buffer: &[u8], at: usize) -> usize {
    u32::from_be_bytes(buffer[at..at + 4].try_into().unwrap()) as usize
}

fn put32(buffer: &mut [u8], at: usize, value: usize) -> Result<()> {
    let value = u32::try_from(value).context("MP4 box is too large")?;
    buffer[at..at + 4].copy_from_slice(&value.to_be_bytes());
    Ok(())
}

/// Offset and size of the first child box named `name` inside `buffer[start..end]`.
fn child(buffer: &[u8], start: usize, end: usize, name: &[u8; 4]) -> Option<(usize, usize)> {
    let mut at = start;
    while at + 8 <= end {
        let size = be32(buffer, at);
        if size < 8 || at + size > end {
            return None;
        }
        if &buffer[at + 4..at + 8] == name {
            return Some((at, size));
        }
        at += size;
    }
    None
}

fn children<'a>(
    buffer: &'a [u8],
    start: usize,
    end: usize,
    name: &'a [u8; 4],
) -> impl Iterator<Item = (usize, usize)> + 'a {
    let mut at = start;
    std::iter::from_fn(move || {
        while at + 8 <= end {
            let size = be32(buffer, at);
            if size < 8 || at + size > end {
                return None;
            }
            let found = &buffer[at + 4..at + 8] == name;
            let offset = at;
            at += size;
            if found {
                return Some((offset, size));
            }
        }
        None
    })
}

/// Writes `value` into the duration field of an `mvhd` or `tkhd` box at `offset`.
/// `fields` gives the offset of the duration for version 0 and version 1 boxes.
fn set_duration(
    buffer: &mut [u8],
    offset: usize,
    fields: (usize, usize),
    value: u64,
) -> Result<()> {
    if buffer[offset + 8] == 0 {
        let value = u32::try_from(value).context("duration does not fit a version 0 MP4 box")?;
        buffer[offset + fields.0..offset + fields.0 + 4].copy_from_slice(&value.to_be_bytes());
    } else {
        buffer[offset + fields.1..offset + fields.1 + 8].copy_from_slice(&value.to_be_bytes());
    }
    Ok(())
}

fn edit_box(video_frames: u64) -> Result<[u8; EDTS_LEN]> {
    let segment = u32::try_from(video_frames).context("video is too long for an edit list")?;
    let mut edts = [0_u8; EDTS_LEN];
    edts[0..4].copy_from_slice(&(EDTS_LEN as u32).to_be_bytes());
    edts[4..8].copy_from_slice(b"edts");
    edts[8..12].copy_from_slice(&((EDTS_LEN - 8) as u32).to_be_bytes());
    edts[12..16].copy_from_slice(b"elst");
    // Version 0, flags 0, one entry.
    edts[20..24].copy_from_slice(&1_u32.to_be_bytes());
    edts[24..28].copy_from_slice(&segment.to_be_bytes());
    edts[28..32].copy_from_slice(&PRIMING.to_be_bytes());
    // Media rate 1.0 (integer 1, fraction 0).
    edts[32..34].copy_from_slice(&1_u16.to_be_bytes());
    Ok(edts)
}

/// Rewrites the `moov` box of a finished MP4 (video track 1, audio track 2). `video_frames` is
/// the video length in movie-timescale ticks (the movie timescale is the frame rate).
pub fn patch_moov(path: &Path, video_frames: u64) -> Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let length = file.metadata()?.len();
    let (mut position, mut moov) = (0_u64, None);
    while position + 8 <= length {
        let mut header = [0_u8; 8];
        file.seek(SeekFrom::Start(position))?;
        file.read_exact(&mut header)?;
        let mut size = u64::from(u32::from_be_bytes(header[0..4].try_into().unwrap()));
        if size == 1 {
            let mut large = [0_u8; 8];
            file.read_exact(&mut large)?;
            size = u64::from_be_bytes(large);
        } else if size == 0 {
            size = length - position;
        }
        ensure!(size >= 8 && position + size <= length, "corrupt MP4 box");
        if &header[4..8] == b"moov" {
            moov = Some((position, size));
        }
        position += size;
    }
    let (moov_at, moov_len) = moov.ok_or_else(|| anyhow!("MP4 has no moov box"))?;
    ensure!(
        moov_at + moov_len == length,
        "moov is not the last MP4 box; cannot patch the audio edit list"
    );
    let mut buffer = vec![0_u8; moov_len as usize];
    file.seek(SeekFrom::Start(moov_at))?;
    file.read_exact(&mut buffer)?;

    let end = buffer.len();
    let (mvhd, _) = child(&buffer, 8, end, b"mvhd").ok_or_else(|| anyhow!("moov has no mvhd"))?;
    set_duration(&mut buffer, mvhd, (24, 32), video_frames)?;
    let traks: Vec<_> = children(&buffer, 8, end, b"trak").collect();
    ensure!(traks.len() == 2, "expected a video and an audio track");
    let (trak, trak_len) = traks[1];
    let trak_end = trak + trak_len;
    let (mdia, mdia_len) = child(&buffer, trak + 8, trak_end, b"mdia")
        .ok_or_else(|| anyhow!("audio track has no mdia"))?;
    let (hdlr, _) = child(&buffer, mdia + 8, mdia + mdia_len, b"hdlr")
        .ok_or_else(|| anyhow!("audio track has no hdlr"))?;
    ensure!(
        &buffer[hdlr + 16..hdlr + 20] == b"soun",
        "second track is not audio"
    );
    let (tkhd, tkhd_len) = child(&buffer, trak + 8, trak_end, b"tkhd")
        .ok_or_else(|| anyhow!("audio track has no tkhd"))?;
    ensure!(
        child(&buffer, trak + 8, trak_end, b"edts").is_none(),
        "audio track already has an edit list"
    );
    set_duration(&mut buffer, tkhd, (28, 36), video_frames)?;
    let insert_at = tkhd + tkhd_len;
    buffer.splice(insert_at..insert_at, edit_box(video_frames)?);
    put32(&mut buffer, trak, trak_len + EDTS_LEN)?;
    put32(&mut buffer, 0, moov_len as usize + EDTS_LEN)?;

    file.set_len(moov_at)?;
    file.seek(SeekFrom::Start(moov_at))?;
    file.write_all(&buffer)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_due_interleaves_by_time() {
        // At 30 fps one frame spans 1600 samples = 1.5625 units.
        assert_eq!(units_due(0, 30), 2);
        assert_eq!(units_due(1, 30), 4);
        // Every unit starting before the end of the video is written by the last frame.
        let frames = 90;
        assert!(units_due(frames - 1, 30) * UNIT_SAMPLES >= frames * 1600);
    }

    #[test]
    fn edit_list_box_layout() {
        let edts = edit_box(120).unwrap();
        assert_eq!(&edts[4..8], b"edts");
        assert_eq!(&edts[12..16], b"elst");
        assert_eq!(u32::from_be_bytes(edts[20..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_be_bytes(edts[24..28].try_into().unwrap()), 120);
        assert_eq!(u32::from_be_bytes(edts[28..32].try_into().unwrap()), 1024);
        assert_eq!(u16::from_be_bytes(edts[32..34].try_into().unwrap()), 1);
    }
}
