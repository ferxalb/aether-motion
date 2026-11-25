//! `data-hf-shader` painting: places the evaluation grid over the visible part of the element's
//! padding box (one point per output pixel, or one per 2 or 4 pixels at reduced resolution,
//! anchored to the box so the pattern never swims), evaluates it through `crate::shader`
//! (AVX2 or baseline build, identical values), and composites it over the surface: bilinear
//! interpolation of premultiplied points at reduced resolution, fractional edge coverage, the
//! element's opacity, and an optional 4x4 ordered dither of under half a level.
//!
//! Each element keeps its last grid with the frame values and placement that produced it, and
//! reuses it when both match (a still shader under a still or panning-free frame costs only the
//! blend). The kept grids share a 64 MiB budget; one that does not fit is dropped after the
//! draw. A kept grid holds exactly what a fresh evaluation would, so pixels never depend on it.

use super::kernels::{lerp_rows, shader_blend_row};
use crate::render::geometry::{PixelRect, Rect};
use crate::render::Renderer;
use crate::shader::{Grid, Shader};
use anyhow::Result;

/// Budget of the grids kept between frames, in bytes.
const KEPT_BYTES: usize = 64 << 20;

/// 4x4 Bayer thresholds.
const BAYER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// The last grid evaluated for one element, with what produced it.
#[derive(Default)]
pub(in crate::render) struct KeptGrid {
    values: Vec<u32>,
    grid: Option<Grid>,
    points: Vec<f32>,
}

impl KeptGrid {
    fn bytes(&self) -> usize {
        self.points.capacity() * 4
    }
}

/// Per-renderer shader state: scratch registers and the kept grids.
#[derive(Default)]
pub(in crate::render) struct ShaderState {
    scratch: crate::shader::Scratch,
    kept: std::collections::HashMap<usize, KeptGrid>,
    kept_bytes: usize,
}

/// Overlap of the pixel `[p, p + 1)` with `[start, end)`.
#[inline(always)]
fn coverage(p: usize, start: f32, end: f32) -> f32 {
    let p = p as f32;
    #[allow(clippy::manual_clamp)]
    let c = ((p + 1.0).min(end) - p.max(start)).max(0.0).min(1.0);
    c
}

impl<'a> Renderer<'a> {
    /// Draws the shader of element `idx` into `rect` (its padding box, surface px / `self.scale`).
    pub(in crate::render) fn draw_shader(
        &mut self,
        idx: usize,
        shader: &Shader,
        rect: Rect,
        cumulative: f32,
        alpha: f32,
    ) -> Result<()> {
        if alpha <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 || cumulative <= 0.0 {
            return Ok(());
        }
        let k = self.scale;
        let (ox, oy, w, h) = (rect.x * k, rect.y * k, rect.w * k, rect.h * k);
        let (sw, sh) = (self.surface.width as f32, self.surface.height as f32);
        let x0 = ox.floor().max(0.0).min(sw) as usize;
        let x1 = (ox + w).ceil().max(0.0).min(sw) as usize;
        let y0 = oy.floor().max(0.0).min(sh) as usize;
        let y1 = (oy + h).ceil().max(0.0).min(sh) as usize;
        if x0 >= x1 || y0 >= y1 || !w.is_finite() || !h.is_finite() {
            return Ok(());
        }
        let elapsed = self.time - self.composition.elements[idx].start;
        let size = [rect.w / cumulative, rect.h / cumulative];
        let values = shader.values(elapsed, size);
        // Grid points every `cell` surface px from the box corner; at full resolution they sit on
        // the pixel centers of the visible area.
        let cell = 1.0 / shader.resolution;
        let (first, grid) = if shader.resolution >= 1.0 {
            (
                [x0 as i64, y0 as i64],
                Grid {
                    width: x1 - x0,
                    height: y1 - y0,
                    origin: [(x0 as f32 + 0.5 - ox) / w, (y0 as f32 + 0.5 - oy) / h],
                    step: [1.0 / w, 1.0 / h],
                    size,
                },
            )
        } else {
            let index = |p: f32, o: f32| ((p - o) / cell - 0.5).floor() as i64;
            let (i0, i1) = (index(x0 as f32 + 0.5, ox), index(x1 as f32 - 0.5, ox) + 1);
            let (j0, j1) = (index(y0 as f32 + 0.5, oy), index(y1 as f32 - 0.5, oy) + 1);
            (
                [i0, j0],
                Grid {
                    width: (i1 - i0 + 1) as usize,
                    height: (j1 - j0 + 1) as usize,
                    origin: [(i0 as f32 + 0.5) * cell / w, (j0 as f32 + 0.5) * cell / h],
                    step: [cell / w, cell / h],
                    size,
                },
            )
        };
        crate::render::allocation::rgba_bytes(grid.width, grid.height * 4, "shader grid")?;
        let value_bits: Vec<u32> = values.iter().map(|v| v.to_bits()).collect();
        let mut kept = self.shaders.kept.remove(&idx).unwrap_or_default();
        self.shaders.kept_bytes -= kept.bytes().min(self.shaders.kept_bytes);
        if kept.grid != Some(grid) || kept.values != value_bits {
            let avx2 = super::kernels::simd_enabled();
            let scratch = &mut self.shaders.scratch;
            crate::profile::time(crate::profile::Phase::ShaderEval, || {
                shader.run(&values, &grid, &mut kept.points, scratch, avx2)
            });
            kept.grid = Some(grid);
            kept.values = value_bits;
        }
        let dither = shader.dither;
        let surface = &mut self.surface;
        let stride = surface.width as usize * 4;
        let points: &[[f32; 4]] = kept.points.as_chunks::<4>().0;
        crate::profile::time(crate::profile::Phase::ShaderBlend, || {
            let width = x1 - x0;
            let columns: Vec<f32> = (x0..x1).map(|x| coverage(x, ox, ox + w)).collect();
            let mut cover = vec![0.0_f32; width];
            let mut sampler = Sampler::new(points, &grid, first, ox, cell, x0, width);
            for y in y0..y1 {
                let cy = coverage(y, oy, oy + h) * alpha;
                if cy <= 0.0 {
                    continue;
                }
                for (c, &cx) in cover.iter_mut().zip(&columns) {
                    *c = cx * cy;
                }
                let offsets: [f32; 4] = if dither {
                    std::array::from_fn(|i| (f32::from(BAYER[(y & 3) * 4 + i]) + 0.5) / 16.0 - 0.5)
                } else {
                    [0.0; 4]
                };
                let row = sampler.row(y, oy);
                let pixels = &mut surface.pixels[y * stride + x0 * 4..y * stride + x1 * 4];
                shader_blend_row(pixels, row, &cover, &offsets, x0 & 3);
            }
        });
        self.surface.mark_painted(PixelRect { x0, y0, x1, y1 });
        let bytes = kept.bytes();
        if self.shaders.kept_bytes + bytes <= KEPT_BYTES {
            self.shaders.kept_bytes += bytes;
            self.shaders.kept.insert(idx, kept);
        }
        Ok(())
    }
}

