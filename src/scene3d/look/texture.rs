//! Decoded image textures with a mip chain and trilinear sampling.
//!
//! Pixels are kept as 8-bit RGBA. Color textures are sRGB and are filtered in linear light; data
//! textures (normal maps) are linear and are filtered as they are. Mip levels are box-filtered at
//! load, so the same file always yields the same levels, and sampling is plain `f64` arithmetic
//! on those levels.

use super::Wrap;
use crate::scene3d::shade::{to_linear, to_srgb};
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::sync::OnceLock;

/// Largest texture edge (px).
pub const MAX_EDGE: u32 = 4096;

/// How the stored bytes map to values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Color: bytes are sRGB and filter in linear light.
    Srgb,
    /// Data: bytes are `value x 255` and filter as they are.
    Linear,
}

#[derive(Debug)]
struct Level {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 4]>,
}

/// An image with its mip chain.
#[derive(Debug)]
pub struct Texture {
    levels: Vec<Level>,
    encoding: Encoding,
    /// Hash of the base level, so equal textures compare equal without comparing pixels.
    id: u64,
    has_alpha: bool,
    /// The file it was decoded from, for dependency tracking.
    path: Option<std::path::PathBuf>,
}

impl PartialEq for Texture {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.encoding == other.encoding
            && self.levels[0].width == other.levels[0].width
            && self.levels[0].height == other.levels[0].height
    }
}

fn srgb_table() -> &'static [f64; 256] {
    static TABLE: OnceLock<[f64; 256]> = OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|i| to_linear(i as u8)))
}

impl Texture {
    /// Decodes the image at `path` (PNG, JPEG, or WebP) within the renderer's image limits.
    pub fn load(path: &Path, encoding: Encoding) -> Result<Self> {
        crate::render::check_image(path)?;
        let mut reader = image::ImageReader::open(path)
            .and_then(|reader| reader.with_guessed_format())
            .with_context(|| format!("could not read texture {}", path.display()))?;
        reader.limits(crate::render::image_limits());
        let decoded = reader
            .decode()
            .with_context(|| format!("could not decode texture {}", path.display()))?
            .into_rgba8();
        let (width, height) = decoded.dimensions();
        if width > MAX_EDGE || height > MAX_EDGE {
            bail!(
                "texture {} is {width}x{height}; the longest edge is at most {MAX_EDGE} px",
                path.display()
            );
        }
        let pixels: Vec<[u8; 4]> = decoded.pixels().map(|p| p.0).collect();
        let mut texture = Self::from_pixels(width as usize, height as usize, pixels, encoding);
        texture.path = Some(path.to_path_buf());
        Ok(texture)
    }

