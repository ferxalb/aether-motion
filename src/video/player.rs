//! On-demand H.264 decoding with random access. A request for display frame `k` finds its closed
//! GOP, starts a fresh decoder at the GOP's keyframe (or continues when the request is ahead in the
//! same GOP), and decodes forward. Closed GOPs make decoder output order equal display order, so
//! the n-th picture a GOP produces is its n-th frame by presentation time.
use super::annexb::sample_to_annex_b;
use super::colour;
use super::index::{open_reader, Reader, VideoIndex};
use anyhow::{anyhow, bail, Result};
use image::{imageops, RgbaImage};
use openh264::decoder::{DecodedYUV, Decoder, DecoderConfig};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct VideoPlayer {
    path: PathBuf,
    index: Arc<VideoIndex>,
    reader: Reader,
    decoder: Option<Decoder>,
    /// GOP being decoded.
    gop: usize,
    /// Next sample (decode order, 0-based) to feed.
    fed: usize,
    /// Pictures the decoder has produced for this GOP so far.
    emitted: usize,
    /// All samples were fed and the decoder was drained.
    flushed: bool,
    /// Decoded pictures not yet returned, consecutive from GOP offset `stash_first`.
    stash: VecDeque<RgbaImage>,
    stash_first: usize,
    /// Offsets below this in the current GOP can no longer be produced without restarting it.
    floor: usize,
    /// The last frame returned, by display index.
    current: Option<(usize, RgbaImage)>,
    annex_b: Vec<u8>,
}

impl VideoPlayer {
    pub fn open(path: &Path, index: Arc<VideoIndex>) -> Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            reader: open_reader(path)?,
            index,
            decoder: None,
            gop: 0,
            fed: 0,
            emitted: 0,
            flushed: false,
            stash: VecDeque::new(),
            stash_first: 0,
            floor: 0,
            current: None,
            annex_b: Vec::new(),
        })
    }

    /// Decoded RGBA picture of display frame `display`, at the source size.
    pub fn frame(&mut self, display: usize) -> Result<&RgbaImage> {
        if display >= self.index.frame_count() {
            bail!(
                "{}: frame {display} is past the last frame",
                self.path.display()
            );
        }
        if self.current.as_ref().is_some_and(|(d, _)| *d == display) {
            return Ok(&self.current.as_ref().expect("checked above").1);
        }
        let (gop, offset) = self.index.gop_of(display);
        if self.decoder.is_none() || self.gop != gop || offset < self.floor {
            self.start(gop)?;
        }
        loop {
            while self.stash_first < offset && !self.stash.is_empty() {
                self.stash.pop_front();
                self.stash_first += 1;
            }
            if self.stash_first == offset {
                if let Some(image) = self.stash.pop_front() {
                    self.stash_first += 1;
                    self.floor = offset + 1;
                    self.current = Some((display, image));
                    return Ok(&self.current.as_ref().expect("just set").1);
                }
            }
            self.step(offset)?;
        }
    }

    /// Starts a fresh decoder at the keyframe of `gop`.
    fn start(&mut self, gop: usize) -> Result<()> {
        let mut decoder =
            Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new())
                .map_err(|e| anyhow!("could not start the H.264 decoder: {e}"))?;
        decoder
            .decode(&self.index.header)
            .map_err(|e| self.decode_error(&e))?;
        self.decoder = Some(decoder);
        self.gop = gop;
        self.fed = self.index.gops[gop].first_sample;
        self.emitted = 0;
        self.flushed = false;
        self.stash.clear();
        self.stash_first = 0;
        self.floor = 0;
        Ok(())
    }

    /// Feeds one sample, or drains the decoder after the last one. Pictures at offsets before
    /// `want` are dropped without color conversion.
    fn step(&mut self, want: usize) -> Result<()> {
        let index = Arc::clone(&self.index);
        let gop = index.gops[self.gop];
        let decoder = self.decoder.as_mut().expect("started before stepping");
        if self.fed < gop.end_sample {
            let sample = self
                .reader
                .read_sample(index.track_id, (self.fed + 1) as u32)
                .map_err(|e| anyhow!("{}: could not read a video sample: {e}", self.path.display()))?
                .ok_or_else(|| anyhow!("{}: a video sample is missing", self.path.display()))?;
            self.fed += 1;
            self.annex_b.clear();
            sample_to_annex_b(&sample.bytes, index.nal_length, &mut self.annex_b)
                .map_err(|e| anyhow!("{}: {e}", self.path.display()))?;
            let output = decoder.decode(&self.annex_b).map_err(|e| {
                anyhow!(
                    "{}: H.264 decoding failed: {e}; the decoder handles 8-bit 4:2:0 Baseline, Main, and High profiles",
                    self.path.display()
                )
            })?;
            if let Some(yuv) = output {
                let at = self.emitted;
                self.emitted += 1;
                if at >= want {
                    keep(&mut self.stash, &mut self.stash_first, at, &yuv, &index, &self.path)?;
                }
            }
        } else if !self.flushed {
            self.flushed = true;
            let rest = decoder.flush_remaining().map_err(|e| {
                anyhow!("{}: H.264 decoding failed: {e}", self.path.display())
            })?;
            for yuv in rest {
                let at = self.emitted;
                self.emitted += 1;
                if at >= want {
                    keep(&mut self.stash, &mut self.stash_first, at, &yuv, &index, &self.path)?;
                }
            }
        } else {
            bail!(
                "{}: the decoder produced fewer pictures than the container lists for this group",
                self.path.display()
            );
        }
        Ok(())
    }

    fn decode_error(&self, error: &dyn std::fmt::Display) -> anyhow::Error {
        anyhow!("{}: H.264 decoding failed: {error}", self.path.display())
    }
}

/// Converts a decoded picture and queues it as the frame at GOP offset `at`.
fn keep(
    stash: &mut VecDeque<RgbaImage>,
    stash_first: &mut usize,
    at: usize,
    yuv: &DecodedYUV<'_>,
    index: &VideoIndex,
    path: &Path,
) -> Result<()> {
    let (w, h) = yuv.dimensions();
    if (w as u32, h as u32) != index.coded {
        bail!(
            "{}: decoded pictures are {w}x{h} but the container says {}x{}",
            path.display(),
            index.coded.0,
            index.coded.1
        );
    }
    let mut rgba = vec![0u8; w * h * 4];
    if index.colour.is_decoder_default() {
        yuv.write_rgba8(&mut rgba);
        // Decoded video is opaque whatever the converter wrote into the alpha byte.
        rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
    } else {
        colour::to_rgba(
            (yuv.y(), yuv.u(), yuv.v()),
            yuv.strides(),
            (w, h),
            index.colour,
            &mut rgba,
        );
    }
    let image = RgbaImage::from_raw(w as u32, h as u32, rgba)
        .ok_or_else(|| anyhow!("{}: invalid decoded picture size", path.display()))?;
    let image = match index.rotation {
        90 => imageops::rotate90(&image),
        180 => imageops::rotate180(&image),
        270 => imageops::rotate270(&image),
        _ => image,
    };
    if stash.is_empty() {
        *stash_first = at;
    }
    stash.push_back(image);
    Ok(())
}