/// Resamples the grid to surface rows. At full resolution a row is a slice of the grid. At
/// reduced resolution each grid row is interpolated horizontally to the output columns once and
/// kept (the two rows bracketing the current output row), and output rows interpolate vertically
/// between them: `top + (bottom - top) * wy` of `a + (b - a) * wx`, the bilinear blend.
struct Sampler<'p> {
    points: &'p [[f32; 4]],
    grid: Grid,
    first: [i64; 2],
    cell: f32,
    /// Per output column: the grid column at or before it, the one after, and the weight of the
    /// one after (reduced resolution only).
    columns: Vec<(usize, usize, f32)>,
    /// Horizontally interpolated grid rows, tagged with their grid row.
    rows: [(usize, Vec<[f32; 4]>); 2],
    out: Vec<[f32; 4]>,
}

/// Position of pixel `p` in grid units relative to point `base` (points every `cell` px from
/// `origin`, `count` of them): the point at or before it, the one after, and the weight of the
/// one after, clamped at the ends.
fn locate(p: usize, origin: f32, cell: f32, base: i64, count: usize) -> (usize, usize, f32) {
    let f = (p as f32 + 0.5 - origin) / cell - 0.5 - base as f32;
    let lower = f.floor();
    #[allow(clippy::manual_clamp)]
    let weight = (f - lower).max(0.0).min(1.0);
    let last = count.saturating_sub(1) as f32;
    #[allow(clippy::manual_clamp)]
    let i = lower.max(0.0).min(last) as usize;
    let next = (i + 1).min(count - 1);
    (i, next, if lower < 0.0 { 0.0 } else { weight })
}

impl<'p> Sampler<'p> {
    fn new(
        points: &'p [[f32; 4]],
        grid: &Grid,
        first: [i64; 2],
        origin_x: f32,
        cell: f32,
        x0: usize,
        width: usize,
    ) -> Self {
        let columns = if cell > 1.0 {
            (x0..x0 + width)
                .map(|x| locate(x, origin_x, cell, first[0], grid.width))
                .collect()
        } else {
            Vec::new()
        };
        Self {
            points,
            grid: *grid,
            first,
            cell,
            columns,
            rows: [
                (usize::MAX, vec![[0.0; 4]; width]),
                (usize::MAX, vec![[0.0; 4]; width]),
            ],
            out: vec![[0.0; 4]; width],
        }
    }

    /// Grid row `j` interpolated to the output columns, in one of the two kept slots (never
    /// the slot of `keep`).
    fn horizontal(&mut self, j: usize, keep: usize) -> usize {
        if let Some(slot) = (0..2).find(|&s| self.rows[s].0 == j) {
            return slot;
        }
        let slot = if self.rows[0].0 == keep { 1 } else { 0 };
        let line = &self.points[j * self.grid.width..(j + 1) * self.grid.width];
        let (tag, out) = &mut self.rows[slot];
        for (o, &(i, next, wx)) in out.iter_mut().zip(&self.columns) {
            let (a, b) = (line[i], line[next]);
            for ch in 0..4 {
                o[ch] = a[ch] + (b[ch] - a[ch]) * wx;
            }
        }
        *tag = j;
        slot
    }

    /// The premultiplied colors of surface row `y`, starting at pixel `x0`.
    fn row(&mut self, y: usize, origin_y: f32) -> &[[f32; 4]] {
        let width = self.out.len();
        if self.cell <= 1.0 {
            // `first` is the pixel of the first point.
            let j = ((y as i64 - self.first[1]).max(0) as usize).min(self.grid.height - 1);
            let start = j * self.grid.width;
            return &self.points[start..start + width.min(self.grid.width)];
        }
        let (j, next, wy) = locate(y, origin_y, self.cell, self.first[1], self.grid.height);
        let top = self.horizontal(j, usize::MAX);
        let bottom = self.horizontal(next, j);
        lerp_rows(&mut self.out, &self.rows[top].1, &self.rows[bottom].1, wy);
        &self.out
    }
}
