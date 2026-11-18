//! Distributed rendering: a render split by segment ranges into part files, and their assembly.
//!
//! [`render_part`] encodes segments `a..b` of a render (see `segments.rs`) into an `.aseg` part
//! file; separate processes or machines can each render one range. [`assemble_parts`] checks that a
//! set of parts covers every segment of one render exactly once with the same encoder output, mixes
//! the composition's audio once, and muxes everything in order. Segments are encoded independently
//! and the muxer sees the samples in the same order, so the MP4 equals a single-process render whose
//! workers took the same ranges (byte-identical unless gradient tiles differ; see "Encoding" in
//! `docs/architecture.md`).
//!
//! Part file layout (integers little-endian): the magic `AETHSEG1`; a `u32` length and a JSON
//! header (`version`, `width`, `height`, `fps`, `qp`, `frames`, `segments`, `first`, `end`); the
//! `u32`-prefixed SPS and PPS; then, for each frame of the range, a `u32` length, a sync flag byte,
//! and the MP4 sample (length-prefixed NAL units).

use super::progress::{ProgressObserver, Tracker};
use super::segments::{encode_segments, segment_count, segment_frames};
use super::{check_audio_bitrate, check_video, partial_path, start_mp4, EncodeOptions};
use crate::audio;
use crate::model::Composition;
use crate::render::Renderer;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// File extension of part files.
pub const PART_EXTENSION: &str = "aseg";
const MAGIC: &[u8; 8] = b"AETHSEG1";
const VERSION: u64 = 1;
/// Upper bound for one header, parameter set, or sample read from a part file.
const MAX_CHUNK: usize = 256 * 1024 * 1024;

/// What a part covers and how it was encoded; parts of one render share all but `first`/`end`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PartHeader {
    width: u32,
    height: u32,
    fps: u32,
    qp: u8,
    /// Frames of the whole render.
    frames: usize,
    /// Segments of the whole render.
    segments: usize,
    /// First segment in this part.
    first: usize,
    /// One past the last segment in this part.
    end: usize,
}

impl PartHeader {
    fn to_json(&self) -> Value {
        json!({
            "version": VERSION, "width": self.width, "height": self.height, "fps": self.fps,
            "qp": self.qp, "frames": self.frames, "segments": self.segments,
            "first": self.first, "end": self.end,
        })
    }

    fn from_json(value: &Value) -> Result<Self> {
        let field = |name: &str| {
            value[name]
                .as_u64()
                .ok_or_else(|| anyhow!("part header has no valid `{name}`"))
        };
        if field("version")? != VERSION {
            bail!("unsupported part version {}", value["version"]);
        }
        let header = Self {
            width: u32::try_from(field("width")?)?,
            height: u32::try_from(field("height")?)?,
            fps: u32::try_from(field("fps")?)?,
            qp: u8::try_from(field("qp")?)?,
            frames: usize::try_from(field("frames")?)?,
            segments: usize::try_from(field("segments")?)?,
            first: usize::try_from(field("first")?)?,
            end: usize::try_from(field("end")?)?,
        };
        if header.fps == 0
            || segment_count(header.frames, header.fps) != Some(header.segments)
            || header.first >= header.end
            || header.end > header.segments
        {
            bail!("inconsistent part header");
        }
        Ok(header)
    }

    /// Frame range of this part within the render.
    fn frame_range(&self) -> Range<usize> {
        let len = segment_frames(self.fps);
        self.first * len..(self.end * len).min(self.frames)
    }

    /// True when `other` belongs to the same render (everything but the segment range matches).
    fn same_render(&self, other: &Self) -> bool {
        (
            self.width,
            self.height,
            self.fps,
            self.qp,
            self.frames,
            self.segments,
        ) == (
            other.width,
            other.height,
            other.fps,
            other.qp,
            other.frames,
            other.segments,
        )
    }
}

fn write_chunk(out: &mut impl Write, bytes: &[u8]) -> Result<()> {
    let len = u32::try_from(bytes.len()).context("part chunk is too large")?;
    out.write_all(&len.to_le_bytes())?;
    out.write_all(bytes)?;
    Ok(())
}

