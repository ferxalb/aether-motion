//! MP4 length-prefixed NAL units to Annex B (start-code separated), the form the decoder reads.
use anyhow::{bail, Result};

const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Appends one Annex B NAL unit (start code plus payload) to `out`.
pub(super) fn push_nal(out: &mut Vec<u8>, nal: &[u8]) {
    out.extend_from_slice(&START_CODE);
    out.extend_from_slice(nal);
}

/// Rewrites one sample: each `nal_length`-byte big-endian length prefix becomes a start code.
pub(super) fn sample_to_annex_b(sample: &[u8], nal_length: usize, out: &mut Vec<u8>) -> Result<()> {
    if !(1..=4).contains(&nal_length) {
        bail!("unsupported NAL length size {nal_length}");
    }
    let mut rest = sample;
    while !rest.is_empty() {
        if rest.len() < nal_length {
            bail!("truncated NAL length prefix in a video sample");
        }
        let (prefix, tail) = rest.split_at(nal_length);
        let length = prefix
            .iter()
            .fold(0usize, |n, &b| (n << 8) | usize::from(b));
        if length == 0 || length > tail.len() {
            bail!("invalid NAL unit length {length} in a video sample");
        }
        push_nal(out, &tail[..length]);
        rest = &tail[length..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefixes_become_start_codes() {
        let sample = [0, 0, 0, 2, 0x65, 0xAA, 0, 0, 0, 1, 0x41];
        let mut out = Vec::new();
        sample_to_annex_b(&sample, 4, &mut out).unwrap();
        assert_eq!(out, [0, 0, 0, 1, 0x65, 0xAA, 0, 0, 0, 1, 0x41]);
        let mut short = Vec::new();
        sample_to_annex_b(&[0, 1, 0x41], 2, &mut short).unwrap();
        assert_eq!(short, [0, 0, 0, 1, 0x41]);
    }

    #[test]
    fn malformed_samples_are_rejected() {
        let mut out = Vec::new();
        assert!(sample_to_annex_b(&[0, 0, 0, 9, 1], 4, &mut out).is_err());
        assert!(sample_to_annex_b(&[0, 0], 4, &mut out).is_err());
        assert!(sample_to_annex_b(&[0, 0, 0, 0], 4, &mut out).is_err());
        assert!(sample_to_annex_b(&[1], 5, &mut out).is_err());
    }
}
