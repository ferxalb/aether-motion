//! Colour tags of an H.264 stream and the YUV to RGBA conversion they select.
//!
//! The SPS carries optional VUI fields: `video_full_range_flag` and `matrix_coefficients`. Players
//! honour them, and when the matrix is missing they assume BT.709 for HD pictures and BT.601
//! otherwise. The decoder's built-in conversion is used for limited-range BT.601; every other case
//! (including the limited-range BT.709 this renderer's encoder writes) goes through `to_rgba`,
//! fixed-point integer math with nearest-neighbour chroma, so output is identical on every machine.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour {
    pub matrix: Matrix,
    pub full_range: bool,
    /// The stream states its matrix; otherwise it was inferred from the picture size.
    pub tagged: bool,
}

impl Colour {
    /// Reads the tags from an SPS NAL unit (without start code). Unreadable SPS data is not an
    /// error: the result is then the untagged guess for the picture size.
    pub fn from_sps(sps: &[u8], width: u32, height: u32) -> Self {
        let (matrix, full_range) = parse_vui(sps).unwrap_or((None, false));
        let guess = if height >= 720 || width >= 1280 {
            Matrix::Bt709
        } else {
            Matrix::Bt601
        };
        match matrix {
            Some(1) => Self {
                matrix: Matrix::Bt709,
                full_range,
                tagged: true,
            },
            Some(5 | 6) => Self {
                matrix: Matrix::Bt601,
                full_range,
                tagged: true,
            },
            // Unspecified (2), absent, or a matrix this renderer does not model.
            _ => Self {
                matrix: guess,
                full_range,
                tagged: false,
            },
        }
    }

    /// The decoder's own conversion gives this result, so `to_rgba` is not needed.
    pub fn is_decoder_default(&self) -> bool {
        self.matrix == Matrix::Bt601 && !self.full_range
    }
}

/// Reads bits from RBSP data (emulation-prevention bytes already removed).
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.at / 8)?;
        let bit = (byte >> (7 - self.at % 8)) & 1;
        self.at += 1;
        Some(u32::from(bit))
    }

    fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | self.bit()?;
        }
        Some(value)
    }

    /// Unsigned Exp-Golomb.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let tail = if zeros > 0 { self.bits(zeros)? } else { 0 };
        Some((1u32 << zeros) - 1 + tail)
    }

    /// Signed Exp-Golomb.
    fn se(&mut self) -> Option<i32> {
        let k = self.ue()?;
        let magnitude = i32::try_from(k.div_ceil(2)).ok()?;
        Some(if k % 2 == 1 { magnitude } else { -magnitude })
    }
}

/// `00 00 03` becomes `00 00`.
fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &byte in nal {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

/// `(matrix_coefficients if stated, video_full_range_flag)` from the SPS, `None` if it cannot be
/// parsed up to the VUI.
fn parse_vui(sps: &[u8]) -> Option<(Option<u8>, bool)> {
    let rbsp = unescape(sps.get(1..)?);
    let mut r = Bits { data: &rbsp, at: 0 };
    let profile = r.bits(8)?;
    r.bits(16)?; // constraint flags and level_idc
    r.ue()?; // seq_parameter_set_id
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        let chroma = r.ue()?;
        if chroma == 3 {
            r.bit()?;
        }
        r.ue()?;
        r.ue()?;
        r.bit()?;
        if r.bit()? == 1 {
            for list in 0..if chroma == 3 { 12 } else { 8 } {
                if r.bit()? == 1 {
                    let mut next = 8_i32;
                    for _ in 0..if list < 6 { 16 } else { 64 } {
                        if next != 0 {
                            next = (next + r.se()? + 256) % 256;
                        }
                    }
                }
            }
        }
    }
    r.ue()?; // log2_max_frame_num_minus4
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.bit()?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?; // max_num_ref_frames
    r.bit()?; // gaps_in_frame_num_value_allowed_flag
    r.ue()?;
    r.ue()?;
    if r.bit()? == 0 {
        r.bit()?; // mb_adaptive_frame_field_flag
    }
    r.bit()?; // direct_8x8_inference_flag
    if r.bit()? == 1 {
        for _ in 0..4 {
            r.ue()?;
        }
    }
    if r.bit()? == 0 {
        return Some((None, false)); // no VUI
    }
    if r.bit()? == 1 && r.bits(8)? == 255 {
        r.bits(32)?; // sample aspect ratio
    }
    if r.bit()? == 1 {
        r.bit()?; // overscan_appropriate_flag
    }
    if r.bit()? == 0 {
        return Some((None, false)); // no video signal type
    }
    r.bits(3)?; // video_format
    let full_range = r.bit()? == 1;
    if r.bit()? == 0 {
        return Some((None, full_range));
    }
    r.bits(8)?; // colour_primaries
    r.bits(8)?; // transfer_characteristics
    let matrix = r.bits(8)? as u8;
    Some((Some(matrix), full_range))
}

