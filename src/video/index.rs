//! Video container index: display-order frame times and closed GOPs, read from the MP4 sample
//! tables without decoding anything.
use super::annexb::push_nal;
use anyhow::{anyhow, bail, Context, Result};
use mp4::{Mp4Reader, TrackType};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Most frames one source may list.
pub const MAX_FRAMES: usize = 200_000;
/// Largest accepted picture (width x height), as for images.
pub const MAX_PIXELS: u64 = 16_000_000;

/// A closed group of pictures: decode samples `first_sample..end_sample` (decode order, 0-based) to
/// get the display frames starting at `first_display`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gop {
    pub first_sample: usize,
    pub end_sample: usize,
    pub first_display: usize,
}

#[derive(Debug)]
pub struct VideoIndex {
    /// Size of the upright picture (after `rotation`).
    pub width: u32,
    pub height: u32,
    /// Clockwise turn (0, 90, 180, or 270 degrees) that the container asks players to apply.
    pub rotation: u16,
    /// Colour matrix and range (from the SPS, or inferred from the picture size).
    pub colour: super::colour::Colour,
    /// Size of the pictures as coded.
    pub(super) coded: (u32, u32),
    pub(super) track_id: u32,
    pub(super) nal_length: usize,
    /// SPS and PPS in Annex B form, fed to every fresh decoder.
    pub(super) header: Vec<u8>,
    /// Presentation time of each frame in display order, in seconds from the first frame.
    pub frame_times: Vec<f64>,
    /// Length of the video: the last frame time plus its frame duration.
    pub duration: f64,
    pub gops: Vec<Gop>,
    /// The file also carries an audio track (never mixed; see the module docs).
    pub has_audio: bool,
}

pub(super) type Reader = Mp4Reader<BufReader<File>>;

pub(super) fn open_reader(path: &Path) -> Result<Reader> {
    let file = File::open(path)
        .with_context(|| format!("could not read video file {}", path.display()))?;
    let size = file.metadata()?.len();
    Mp4Reader::read_header(BufReader::new(file), size).map_err(|e| {
        anyhow!(
            "unsupported or unreadable video file {}: {e}; use an H.264 .mp4 file",
            path.display()
        )
    })
}

impl VideoIndex {
    pub fn load(path: &Path) -> Result<Self> {
        let name = path.display();
        let reader = open_reader(path)?;
        if reader.is_fragmented() {
            bail!("{name} is a fragmented MP4, which is not supported; remux it with `ffmpeg -i in.mp4 -c copy out.mp4`");
        }
        let has_audio = reader
            .tracks()
            .values()
            .any(|t| matches!(t.track_type(), Ok(TrackType::Audio)));
        let (track_id, track) = reader
            .tracks()
            .iter()
            .filter(|(_, t)| matches!(t.track_type(), Ok(TrackType::Video)))
            .min_by_key(|(id, _)| **id)
            .map(|(id, t)| (*id, t))
            .ok_or_else(|| anyhow!("{name} has no video track"))?;
        let stbl = &track.trak.mdia.minf.stbl;
        let avc1 = stbl.stsd.avc1.as_ref().ok_or_else(|| {
            anyhow!("{name}: only H.264 (AVC) video is supported; re-encode it with `ffmpeg -i in -c:v libx264 -an out.mp4`")
        })?;
        let m = &track.trak.tkhd.matrix;
        // QuickTime display matrix: x' = a*x + c*y, y' = b*x + d*y (16.16 fixed point).
        let rotation = match (m.a.signum(), m.b.signum(), m.c.signum(), m.d.signum()) {
            (1, 0, 0, 1) => 0,
            (0, 1, -1, 0) => 90,
            (-1, 0, 0, -1) => 180,
            (0, -1, 1, 0) => 270,
            _ => bail!("{name} carries mirroring or skew metadata, which is not supported; re-encode it so the picture is stored upright"),
        };
        let (coded_w, coded_h) = (u32::from(avc1.width), u32::from(avc1.height));
        let (width, height) = if rotation % 180 == 0 {
            (coded_w, coded_h)
        } else {
            (coded_h, coded_w)
        };
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
            bail!("{name}: the picture must be non-empty and at most 16 megapixels ({width}x{height})");
        }
        let nal_length = usize::from(avc1.avcc.length_size_minus_one & 3) + 1;
        let colour = avc1.avcc.sequence_parameter_sets.first().map_or_else(
            || super::colour::Colour::from_sps(&[], coded_w, coded_h),
            |sps| super::colour::Colour::from_sps(&sps.bytes, coded_w, coded_h),
        );
        let mut header = Vec::new();
        for nal in avc1
            .avcc
            .sequence_parameter_sets
            .iter()
            .chain(&avc1.avcc.picture_parameter_sets)
        {
            push_nal(&mut header, &nal.bytes);
        }
        if header.is_empty() {
            bail!("{name} has no H.264 parameter sets");
        }

