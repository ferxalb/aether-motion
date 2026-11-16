//! `text-shadow`: a blurred copy of the glyph coverage, built once per text run and blitted under it.
//!
//! The mask is expressed relative to the text block's top-left pixel and does not include the
//! shadow offset, so moving or animating the element reuses it. Every input of the mask is part of
//! the run's cache key, which keeps frames independent of the order they are rendered in.

use crate::model::{Color, Shadow};
use crate::render::geometry::PixelRect;
use crate::render::text::raster::{baseline_offset, pen_offset, Glyph, RunLine};
use anyhow::{anyhow, Result};

/// Largest mask, in pixels; larger ones need a smaller blur or `--scale`.
const MAX_MASK_PIXELS: usize = 32 * 1024 * 1024;

/// Blurred coverage of a whole text block plus how it is painted.
pub(in crate::render) struct ShadowMask {
    /// Position of the mask's top-left pixel relative to the block origin.
    x0: i32,
    y0: i32,
    width: usize,
    height: usize,
    alpha: Vec<u8>,
    color: Color,
    /// Shadow offset in paint pixels.
    offset: (f32, f32),
}

/// Block geometry in paint pixels that decides where each line's glyphs land.
pub(super) struct BlockLayout {
    pub(super) width: f32,
    pub(super) size: f32,
    pub(super) line_height: f32,
    pub(super) center_x: bool,
    pub(super) right: bool,
}

impl ShadowMask {
    pub(in crate::render) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.alpha.len()
    }

    /// Builds the mask for `lines` of `glyphs`. `unit` converts shadow px to paint pixels.
    /// Returns `None` when the text has no visible glyphs.
    pub(super) fn build(
        glyphs: &[Glyph],
        lines: &[RunLine],
        layout: &BlockLayout,
        shadow: Shadow,
        unit: f32,
    ) -> Result<Option<Self>> {
        let radii = box_radii(shadow.blur * unit / 2.0);
        let reach = radii.iter().sum::<usize>() as i32;
        // (glyph index, x, y) of every glyph with pixels, relative to the block origin.
        let mut placed = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let mut pen = pen_offset(layout.center_x, layout.right, layout.width, line.advance);
            let baseline = baseline_offset(index, layout.line_height, layout.size);
            for (glyph_index, glyph) in glyphs.iter().enumerate().take(line.last).skip(line.first) {
                let metrics = glyph.metrics;
                let x = (pen + metrics.xmin as f32).round() as i32;
                let y = (baseline - metrics.ymin as f32 - metrics.height as f32).round() as i32;
                pen += metrics.advance_width;
                if metrics.width > 0 && metrics.height > 0 {
                    placed.push((glyph_index, x, y));
                }
            }
        }
        let Some(bounds) = placed
            .iter()
            .fold(None, |bounds: Option<[i32; 4]>, &(i, x, y)| {
                let m = glyphs[i].metrics;
                let (x1, y1) = (x + m.width as i32, y + m.height as i32);
                Some(bounds.map_or([x, y, x1, y1], |b| {
                    [b[0].min(x), b[1].min(y), b[2].max(x1), b[3].max(y1)]
                }))
            })
        else {
            return Ok(None);
        };
        let (x0, y0) = (bounds[0] - reach, bounds[1] - reach);
        let width = (bounds[2] - bounds[0] + 2 * reach) as usize;
        let height = (bounds[3] - bounds[1] + 2 * reach) as usize;
        if width.saturating_mul(height) > MAX_MASK_PIXELS {
            return Err(anyhow!(
                "text-shadow needs a mask over {MAX_MASK_PIXELS} pixels; reduce the blur or use --scale"
            ));
        }
        let mut alpha = vec![0u8; width * height];
        for (glyph_index, x, y) in placed {
            let glyph = &glyphs[glyph_index];
            let (gw, gh) = (glyph.metrics.width, glyph.metrics.height);
            for row in 0..gh {
                let start = (y - y0) as usize + row;
                let base = start * width + (x - x0) as usize;
                for col in 0..gw {
                    // Overlapping glyph shadows composite "over" each other.
                    let (below, c) = (
                        u16::from(alpha[base + col]),
                        u16::from(glyph.coverage[row * gw + col]),
                    );
                    alpha[base + col] = (below + c - (below * c + 127) / 255) as u8;
                }
            }
        }
        blur(&mut alpha, width, height, &radii);
        Ok(Self::cropped(
            alpha,
            (x0, y0),
            (width, height),
            shadow,
            unit,
        ))
    }

    /// Keeps only the rows and columns that still hold coverage, so painting skips empty margins.
    fn cropped(
        alpha: Vec<u8>,
        (x0, y0): (i32, i32),
        (width, height): (usize, usize),
        shadow: Shadow,
        unit: f32,
    ) -> Option<Self> {
        let row_used = |y: usize| alpha[y * width..(y + 1) * width].iter().any(|&v| v > 0);
        let col_used = |x: usize| (0..height).any(|y| alpha[y * width + x] > 0);
        let top = (0..height).find(|&y| row_used(y))?;
        let bottom = (0..height).rfind(|&y| row_used(y))? + 1;
        let left = (0..width).find(|&x| col_used(x))?;
        let right = (0..width).rfind(|&x| col_used(x))? + 1;
        let cropped_width = right - left;
        let mut cropped = Vec::with_capacity(cropped_width * (bottom - top));
        for y in top..bottom {
            cropped.extend_from_slice(&alpha[y * width + left..y * width + right]);
        }
        Some(Self {
            x0: x0 + left as i32,
            y0: y0 + top as i32,
            width: cropped_width,
            height: bottom - top,
            alpha: cropped,
            color: shadow.color,
            offset: (shadow.x * unit, shadow.y * unit),
        })
    }

    /// Blits the mask for a block whose top-left pixel is `origin`, under `alpha` opacity.
    pub(super) fn paint(
        &self,
        pixels: &mut [u8],
        (width, height): (u32, u32),
        painted: &mut Option<PixelRect>,
        origin: (f32, f32),
        alpha: f32,
        zoom: f32,
    ) {
        super::mask::paint_mask(
            pixels,
            (width, height),
            painted,
            &self.alpha,
            (self.width, self.height),
            (
                origin.0 + (self.offset.0 + self.x0 as f32) * zoom,
                origin.1 + (self.offset.1 + self.y0 as f32) * zoom,
            ),
            (self.color, alpha),
            zoom,
        );
    }
}