    /// A texture from `width x height` RGBA pixels (row-major, top row first).
    pub fn from_pixels(
        width: usize,
        height: usize,
        pixels: Vec<[u8; 4]>,
        encoding: Encoding,
    ) -> Self {
        assert_eq!(
            pixels.len(),
            width * height,
            "texture size and pixels differ"
        );
        let id = pixels.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
            p.iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        });
        let has_alpha = pixels.iter().any(|p| p[3] != 255);
        let mut levels = vec![Level {
            width,
            height,
            pixels,
        }];
        while levels.last().is_some_and(|l| l.width > 1 || l.height > 1) {
            let next = shrink(levels.last().expect("a level exists"), encoding);
            levels.push(next);
        }
        Self {
            levels,
            encoding,
            id,
            has_alpha,
            path: None,
        }
    }

    /// The file this texture was read from, if any.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn width(&self) -> usize {
        self.levels[0].width
    }

    pub fn height(&self) -> usize {
        self.levels[0].height
    }

    pub fn texels(&self) -> u64 {
        (self.width() * self.height()) as u64
    }

    pub fn has_alpha(&self) -> bool {
        self.has_alpha
    }

    /// Number of mip levels.
    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    /// Bytes held by every level.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.pixels.len() * 4).sum()
    }

    /// The filtered value at `uv` (x right, y down in the image; `uv = (0, 0)` is the top-left
    /// corner) for a footprint of `lod` mip levels (0 is the base): linear RGB for color
    /// textures, raw `[0, 1]` values for data textures, and alpha in `[0, 1]`.
    pub fn sample(&self, uv: [f64; 2], lod: f64, wrap: Wrap) -> [f64; 4] {
        let top = (self.levels.len() - 1) as f64;
        let lod = if lod.is_finite() {
            lod.clamp(0.0, top)
        } else {
            0.0
        };
        let low = lod.floor();
        let fraction = lod - low;
        let a = self.bilinear(low as usize, uv, wrap);
        if fraction <= 0.0 || low >= top {
            return a;
        }
        let b = self.bilinear(low as usize + 1, uv, wrap);
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * fraction)
    }

    fn bilinear(&self, level: usize, uv: [f64; 2], wrap: Wrap) -> [f64; 4] {
        let level = &self.levels[level];
        let x = uv[0] * level.width as f64 - 0.5;
        let y = uv[1] * level.height as f64 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let fetch = |dx: i64, dy: i64| self.texel(level, x0 as i64 + dx, y0 as i64 + dy, wrap);
        let (p00, p10, p01, p11) = (fetch(0, 0), fetch(1, 0), fetch(0, 1), fetch(1, 1));
        std::array::from_fn(|i| {
            let top = p00[i] + (p10[i] - p00[i]) * fx;
            let bottom = p01[i] + (p11[i] - p01[i]) * fx;
            top + (bottom - top) * fy
        })
    }

    fn texel(&self, level: &Level, x: i64, y: i64, wrap: Wrap) -> [f64; 4] {
        let (w, h) = (level.width as i64, level.height as i64);
        let (x, y) = match wrap {
            Wrap::Repeat => (x.rem_euclid(w), y.rem_euclid(h)),
            Wrap::Clamp => (x.clamp(0, w - 1), y.clamp(0, h - 1)),
        };
        let p = level.pixels[(y * w + x) as usize];
        let alpha = f64::from(p[3]) / 255.0;
        match self.encoding {
            Encoding::Srgb => {
                let table = srgb_table();
                [
                    table[p[0] as usize],
                    table[p[1] as usize],
                    table[p[2] as usize],
                    alpha,
                ]
            }
            Encoding::Linear => [
                f64::from(p[0]) / 255.0,
                f64::from(p[1]) / 255.0,
                f64::from(p[2]) / 255.0,
                alpha,
            ],
        }
    }
}

/// The next mip level: each pixel is the mean of a 2x2 block (edges clamp for odd sizes).
fn shrink(source: &Level, encoding: Encoding) -> Level {
    let (width, height) = ((source.width / 2).max(1), (source.height / 2).max(1));
    let table = srgb_table();
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0_f64; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (x * 2 + dx).min(source.width - 1);
                let sy = (y * 2 + dy).min(source.height - 1);
                let p = source.pixels[sy * source.width + sx];
                for c in 0..3 {
                    sum[c] += match encoding {
                        Encoding::Srgb => table[p[c] as usize],
                        Encoding::Linear => f64::from(p[c]) / 255.0,
                    };
                }
                sum[3] += f64::from(p[3]) / 255.0;
            }
            let mean = sum.map(|v| v / 4.0);
            let encode = |v: f64| match encoding {
                Encoding::Srgb => to_srgb(v),
                Encoding::Linear => (v * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8,
            };
            pixels.push([
                encode(mean[0]),
                encode(mean[1]),
                encode(mean[2]),
                (mean[3] * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8,
            ]);
        }
    }
    Level {
        width,
        height,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker() -> Texture {
        let pixels = vec![
            [255, 255, 255, 255],
            [0, 0, 0, 255],
            [0, 0, 0, 255],
            [255, 255, 255, 255],
        ];
        Texture::from_pixels(2, 2, pixels, Encoding::Srgb)
    }

    #[test]
    fn mip_chain_ends_at_one_texel_and_averages_in_linear_light() {
        let texture = checker();
        assert_eq!(texture.levels(), 2);
        let top = texture.sample([0.5, 0.5], 1.0, Wrap::Repeat);
        // Half white, half black in linear light is 0.5, not the sRGB midpoint.
        assert!((top[0] - 0.5).abs() < 0.01, "{top:?}");
    }

    #[test]
    fn texel_centers_return_the_stored_value_and_wrap_repeats() {
        let texture = checker();
        let white = texture.sample([0.25, 0.25], 0.0, Wrap::Repeat);
        assert!((white[0] - 1.0).abs() < 1e-9);
        let again = texture.sample([1.25, 2.25], 0.0, Wrap::Repeat);
        assert_eq!(white, again);
        let clamped = texture.sample([-3.0, 0.25], 0.0, Wrap::Clamp);
        assert!((clamped[0] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn equal_pixels_compare_equal() {
        assert_eq!(checker(), checker());
        let other = Texture::from_pixels(1, 1, vec![[1, 2, 3, 255]], Encoding::Srgb);
        assert_ne!(checker(), other);
    }
}