        let count = stbl.stsz.sample_count as usize;
        if count == 0 {
            bail!("{name} has no frames");
        }
        if count > MAX_FRAMES {
            bail!("{name} has {count} frames; at most {MAX_FRAMES} are supported");
        }
        let timescale = track.timescale();
        if timescale == 0 {
            bail!("{name} has a zero timescale");
        }
        let mut pts: Vec<i64> = Vec::with_capacity(count);
        let mut dts: u64 = 0;
        for entry in &stbl.stts.entries {
            let repeat = entry.sample_count as usize;
            if pts.len() + repeat > count {
                bail!("{name}: the time-to-sample table lists more frames than the file has");
            }
            for _ in 0..repeat {
                pts.push(dts as i64);
                dts += u64::from(entry.sample_delta);
            }
        }
        if pts.len() != count {
            bail!("{name}: the time-to-sample table does not match the frame count");
        }
        if let Some(ctts) = &stbl.ctts {
            let mut at = 0usize;
            for entry in &ctts.entries {
                let repeat = entry.sample_count as usize;
                if at + repeat > count {
                    bail!("{name}: the composition-offset table lists more frames than the file has");
                }
                for p in &mut pts[at..at + repeat] {
                    *p += i64::from(entry.sample_offset);
                }
                at += repeat;
            }
        }
        let mut sync = vec![stbl.stss.is_none(); count];
        if let Some(stss) = &stbl.stss {
            for &id in &stss.entries {
                match (id as usize).checked_sub(1).filter(|&i| i < count) {
                    Some(i) => sync[i] = true,
                    None => bail!("{name}: the sync-sample table points outside the file"),
                }
            }
        }
        if !sync[0] {
            bail!("{name}: the first frame is not a keyframe");
        }

        let starts: Vec<usize> = (0..count).filter(|&i| sync[i]).collect();
        let mut gops = Vec::with_capacity(starts.len());
        let mut display: Vec<i64> = Vec::with_capacity(count);
        for (j, &first) in starts.iter().enumerate() {
            let next = starts.get(j + 1).copied();
            let end = next.unwrap_or(count);
            let mut group: Vec<i64> = pts[first..end].to_vec();
            let open = group.iter().any(|&p| p < pts[first])
                || next.is_some_and(|n| group.iter().any(|&p| p >= pts[n]));
            if open {
                bail!("{name} uses open GOPs (frames that depend on the previous keyframe group), which are not supported; re-encode with `ffmpeg -i in.mp4 -c:v libx264 -x264-params open-gop=0 -an out.mp4`");
            }
            group.sort_unstable();
            gops.push(Gop {
                first_sample: first,
                end_sample: end,
                first_display: display.len(),
            });
            display.extend(group);
        }
        let origin = display[0];
        let frame_times: Vec<f64> = display
            .iter()
            .map(|&p| (p - origin) as f64 / f64::from(timescale))
            .collect();
        let last = frame_times[count - 1];
        let step = if count >= 2 {
            last - frame_times[count - 2]
        } else {
            1.0 / 30.0
        };
        if !(step > 0.0) || !last.is_finite() {
            bail!("{name} has frame times that do not increase");
        }
        Ok(Self {
            width,
            height,
            rotation,
            colour,
            coded: (coded_w, coded_h),
            track_id,
            nal_length,
            header,
            frame_times,
            duration: last + step,
            gops,
            has_audio,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frame_times.len()
    }

    /// Average frame rate.
    pub fn fps(&self) -> f64 {
        self.frame_count() as f64 / self.duration
    }

    /// The display frame shown at `media` seconds: the last frame whose time is not after it,
    /// clamped to the first and last frame.
    pub fn frame_at(&self, media: f64) -> usize {
        let shown = self
            .frame_times
            .partition_point(|&t| t <= media + 1e-6);
        shown.saturating_sub(1)
    }

    /// The GOP holding a display frame and the frame's position inside it.
    pub(super) fn gop_of(&self, display: usize) -> (usize, usize) {
        let gop = self
            .gops
            .partition_point(|g| g.first_display <= display)
            .saturating_sub(1);
        (gop, display - self.gops[gop].first_display)
    }
}
