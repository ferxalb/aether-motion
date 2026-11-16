//! Gaussian blur with continuously variable strength, for animated `filter: blur()`.
//!
//! The static blur rounds to a whole box radius, so an animated radius would change in visible
//! steps (only a few levels below 2 px). This blur uses three passes of an extended box filter
//! (Gwosdek et al., "Theoretical foundations of Gaussian convolution by extended box filtering",
//! 2011): a box of whole radius `r` plus a fractional weight `alpha` on the two next samples, with
//! `r` and `alpha` chosen so the three passes have exactly the variance `sigma²`. Edges are clamped
//! like the static blur. Every operation is a fixed sequence of float operations, so the result is
//! deterministic.

/// One extended box pass: whole radius and fractional end weight (`0 <= alpha < 1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::render) struct ExactBox {
    radius: usize,
    alpha: f32,
}

const PASSES: f32 = 3.0;

impl ExactBox {
    /// The pass for a gaussian of standard deviation `sigma` px; `None` when it is (nearly) zero.
    pub(in crate::render) fn new(sigma: f32) -> Option<Self> {
        if !sigma.is_finite() || sigma < 1e-3 {
            return None;
        }
        let variance = sigma * sigma / PASSES;
        let radius = ((0.5 * (12.0 * variance + 1.0).sqrt() - 0.5).floor()).max(0.0);
        let r = radius;
        let alpha = (2.0 * r + 1.0) * (r * (r + 1.0) - 3.0 * variance)
            / (6.0 * (variance - (r + 1.0) * (r + 1.0)));
        Some(Self {
            radius: radius as usize,
            alpha: alpha.clamp(0.0, 1.0),
        })
    }

    /// Pixels the three passes spread past a shape.
    pub(in crate::render) fn reach(self) -> usize {
        3 * (self.radius + 1)
    }

    fn total(self) -> f32 {
        (2 * self.radius + 1) as f32 + 2.0 * self.alpha
    }
}

/// Clamped index `i + offset` into `0..len`.
#[inline]
fn at(i: usize, offset: isize, len: usize) -> usize {
    (i as isize + offset).clamp(0, len as isize - 1) as usize
}

/// One pass along a line, from `src` into `dst`.
fn line(src: &[f32], dst: &mut [f32], pass: ExactBox) {
    let n = src.len();
    let r = pass.radius as isize;
    let scale = 1.0 / pass.total();
    let mut sum: f32 = (-r..=r).map(|k| src[at(0, k, n)]).sum();
    for i in 0..n {
        let ends = src[at(i, -r - 1, n)] + src[at(i, r + 1, n)];
        dst[i] = (sum + pass.alpha * ends) * scale;
        sum += src[at(i, r + 1, n)] - src[at(i, -r, n)];
    }
}

/// Three horizontal then three vertical passes over a `width` x `height` plane, in place.
pub(in crate::render) fn blur_plane_exact(
    plane: &mut [f32],
    width: usize,
    height: usize,
    pass: ExactBox,
) {
    if width == 0 || height == 0 {
        return;
    }
    let mut tmp = vec![0.0_f32; width.max(height)];
    for row in plane.chunks_exact_mut(width) {
        for _ in 0..3 {
            tmp[..width].copy_from_slice(row);
            line(&tmp[..width], row, pass);
        }
    }
    // Vertical passes run over whole rows: a running sum of the rows in the window.
    let mut scratch = vec![0.0_f32; plane.len()];
    let mut sums = vec![0.0_f32; width];
    let r = pass.radius as isize;
    let scale = 1.0 / pass.total();
    for step in 0..3 {
        let (src, dst): (&[f32], &mut [f32]) = if step % 2 == 0 {
            (&*plane, &mut scratch)
        } else {
            (&*scratch, &mut *plane)
        };
        let row = |y: usize| &src[y * width..(y + 1) * width];
        sums.fill(0.0);
        for k in -r..=r {
            for (sum, &v) in sums.iter_mut().zip(row(at(0, k, height))) {
                *sum += v;
            }
        }
        for y in 0..height {
            let (before, after) = (row(at(y, -r - 1, height)), row(at(y, r + 1, height)));
            let leaving = row(at(y, -r, height));
            let out = &mut dst[y * width..(y + 1) * width];
            for x in 0..width {
                out[x] = (sums[x] + pass.alpha * (before[x] + after[x])) * scale;
                sums[x] += after[x] - leaving[x];
            }
        }
        if step == 2 {
            // Three passes end in the scratch buffer.
            plane.copy_from_slice(&scratch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Variance of a blurred impulse in the middle of a long line.
    fn impulse_variance(sigma: f32) -> f32 {
        let n = 401;
        let mut plane = vec![0.0_f32; n];
        plane[n / 2] = 1.0;
        blur_plane_exact(&mut plane, n, 1, ExactBox::new(sigma).unwrap());
        // Running sums leave residues near 1e-9 far from the impulse; weighted by the squared
        // distance they would dominate, so measure within the kernel's reach only.
        let center = n / 2;
        let reach = ExactBox::new(sigma).unwrap().reach() + 1;
        let window = &plane[center - reach..=center + reach];
        let mass: f64 = window.iter().map(|&w| f64::from(w)).sum();
        let moment: f64 = window
            .iter()
            .enumerate()
            .map(|(i, &w)| f64::from(w) * (i as f64 - reach as f64).powi(2))
            .sum();
        (moment / mass) as f32
    }

    #[test]
    fn variance_matches_sigma_continuously() {
        for sigma in [0.3_f32, 0.8, 1.0, 1.7, 2.5, 6.0, 13.3, 40.0] {
            let variance = impulse_variance(sigma);
            let error = (variance - sigma * sigma).abs() / (sigma * sigma);
            assert!(error < 1e-3, "sigma {sigma}: variance {variance}");
        }
    }

    #[test]
    fn mass_is_kept_and_edges_clamp() {
        let mut plane: Vec<f32> = (0..64).map(|i| ((i * 37) % 11) as f32).collect();
        let total: f32 = plane.iter().sum();
        let flat = vec![7.0_f32; 48];
        let mut flat_out = flat.clone();
        blur_plane_exact(&mut flat_out, 8, 6, ExactBox::new(3.0).unwrap());
        for v in flat_out {
            assert!((v - 7.0).abs() < 1e-4, "a flat plane stays flat: {v}");
        }
        blur_plane_exact(&mut plane, 64, 1, ExactBox::new(1.5).unwrap());
        assert!(plane.iter().all(|v| v.is_finite()));
        // Clamped edges are not mass-preserving exactly, but stay close for a wide line.
        assert!((plane.iter().sum::<f32>() - total).abs() / total < 0.05);
    }

    #[test]
    fn strength_grows_without_steps() {
        let mut previous = 0.0;
        for i in 1..=40 {
            let variance = impulse_variance(i as f32 * 0.1);
            assert!(
                variance > previous,
                "variance must increase at sigma {}",
                i as f32 * 0.1
            );
            previous = variance;
        }
        assert!(ExactBox::new(0.0).is_none());
    }
}
