//! CSS `filter` color functions as a chain of affine color steps.
//!
//! Each function follows the Filter Effects Level 1 matrices in sRGB space: `brightness`,
//! `contrast`, and `invert` scale and offset each channel; `saturate`, `grayscale`, `sepia`, and
//! `hue-rotate` are 3x3 matrices; `opacity` multiplies alpha. Values are clamped to 0..=1 after
//! every step, as a chain of separate filter primitives would. The chain grades whole frames on
//! the root (`grade_frame`), where channel-wise chains run through a 256-entry lookup table.

use crate::model::ColorFilter;
use crate::render::paint::float_ops::round_to_u8;

/// One step: `rgb' = clamp(m * rgb + offset)`, then `alpha' = alpha * opacity`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    m: [[f32; 3]; 3],
    offset: [f32; 3],
    opacity: f32,
}

impl Step {
    const IDENTITY: Self = Self {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        offset: [0.0; 3],
        opacity: 1.0,
    };

    fn channelwise(scale: f32, offset: f32) -> Self {
        Self {
            m: [[scale, 0.0, 0.0], [0.0, scale, 0.0], [0.0, 0.0, scale]],
            offset: [offset; 3],
            ..Self::IDENTITY
        }
    }

    fn matrix(m: [[f32; 3]; 3]) -> Self {
        Self {
            m,
            ..Self::IDENTITY
        }
    }