/// Radii of three box blurs whose combined variance matches a Gaussian of deviation `sigma`
/// (CSS lets engines approximate the Gaussian this way). Empty when there is no blur.
fn box_radii(sigma: f32) -> Vec<usize> {
    if sigma <= 0.0 {
        return Vec::new();
    }
    const PASSES: f32 = 3.0;
    let ideal = (12.0 * sigma * sigma / PASSES + 1.0).sqrt();
    let mut small = ideal.floor() as i32;
    if small % 2 == 0 {
        small -= 1;
    }
    let small = small.max(1);
    let wide = small + 2;
    let wide_boxes = ((12.0 * sigma * sigma
        - PASSES * (small * small) as f32
        - 4.0 * PASSES * small as f32
        - 3.0 * PASSES)
        / (-4.0 * small as f32 - 4.0))
        .round() as i32;
    (0..3)
        .map(|pass| {
            let size = if pass < wide_boxes { small } else { wide };
            ((size - 1) / 2) as usize
        })
        .collect()
}

/// Fractional bits kept between passes so the faint tails of a blur survive rounding.
const FRACTION: u32 = 8;

/// Blurs an 8-bit mask with one box blur per radius on each axis; pixels outside count as zero.
/// Integer running sums keep it linear in the area and bit-exact on every platform.
fn blur(mask: &mut [u8], width: usize, height: usize, radii: &[usize]) {
    if radii.iter().all(|&radius| radius == 0) {
        return;
    }
    let mut work: Vec<u32> = mask.iter().map(|&v| u32::from(v) << FRACTION).collect();
    let mut line = vec![0u32; width.max(height)];
    let mut source = vec![0u32; width.max(height)];
    for &radius in radii.iter().filter(|&&radius| radius > 0) {
        for row in work.chunks_exact_mut(width) {
            box_line(row, radius, &mut source);
        }
        for x in 0..width {
            for y in 0..height {
                line[y] = work[y * width + x];
            }
            box_line(&mut line[..height], radius, &mut source);
            for y in 0..height {
                work[y * width + x] = line[y];
            }
        }
    }
    for (value, blurred) in mask.iter_mut().zip(&work) {
        *value = ((blurred + (1 << (FRACTION - 1))) >> FRACTION).min(255) as u8;
    }
}

/// Replaces each value by the rounded mean of the `2 * radius + 1` values around it.
/// `scratch` holds a copy of the input and must be at least as long as `values`.
fn box_line(values: &mut [u32], radius: usize, scratch: &mut [u32]) {
    let len = values.len();
    let size = (2 * radius + 1) as u32;
    let source = &mut scratch[..len];
    source.copy_from_slice(values);
    let mut sum: u32 = source.iter().take(radius).sum();
    for i in 0..len {
        if i + radius < len {
            sum += source[i + radius];
        }
        values[i] = (sum + size / 2) / size;
        if i >= radius {
            sum -= source[i - radius];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{blur, box_radii};

    #[test]
    fn no_blur_means_no_radii_and_an_untouched_mask() {
        assert!(box_radii(0.0).is_empty());
        let mut mask = vec![0, 255, 0, 7];
        blur(&mut mask, 2, 2, &box_radii(0.0));
        assert_eq!(mask, [0, 255, 0, 7]);
    }

    #[test]
    fn radii_cover_about_three_deviations() {
        for sigma in [0.5_f32, 1.0, 3.0, 10.0, 42.0] {
            let total = box_radii(sigma).iter().sum::<usize>() as f32;
            assert!((total - 3.0 * sigma).abs() <= 3.0, "sigma {sigma}: {total}");
        }
    }

    #[test]
    fn a_point_spreads_symmetrically_and_keeps_its_mass() {
        let (w, h) = (41, 41);
        let mut mask = vec![0u8; w * h];
        mask[20 * w + 20] = 255;
        blur(&mut mask, w, h, &box_radii(2.0));
        let at = |x: usize, y: usize| i32::from(mask[y * w + x]);
        for d in 1..7 {
            assert_eq!(at(20 + d, 20), at(20 - d, 20));
            assert_eq!(at(20, 20 + d), at(20, 20 - d));
            assert_eq!(at(20 + d, 20), at(20, 20 + d));
        }
        assert!(at(20, 20) > at(22, 20) && at(22, 20) > at(25, 20));
        let mass: u32 = mask.iter().map(|&v| u32::from(v)).sum();
        assert!((mass as f32 - 255.0).abs() < 255.0 * 0.15, "mass {mass}");
    }

    #[test]
    fn a_uniform_mask_keeps_its_interior_value() {
        let (w, h) = (60, 60);
        let mut mask = vec![200u8; w * h];
        blur(&mut mask, w, h, &box_radii(1.5));
        assert_eq!(mask[30 * w + 30], 200);
        assert!(mask[0] < 200, "edges fade into the zero border");
    }
}
