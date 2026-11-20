//! RGBA to limited-range BT.709 YUV 4:2:0 conversion for the native H.264 encoder.
//!
//! Rendered frames are sRGB, whose primaries are the BT.709 primaries, so the encoder converts
//! with the BT.709 matrix (Kr 0.2126, Kb 0.0722) into limited range (Y 16..235, Cb/Cr 16..240)
//! and `make_encoder` signals exactly that in the SPS VUI (primaries, transfer, and matrix 1,
//! `video_full_range_flag` 0). Chroma is the average of each 2x2 block (centered siting, as the
//! previous BT.601 path did), converted after averaging.
//!
//! The math is fixed-point integer with round-to-nearest, so the planes are a pure function of
//! the RGBA bytes: the scalar and AVX2 builds of the same loop, every thread count, and every
//! render split give identical bytes. [`Yuv420`] owns the planes and is what the encoder reads.

use openh264::formats::YUVSource;

/// Fractional bits of the coefficients.
const SHIFT: u32 = 16;
/// BT.709 luma coefficients times 219/255 (limited-range luma excursion), scaled by 2^16 and
/// rounded so that 255 on every channel gives exactly 235.
const Y_COEF: [i32; 3] = [11_966, 40_254, 4_064];
/// BT.709 Cb and Cr coefficients times 224/255 (chroma excursion), scaled by 2^16. Each row sums
/// to zero, so every gray maps to exactly 128.
const CB_COEF: [i32; 3] = [-6_596, -22_188, 28_784];
const CR_COEF: [i32; 3] = [28_784, -26_145, -2_639];

/// One limited-range BT.709 Y value from 8-bit sRGB.
#[inline(always)]
fn luma(r: i32, g: i32, b: i32) -> u8 {
    let y = (Y_COEF[0] * r + Y_COEF[1] * g + Y_COEF[2] * b + (16 << SHIFT) + (1 << (SHIFT - 1)))
        >> SHIFT;
    y.clamp(16, 235) as u8
}

/// Cb and Cr from the channel sums of a 2x2 block (each sum 0..=1020).
#[inline(always)]
fn chroma(r: i32, g: i32, b: i32) -> (u8, u8) {
    const S: u32 = SHIFT + 2;
    let bias = (128 << S) + (1 << (S - 1));
    let cb = (CB_COEF[0] * r + CB_COEF[1] * g + CB_COEF[2] * b + bias) >> S;
    let cr = (CR_COEF[0] * r + CR_COEF[1] * g + CR_COEF[2] * b + bias) >> S;
    (cb.clamp(16, 240) as u8, cr.clamp(16, 240) as u8)
}

/// Converts one RGBA row into a luma row. Kept separate from the chroma loop: one output per
/// input pixel is a shape the compiler vectorizes.
#[inline(always)]
fn luma_row(rgba: &[u8], y: &mut [u8]) {
    let (pixels, _) = rgba.as_chunks::<4>();
    for (dst, p) in y.iter_mut().zip(pixels) {
        *dst = luma(i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
    }
}

/// Converts one pair of RGBA rows into one row of each chroma plane.
#[inline(always)]
fn chroma_row(top: &[u8], bottom: &[u8], u: &mut [u8], v: &mut [u8]) {
    let (top, _) = top.as_chunks::<8>();
    let (bottom, _) = bottom.as_chunks::<8>();
    for (((t, b), u), v) in top.iter().zip(bottom).zip(u.iter_mut()).zip(v.iter_mut()) {
        let sum = |c: usize| {
            i32::from(t[c]) + i32::from(t[c + 4]) + i32::from(b[c]) + i32::from(b[c + 4])
        };
        (*u, *v) = chroma(sum(0), sum(1), sum(2));
    }
}

/// Converts one pair of RGBA rows into two luma rows and one row of each chroma plane.
#[inline(always)]
fn row_pair(top: &[u8], bottom: &[u8], y0: &mut [u8], y1: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    luma_row(top, y0);
    luma_row(bottom, y1);
    chroma_row(top, bottom, u, v);
}

fn convert_scalar(rgba: &[u8], width: usize, planes: (&mut [u8], &mut [u8], &mut [u8])) {
    let (y, u, v) = planes;
    let half = width / 2;
    for (((rows, ys), u), v) in rgba
        .chunks_exact(width * 8)
        .zip(y.chunks_exact_mut(width * 2))
        .zip(u.chunks_exact_mut(half))
        .zip(v.chunks_exact_mut(half))
    {
        let (top, bottom) = rows.split_at(width * 4);
        let (y0, y1) = ys.split_at_mut(width);
        row_pair(top, bottom, y0, y1, u, v);
    }
}

/// The same loop compiled with AVX2. Integer arithmetic is exact in every width, so it writes the
/// same bytes as the baseline build.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn convert_avx2(rgba: &[u8], width: usize, planes: (&mut [u8], &mut [u8], &mut [u8])) {
    convert_scalar(rgba, width, planes);
}