fn read_chunk(input: &mut impl Read) -> Result<Vec<u8>> {
    let mut len = [0; 4];
    input.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_CHUNK {
        bail!("part chunk of {len} bytes is too large");
    }
    let mut bytes = vec![0; len];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Renders and encodes segments `segments` of the render into the part file `output`.
///
/// Use [`segment_count`] with [`Renderer::frame_count`] to plan the ranges; every range of one
/// render must use the same composition, variables, output size, fps, and quality. Fails for a
/// render that is not longer than one segment (render it in one piece instead).
pub fn render_part(
    renderer: &mut Renderer<'_>,
    output: &Path,
    options: &EncodeOptions,
    segments: Range<usize>,
) -> Result<()> {
    render_part_with_progress(renderer, output, options, segments, None)
}

/// [`render_part`] that reports the frames of the part to `observer` while it encodes.
pub fn render_part_with_progress(
    renderer: &mut Renderer<'_>,
    output: &Path,
    options: &EncodeOptions,
    segments: Range<usize>,
    observer: Option<&dyn ProgressObserver>,
) -> Result<()> {
    let EncodeOptions {
        fps,
        threads,
        quality,
        ..
    } = *options;
    check_video(renderer, fps, threads)?;
    let frames = renderer.frame_count(fps);
    let total = segment_count(frames, fps).ok_or_else(|| {
        anyhow!("the render is one segment long or shorter; render it in one piece")
    })?;
    if segments.is_empty() || segments.end > total {
        bail!(
            "--segments {}..{} is outside the render (0..{total})",
            segments.start,
            segments.end
        );
    }
    let (width, height) = renderer.dimensions();
    let header = PartHeader {
        width,
        height,
        fps,
        qp: quality.base_qp(),
        frames,
        segments: total,
        first: segments.start,
        end: segments.end,
    };
    let partial = partial_path(output)?;
    let len = segment_frames(fps);
    let part_frames = (segments.end * len).min(frames) - segments.start * len;
    let tracker = Tracker::new(observer, part_frames);
    let result = (|| -> Result<()> {
        let mut out = BufWriter::new(File::create(&partial).context("could not create part")?);
        let mut written = 0;
        let mut started = false;
        let (rasterized, reused) = encode_segments(
            renderer,
            segments,
            (fps, quality),
            threads as usize,
            frames,
            &tracker,
            |segment| {
                if !started {
                    out.write_all(MAGIC)?;
                    write_chunk(&mut out, header.to_json().to_string().as_bytes())?;
                    write_chunk(&mut out, &segment.sps)?;
                    write_chunk(&mut out, &segment.pps)?;
                    started = true;
                }
                for (sample, sync) in segment.samples {
                    write_chunk(&mut out, &sample)?;
                    out.write_all(&[u8::from(sync)])?;
                    written += 1;
                }
                Ok(())
            },
        )?;
        if written != header.frame_range().len() {
            bail!(
                "produced {written} of {} frames",
                header.frame_range().len()
            );
        }
        out.into_inner()
            .map_err(|e| e.into_error())?
            .sync_all()
            .context("could not write part")?;
        renderer.add_stats(rasterized, reused);
        fs::rename(&partial, output).context("could not save part")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

/// An open part file positioned at its first sample.
struct PartReader {
    path: PathBuf,
    header: PartHeader,
    sps: Vec<u8>,
    pps: Vec<u8>,
    input: BufReader<File>,
}

impl PartReader {
    fn open(path: &Path) -> Result<Self> {
        let read = || -> Result<Self> {
            let mut input = BufReader::new(File::open(path)?);
            let mut magic = [0; 8];
            input.read_exact(&mut magic)?;
            if &magic != MAGIC {
                bail!("not an AETHER Motion part file");
            }
            let header: Value = serde_json::from_slice(&read_chunk(&mut input)?)?;
            Ok(Self {
                path: path.to_path_buf(),
                header: PartHeader::from_json(&header)?,
                sps: read_chunk(&mut input)?,
                pps: read_chunk(&mut input)?,
                input,
            })
        };
        read().with_context(|| format!("could not read part {}", path.display()))
    }

    fn sample(&mut self) -> Result<(Vec<u8>, bool)> {
        let read = |input: &mut BufReader<File>| -> Result<(Vec<u8>, bool)> {
            let bytes = read_chunk(input)?;
            let mut sync = [0];
            input.read_exact(&mut sync)?;
            Ok((bytes, sync[0] != 0))
        };
        read(&mut self.input).with_context(|| format!("part {} is truncated", self.path.display()))
    }

    fn expect_end(&mut self) -> Result<()> {
        let mut byte = [0];
        if self.input.read(&mut byte)? != 0 {
            bail!("part {} has trailing data", self.path.display());
        }
        Ok(())
    }
}

/// Assembles part files (in any order) into the MP4 `output`, with the AAC track of
/// `composition` at `audio_bitrate` bits per second. The parts must cover every segment of one
/// render exactly once; `composition` must be parsed with the variables the parts used.
pub fn assemble_parts(
    composition: &Composition,
    parts: &[PathBuf],
    output: &Path,
    audio_bitrate: u32,
) -> Result<()> {
    check_audio_bitrate(audio_bitrate)?;
    let mut readers = parts
        .iter()
        .map(|path| PartReader::open(path))
        .collect::<Result<Vec<_>>>()?;
    readers.sort_by_key(|r| r.header.first);
    let first = readers
        .first()
        .ok_or_else(|| anyhow!("no parts to assemble"))?;
    let (header, sps, pps) = (first.header.clone(), first.sps.clone(), first.pps.clone());
    let mut covered = 0;
    for reader in &readers {
        if !reader.header.same_render(&header) || reader.sps != sps || reader.pps != pps {
            bail!(
                "part {} belongs to a different render",
                reader.path.display()
            );
        }
        if reader.header.first != covered {
            bail!(
                "parts do not cover segment {covered} exactly once (next part starts at {})",
                reader.header.first
            );
        }
        covered = reader.header.end;
    }
    if covered != header.segments {
        bail!(
            "parts cover segments 0..{covered} of 0..{}",
            header.segments
        );
    }
    let expected = (composition.duration * header.fps as f64).ceil() as usize;
    if expected != header.frames {
        bail!(
            "the parts have {} frames but the composition has {expected} at {} fps",
            header.frames,
            header.fps
        );
    }
    let partial = partial_path(output)?;
    let audio = audio::build_track(composition, header.frames, header.fps, audio_bitrate)?;
    let result = (|| -> Result<()> {
        let mut muxer = start_mp4(
            &partial,
            (header.width, header.height),
            header.fps,
            sps,
            pps,
            audio.as_ref(),
        )?;
        for reader in &mut readers {
            for frame in reader.header.frame_range() {
                muxer.video(frame, reader.sample()?)?;
            }
            reader.expect_end()?;
        }
        muxer.finish(&partial, header.frames)?;
        fs::rename(&partial, output).context("could not save MP4")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(first: usize, end: usize) -> PartHeader {
        PartHeader {
            width: 64,
            height: 36,
            fps: 10,
            qp: 26,
            frames: 35,
            segments: 4,
            first,
            end,
        }
    }

    #[test]
    fn headers_round_trip_and_reject_inconsistent_ranges() {
        let part = header(1, 4);
        assert_eq!(PartHeader::from_json(&part.to_json()).unwrap(), part);
        assert_eq!(part.frame_range(), 10..35);
        assert!(PartHeader::from_json(&header(2, 2).to_json()).is_err());
        assert!(PartHeader::from_json(&header(0, 5).to_json()).is_err());
        let mut wrong = header(0, 1).to_json();
        wrong["segments"] = json!(3);
        assert!(PartHeader::from_json(&wrong).is_err());
    }
}