    fn from_filter(filter: ColorFilter) -> Self {
        match filter {
            ColorFilter::Brightness(b) => Self::channelwise(b, 0.0),
            ColorFilter::Contrast(c) => Self::channelwise(c, 0.5 - 0.5 * c),
            ColorFilter::Invert(a) => Self::channelwise(1.0 - 2.0 * a, a),
            ColorFilter::Opacity(a) => Self {
                opacity: a,
                ..Self::IDENTITY
            },
            ColorFilter::Saturate(s) => Self::matrix([
                [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
                [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
                [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
            ]),
            ColorFilter::Grayscale(a) => {
                let k = 1.0 - a;
                Self::matrix([
                    [
                        0.2126 + 0.7874 * k,
                        0.7152 - 0.7152 * k,
                        0.0722 - 0.0722 * k,
                    ],
                    [
                        0.2126 - 0.2126 * k,
                        0.7152 + 0.2848 * k,
                        0.0722 - 0.0722 * k,
                    ],
                    [
                        0.2126 - 0.2126 * k,
                        0.7152 - 0.7152 * k,
                        0.0722 + 0.9278 * k,
                    ],
                ])
            }
            ColorFilter::Sepia(a) => {
                let k = 1.0 - a;
                Self::matrix([
                    [0.393 + 0.607 * k, 0.769 - 0.769 * k, 0.189 - 0.189 * k],
                    [0.349 - 0.349 * k, 0.686 + 0.314 * k, 0.168 - 0.168 * k],
                    [0.272 - 0.272 * k, 0.534 - 0.534 * k, 0.131 + 0.869 * k],
                ])
            }
            ColorFilter::HueRotate(degrees) => {
                let (sin, cos) = degrees.to_radians().sin_cos();
                Self::matrix([
                    [
                        0.213 + cos * 0.787 - sin * 0.213,
                        0.715 - cos * 0.715 - sin * 0.715,
                        0.072 - cos * 0.072 + sin * 0.928,
                    ],
                    [
                        0.213 - cos * 0.213 + sin * 0.143,
                        0.715 + cos * 0.285 + sin * 0.140,
                        0.072 - cos * 0.072 - sin * 0.283,
                    ],
                    [
                        0.213 - cos * 0.213 - sin * 0.787,
                        0.715 - cos * 0.715 + sin * 0.715,
                        0.072 + cos * 0.928 + sin * 0.072,
                    ],
                ])
            }
        }
    }

    /// Whether the step leaves every color unchanged (within float rounding of the matrices).
    fn is_neutral(&self) -> bool {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        self.opacity == 1.0
            && self.offset.iter().all(|&o| close(o, 0.0))
            && (0..3).all(|i| (0..3).all(|j| close(self.m[i][j], f32::from(u8::from(i == j)))))
    }

    /// Whether the step treats each channel on its own (a diagonal matrix with equal entries).
    fn is_channelwise(&self) -> bool {
        let m = self.m;
        m[0][1] == 0.0
            && m[0][2] == 0.0
            && m[1][0] == 0.0
            && m[1][2] == 0.0
            && m[2][0] == 0.0
            && m[2][1] == 0.0
    }

    fn apply(&self, [r, g, b]: [f32; 3]) -> [f32; 3] {
        let row = |i: usize| {
            let m = self.m[i];
            (m[0] * r + m[1] * g + m[2] * b + self.offset[i]).clamp(0.0, 1.0)
        };
        [row(0), row(1), row(2)]
    }
}

/// A parsed `filter` color chain, ready to run on unpremultiplied colors.
#[derive(Clone, Debug, Default)]
pub(in crate::render) struct ColorChain {
    steps: Vec<Step>,
}

impl ColorChain {
    pub(in crate::render) fn new(filters: &[ColorFilter]) -> Self {
        Self {
            steps: filters
                .iter()
                .map(|&f| Step::from_filter(f))
                .filter(|step| !step.is_neutral())
                .collect(),
        }
    }

    pub(in crate::render) fn is_identity(&self) -> bool {
        self.steps.is_empty()
    }

    /// Runs the chain on an unpremultiplied color (0..=1 per channel) and its alpha.
    pub(in crate::render) fn apply(&self, mut rgb: [f32; 3], mut alpha: f32) -> ([f32; 3], f32) {
        for step in &self.steps {
            rgb = step.apply(rgb);
            alpha *= step.opacity;
        }
        (rgb, alpha)
    }

    /// Recolors one premultiplied texel (channels and alpha in 0..=255, alpha above 0).
    #[inline]
    pub(in crate::render) fn recolor(&self, [r, g, b, a]: [f32; 4]) -> [f32; 4] {
        let unpremultiply = |v: f32| (v / a).clamp(0.0, 1.0);
        let ([cr, cg, cb], alpha) = self.apply(
            [unpremultiply(r), unpremultiply(g), unpremultiply(b)],
            a / 255.0,
        );
        let scale = alpha * 255.0;
        [cr * scale, cg * scale, cb * scale, scale]
    }

    /// For chains made only of channel-wise steps, the 8-bit output of every 8-bit input value.
    fn lut(&self) -> Option<[[u8; 256]; 3]> {
        if !self.steps.iter().all(Step::is_channelwise) {
            return None;
        }
        let mut lut = [[0_u8; 256]; 3];
        for v in 0..256 {
            let x = v as f32 / 255.0;
            let out = self.apply([x; 3], 1.0).0;
            for (table, c) in lut.iter_mut().zip(out) {
                table[v] = round_to_u8(c * 255.0);
            }
        }
        Some(lut)
    }

    /// Grades opaque RGBA pixels in place (the root's `filter`; alpha is left at 255).
    pub(in crate::render) fn grade_frame(&self, pixels: &mut [u8]) {
        if self.is_identity() {
            return;
        }
        if let Some([lr, lg, lb]) = self.lut() {
            for px in pixels.as_chunks_mut::<4>().0 {
                px[0] = lr[usize::from(px[0])];
                px[1] = lg[usize::from(px[1])];
                px[2] = lb[usize::from(px[2])];
            }
            return;
        }
        // Leading channel-wise steps become per-channel tables of their exact float results.
        let split = self
            .steps
            .iter()
            .position(|step| !step.is_channelwise())
            .unwrap_or(self.steps.len());
        let (prefix, rest) = self.steps.split_at(split);
        let mut tables = [[0.0_f32; 256]; 3];
        for v in 0..256 {
            let mut rgb = [v as f32 / 255.0; 3];
            for step in prefix {
                rgb = step.apply(rgb);
            }
            for (table, c) in tables.iter_mut().zip(rgb) {
                table[v] = c;
            }
        }
        // Neighbouring pixels often repeat (flat areas), so the last result is reused.
        let mut last: Option<([u8; 3], [u8; 3])> = None;
        for px in pixels.as_chunks_mut::<4>().0 {
            let input = [px[0], px[1], px[2]];
            let output = match last {
                Some((seen, out)) if seen == input => out,
                _ => {
                    let mut rgb = [0, 1, 2].map(|c| tables[c][usize::from(input[c])]);
                    for step in rest {
                        rgb = step.apply(rgb);
                    }
                    let out = rgb.map(|c| round_to_u8(c * 255.0));
                    last = Some((input, out));
                    out
                }
            };
            px[..3].copy_from_slice(&output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(filters: &[ColorFilter], rgb: [f32; 3]) -> [f32; 3] {
        ColorChain::new(filters).apply(rgb, 1.0).0
    }

    #[test]
    fn neutral_amounts_are_identity() {
        let chain = ColorChain::new(&[
            ColorFilter::Brightness(1.0),
            ColorFilter::Saturate(1.0),
            ColorFilter::HueRotate(0.0),
            ColorFilter::Grayscale(0.0),
            ColorFilter::Invert(0.0),
        ]);
        assert!(chain.is_identity());
    }

    #[test]
    fn functions_follow_the_spec_matrices() {
        let [r, g, b] = run(&[ColorFilter::Grayscale(1.0)], [1.0, 0.0, 0.0]);
        assert!((r - 0.2126).abs() < 1e-6 && (g - r).abs() < 1e-6 && (b - r).abs() < 1e-6);
        assert_eq!(
            run(&[ColorFilter::Invert(1.0)], [0.25, 1.0, 0.0]),
            [0.75, 0.0, 1.0]
        );
        assert_eq!(
            run(&[ColorFilter::Contrast(2.0)], [0.75, 0.5, 0.1]),
            [1.0, 0.5, 0.0]
        );
        // Clamping between steps: brightening past white and darkening again stays below white.
        let [r, ..] = run(
            &[ColorFilter::Brightness(2.0), ColorFilter::Brightness(0.5)],
            [0.8, 0.0, 0.0],
        );
        assert!((r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn lut_matches_the_float_path() {
        for filters in [
            &[ColorFilter::Contrast(1.3), ColorFilter::Brightness(0.9)][..],
            &[ColorFilter::Contrast(1.08), ColorFilter::Saturate(1.15)][..],
            &[ColorFilter::Sepia(0.4), ColorFilter::Invert(0.2)][..],
        ] {
            check_grade(filters);
        }
    }

    fn check_grade(filters: &[ColorFilter]) {
        let chain = ColorChain::new(filters);
        let mut fast = (0..=255_u8)
            .flat_map(|v| [v, 255 - v, v / 2, 255])
            .collect::<Vec<_>>();
        let mut slow = fast.clone();
        chain.grade_frame(&mut fast);
        for px in slow.as_chunks_mut::<4>().0 {
            let rgb = [px[0], px[1], px[2]].map(|c| f32::from(c) / 255.0);
            let (out, _) = chain.apply(rgb, 1.0);
            for (channel, c) in px.iter_mut().zip(out) {
                *channel = (c * 255.0).round() as u8;
            }
        }
        assert_eq!(fast, slow);
    }
}