/// Whether the AVX2 build runs: the CPU has AVX2 and `AETHER_SIMD` is not `off` (the switch the
/// paint kernels honour, to compare speed and confirm identical output).
#[cfg(target_arch = "x86_64")]
fn use_avx2() -> bool {
    static AVX2: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVX2.get_or_init(|| {
        std::arch::is_x86_feature_detected!("avx2")
            && std::env::var("AETHER_SIMD").map_or(true, |value| value != "off")
    })
}

/// A YUV 4:2:0 picture in limited-range BT.709, planes stored contiguously (Y, then Cb, then Cr).
pub struct Yuv420 {
    data: Vec<u8>,
    width: usize,
    height: usize,
}

impl Yuv420 {
    /// A black picture. Both dimensions must be even (checked by `check_video`).
    pub fn new(width: usize, height: usize) -> Self {
        assert!(
            width.is_multiple_of(2) && height.is_multiple_of(2),
            "yuv420p needs even dimensions"
        );
        let luma = width * height;
        let mut data = vec![16; luma + luma / 2];
        data[luma..].fill(128);
        Self {
            data,
            width,
            height,
        }
    }

    /// Converts an opaque RGBA frame of this picture's size (alpha is ignored).
    pub fn read_rgba(&mut self, rgba: &[u8]) {
        assert_eq!(rgba.len(), self.width * self.height * 4, "RGBA frame size");
        let luma = self.width * self.height;
        let (y, chroma) = self.data.split_at_mut(luma);
        let (u, v) = chroma.split_at_mut(luma / 4);
        #[cfg(target_arch = "x86_64")]
        if use_avx2() {
            // SAFETY: `use_avx2` confirmed the CPU supports AVX2.
            unsafe { convert_avx2(rgba, self.width, (y, u, v)) };
            return;
        }
        convert_scalar(rgba, self.width, (y, u, v));
    }
}

impl YUVSource for Yuv420 {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn strides(&self) -> (usize, usize, usize) {
        (self.width, self.width / 2, self.width / 2)
    }

    fn y(&self) -> &[u8] {
        &self.data[..self.width * self.height]
    }

    fn u(&self) -> &[u8] {
        let luma = self.width * self.height;
        &self.data[luma..luma + luma / 4]
    }

