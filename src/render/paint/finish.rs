//! Frame-wide finishing applied after everything is drawn: vignette and film grain.
//!
//! Both are pure functions of pixel position (and, for grain, of the frame time), so output stays
//! deterministic and independent of render order.

use crate::render::geometry::PixelRect;
use crate::render::paint::Surface;

/// Integer hash of a pixel position and seed; every bit depends on all inputs.
fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

/// Brightness factor of the vignette at normalized distance `d` (0 center, 1 corner).
fn vignette_factor(strength: f32, d: f32) -> f32 {
    let t = ((d - 0.35) / 0.65).clamp(0.0, 1.0);
    1.0 - strength * t * t * (3.0 - 2.0 * t)
}

impl Surface {
    /// Darkens the corners by `vignette` (0..=1) and adds monochrome noise of amplitude `grain` (0..=1),
    /// reseeded by `seed` each frame. Marks the whole surface as painted so the next frame resets it.
    pub(in crate::render) fn apply_finish(&mut self, vignette: f32, grain: f32, seed: u32) {
        if vignette <= 0.0 && grain <= 0.0 {
            return;
        }
        let (width, height) = (self.width as usize, self.height as usize);
        for y in 0..height {
            let ny = (y as f32 + 0.5) / height as f32 * 2.0 - 1.0;
            for x in 0..width {
                let nx = (x as f32 + 0.5) / width as f32 * 2.0 - 1.0;
                let factor = if vignette > 0.0 {
                    vignette_factor(
                        vignette,
                        (nx * nx + ny * ny).sqrt() / std::f32::consts::SQRT_2,
                    )
                } else {
                    1.0
                };
                let noise = if grain > 0.0 {
                    let bits = (hash(x as u32, y as u32, seed) >> 8) & 0xFFFF;
                    (bits as f32 / 65535.0 * 2.0 - 1.0) * grain * 64.0
                } else {
                    0.0
                };
                let offset = (y * width + x) * 4;
                for c in 0..3 {
                    let value = f32::from(self.pixels[offset + c]) * factor + noise;
                    self.pixels[offset + c] = value.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        self.mark_painted(PixelRect {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vignette_only_darkens_towards_the_corners() {
        assert_eq!(vignette_factor(0.5, 0.0), 1.0);
        assert_eq!(vignette_factor(0.5, 0.35), 1.0);
        assert!((vignette_factor(0.5, 1.0) - 0.5).abs() < 1e-6);
        assert!(vignette_factor(0.5, 0.7) < 1.0);
    }

    #[test]
    fn noise_is_deterministic_and_varies_with_seed() {
        assert_eq!(hash(3, 4, 5), hash(3, 4, 5));
        assert_ne!(hash(3, 4, 5), hash(3, 4, 6));
        assert_ne!(hash(3, 4, 5), hash(4, 3, 5));
    }
}