/// Converts I420 planes to RGBA (alpha 255). `strides` are `(y, u, v)`; `out` is `w * h * 4` bytes.
pub fn to_rgba(
    (y_plane, u_plane, v_plane): (&[u8], &[u8], &[u8]),
    (y_stride, u_stride, v_stride): (usize, usize, usize),
    (w, h): (usize, usize),
    colour: Colour,
    out: &mut [u8],
) {
    let (kr, kb, kg_u, kg_v) = match colour.matrix {
        Matrix::Bt601 => (1.402, 1.772, 0.344_136, 0.714_136),
        Matrix::Bt709 => (1.5748, 1.8556, 0.1873, 0.4681),
    };
    let (luma_gain, chroma_gain, luma_offset) = if colour.full_range {
        (1.0, 1.0, 0)
    } else {
        (255.0 / 219.0, 255.0 / 224.0, 16)
    };
    let q = |value: f64| (value * 65536.0).round() as i32;
    let (yq, rv, bu) = (q(luma_gain), q(chroma_gain * kr), q(chroma_gain * kb));
    let (gu, gv) = (q(chroma_gain * kg_u), q(chroma_gain * kg_v));
    for y in 0..h {
        for x in 0..w {
            let luma = i32::from(y_plane[y * y_stride + x]) - luma_offset;
            let cb = i32::from(u_plane[(y / 2) * u_stride + x / 2]) - 128;
            let cr = i32::from(v_plane[(y / 2) * v_stride + x / 2]) - 128;
            let base = luma * yq + 32768;
            let pixel = &mut out[(y * w + x) * 4..(y * w + x) * 4 + 4];
            pixel[0] = ((base + rv * cr) >> 16).clamp(0, 255) as u8;
            pixel[1] = ((base - gu * cb - gv * cr) >> 16).clamp(0, 255) as u8;
            pixel[2] = ((base + bu * cb) >> 16).clamp(0, 255) as u8;
            pixel[3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(colour: Colour, y: u8, cb: u8, cr: u8) -> [u8; 3] {
        let mut out = [0u8; 4 * 4];
        let (yp, up, vp) = ([y; 4], [cb], [cr]);
        to_rgba((&yp, &up, &vp), (2, 1, 1), (2, 2), colour, &mut out);
        [out[0], out[1], out[2]]
    }

    const LIMITED_709: Colour = Colour {
        matrix: Matrix::Bt709,
        full_range: false,
        tagged: true,
    };
    const LIMITED_601: Colour = Colour {
        matrix: Matrix::Bt601,
        full_range: false,
        tagged: true,
    };
    const FULL_601: Colour = Colour {
        matrix: Matrix::Bt601,
        full_range: true,
        tagged: true,
    };

    #[test]
    fn limited_range_maps_black_and_white() {
        assert_eq!(one(LIMITED_709, 16, 128, 128), [0, 0, 0]);
        assert_eq!(one(LIMITED_709, 235, 128, 128), [255, 255, 255]);
        assert_eq!(one(FULL_601, 0, 128, 128), [0, 0, 0]);
        assert_eq!(one(FULL_601, 255, 128, 128), [255, 255, 255]);
    }

    #[test]
    fn matrices_differ_on_saturated_colors() {
        // BT.709 red is Y 63, Cb 102, Cr 240 (limited range); BT.601 red is Y 81, Cb 90, Cr 240.
        let red_709 = one(LIMITED_709, 63, 102, 240);
        assert!(
            red_709[0] >= 250 && red_709[1] <= 6 && red_709[2] <= 6,
            "{red_709:?}"
        );
        let red_601 = one(LIMITED_601, 81, 90, 240);
        assert!(
            red_601[0] >= 250 && red_601[1] <= 6 && red_601[2] <= 6,
            "{red_601:?}"
        );
        // Reading BT.601 red with the 709 matrix gives a visibly different color.
        assert_ne!(one(LIMITED_709, 81, 90, 240), red_601);
    }

    #[test]
    fn untagged_pictures_follow_their_size() {
        assert_eq!(Colour::from_sps(&[], 1920, 1080).matrix, Matrix::Bt709);
        assert_eq!(Colour::from_sps(&[], 640, 360).matrix, Matrix::Bt601);
        assert!(!Colour::from_sps(&[], 640, 360).tagged);
    }

    #[test]
    fn exp_golomb_codes_decode() {
        // 1 -> 0, 010 -> 1, 011 -> 2, 00100 -> 3 (bits: 1 010 011 00100 + padding).
        let data = [0b1010_0110, 0b0100_0000];
        let mut r = Bits { data: &data, at: 0 };
        assert_eq!(
            [r.ue(), r.ue(), r.ue(), r.ue()],
            [Some(0), Some(1), Some(2), Some(3)]
        );
        let mut s = Bits { data: &data, at: 0 };
        assert_eq!([s.se(), s.se(), s.se()], [Some(0), Some(1), Some(-1)]);
    }
}