    fn v(&self) -> &[u8] {
        let luma = self.width * self.height;
        &self.data[luma + luma / 4..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts a solid 2x2 picture of one color.
    fn solid(rgb: [u8; 3]) -> [u8; 3] {
        let mut picture = Yuv420::new(2, 2);
        let rgba: Vec<u8> = (0..4).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
        picture.read_rgba(&rgba);
        [picture.y()[0], picture.u()[0], picture.v()[0]]
    }

    #[test]
    fn bt709_limited_range_reference_colors() {
        // ITU-R BT.709 limited-range codes of full-intensity primaries, secondaries, and grays.
        let cases = [
            ([0, 0, 0], [16, 128, 128]),
            ([255, 255, 255], [235, 128, 128]),
            ([128, 128, 128], [126, 128, 128]),
            ([255, 0, 0], [63, 102, 240]),
            ([0, 255, 0], [173, 42, 26]),
            ([0, 0, 255], [32, 240, 118]),
            ([255, 255, 0], [219, 16, 138]),
            ([0, 255, 255], [188, 154, 16]),
            ([255, 0, 255], [78, 214, 230]),
        ];
        for (rgb, expected) in cases {
            assert_eq!(solid(rgb), expected, "{rgb:?}");
        }
    }

    #[test]
    fn every_gray_has_neutral_chroma() {
        for level in 0..=255u8 {
            let [y, u, v] = solid([level; 3]);
            assert_eq!((u, v), (128, 128), "gray {level}");
            let exact = 16.0 + 219.0 * f64::from(level) / 255.0;
            assert!(
                (f64::from(y) - exact).abs() <= 0.5 + 1e-9,
                "gray {level}: {y}"
            );
        }
    }

    #[test]
    fn matches_the_floating_point_definition() {
        // Every 8-bit code within half a step (plus fixed-point slack) of the real-valued BT.709
        // equations, over a coarse lattice of the RGB cube.
        let (kr, kb) = (0.2126, 0.0722);
        for r in (0..=255u8).step_by(15) {
            for g in (0..=255u8).step_by(15) {
                for b in (0..=255u8).step_by(15) {
                    let (fr, fg, fb) = (f64::from(r), f64::from(g), f64::from(b));
                    let l = kr * fr + (1.0 - kr - kb) * fg + kb * fb;
                    let y = 16.0 + 219.0 / 255.0 * l;
                    let cb = 128.0 + 224.0 / 255.0 * (fb - l) / (2.0 * (1.0 - kb));
                    let cr = 128.0 + 224.0 / 255.0 * (fr - l) / (2.0 * (1.0 - kr));
                    let got = solid([r, g, b]);
                    for (code, exact) in got.iter().zip([y, cb, cr]) {
                        assert!(
                            (f64::from(*code) - exact).abs() <= 0.51,
                            "{:?}: {got:?} vs {exact}",
                            (r, g, b)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn chroma_averages_each_two_by_two_block() {
        // Left block: red over blue; right block: white. Chroma of the left block is the
        // conversion of the averaged color, luma stays per pixel.
        let mut picture = Yuv420::new(4, 2);
        let px = |c: [u8; 3]| [c[0], c[1], c[2], 255];
        let rgba: Vec<u8> = [
            px([255, 0, 0]),
            px([255, 0, 0]),
            px([255; 3]),
            px([255; 3]),
            px([0, 0, 255]),
            px([0, 0, 255]),
            px([255; 3]),
            px([255; 3]),
        ]
        .concat();
        picture.read_rgba(&rgba);
        assert_eq!(picture.y(), [63, 63, 235, 235, 32, 32, 235, 235]);
        let average = solid([128, 0, 128]);
        // Averaging the sums (127.5 per channel) can differ from the rounded 128 by one code.
        assert!(
            picture.u()[0].abs_diff(average[1]) <= 1,
            "{:?}",
            picture.u()
        );
        assert!(
            picture.v()[0].abs_diff(average[2]) <= 1,
            "{:?}",
            picture.v()
        );
        assert_eq!((picture.u()[1], picture.v()[1]), (128, 128));
    }

    #[test]
    fn simd_and_scalar_builds_agree() {
        // A deterministic pseudo-random frame wide enough for every vector width.
        let (width, height) = (70, 6);
        let mut state = 0x2545_f491_u32;
        let rgba: Vec<u8> = (0..width * height * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        let mut picture = Yuv420::new(width, height);
        picture.read_rgba(&rgba);
        let mut reference = Yuv420::new(width, height);
        let luma = width * height;
        let (y, chroma) = reference.data.split_at_mut(luma);
        let (u, v) = chroma.split_at_mut(luma / 4);
        convert_scalar(&rgba, width, (y, u, v));
        assert_eq!(picture.data, reference.data);
    }
}
