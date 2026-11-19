//! Lossless row bands for fitted SVGs under cache pressure. Bands outside the visible rows are skipped.

use lz4_flex::block::{compress_into, decompress_into, get_maximum_output_size};

pub(super) const BAND_BYTES: usize = 256 * 1024;

pub(super) struct PackedImage {
    bands: Vec<Box<[u8]>>,
    pub(super) rows_per_band: usize,
    pub(super) bytes: usize,
}

impl PackedImage {
    /// Keep compression only when it saves at least a quarter of the original storage.
    pub(super) fn pack(pixels: &[u8], stride: usize) -> Option<Self> {
        if stride > BAND_BYTES {
            return None;
        }
        let rows_per_band = BAND_BYTES / stride;
        let band_size = rows_per_band * stride;
        let count = pixels.len().div_ceil(band_size);
        let mut bands = Vec::with_capacity(count);
        let mut bytes = count * size_of::<Box<[u8]>>();
        let ceiling = pixels.len() - pixels.len() / 4;
        let mut scratch = vec![0; get_maximum_output_size(band_size.min(pixels.len()))];
        for block in pixels.chunks(band_size) {
            let n = compress_into(block, &mut scratch).ok()?;
            bytes += n;
            if bytes >= ceiling {
                return None;
            }
            bands.push(scratch[..n].into());
        }
        Some(Self {
            bands,
            rows_per_band,
            bytes,
        })
    }

    pub(super) fn unpack(&self, band: usize, output: &mut [u8]) {
        let written =
            decompress_into(&self.bands[band], output).expect("internally compressed SVG band");
        debug_assert_eq!(written, output.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_exact_rgba_including_final_partial_band() {
        let stride = 4000;
        let rgba: Vec<_> = (0..stride * 131)
            .map(|i| ((i / 4 / 19 + i / stride) % 251) as u8)
            .collect();
        let packed = PackedImage::pack(&rgba, stride).unwrap();
        assert!(packed.bytes < rgba.len() / 2);
        for (band, expected) in rgba.chunks(packed.rows_per_band * stride).enumerate() {
            let mut actual = vec![0; expected.len()];
            packed.unpack(band, &mut actual);
            assert_eq!(actual, expected);
        }
    }
}
