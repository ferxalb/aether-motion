//! Distance, error-function, and gaussian edge helpers shared by border and shadow painting.

/// Signed distance from `(x, y)` to a rounded rectangle; negative inside.
pub(in crate::render) fn rounded_distance(
    x: f32,
    y: f32,
    [x0, y0, x1, y1]: [f32; 4],
    radius: f32,
) -> f32 {
    let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let radius = radius.clamp(0.0, hx.min(hy).max(0.0));
    let qx = (x - (x0 + hx)).abs() - (hx - radius);
    let qy = (y - (y0 + hy)).abs() - (hy - radius);
    // Only the corner arcs need a square root.
    let outside = if qx > 0.0 && qy > 0.0 {
        qx.hypot(qy)
    } else {
        qx.max(qy)
    };
    outside - radius
}

/// Complementary error function (Abramowitz & Stegun 7.1.26, error below 1.5e-7).
pub(in crate::render) fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_72 + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let tail = poly * (-z * z).exp();
    if x >= 0.0 {
        tail
    } else {
        2.0 - tail
    }
}

/// Gaussian-blurred edge profile: coverage as a function of the signed distance to a sharp edge.
///
/// Sampled every 1/16 px; beyond +-3 sigma the profile is flat (0 outside, 1 inside).
pub(in crate::render) struct GaussianEdge {
    /// Distance in px beyond which the profile is flat (3 sigma).
    pub tail: f32,
    table: Vec<f32>,
}

impl GaussianEdge {
    const STEPS_PER_PX: f32 = 16.0;

    /// Builds the profile for a blur of standard deviation `sigma` (px); tiny sigmas give a hard edge.
    pub fn new(sigma: f32) -> Self {
        let tail = sigma * 3.0;
        let table = if sigma < 0.01 {
            Vec::new()
        } else {
            let n = (tail * 2.0 * Self::STEPS_PER_PX).ceil() as usize + 1;
            (0..=n)
                .map(|i| {
                    let d = -tail + i as f32 / Self::STEPS_PER_PX;
                    0.5 * erfc(d / (sigma * std::f32::consts::SQRT_2))
                })
                .collect()
        };
        Self { tail, table }
    }

    /// Coverage in 0..=1 at signed `distance` from the edge (negative inside the shape).
    pub fn coverage(&self, distance: f32) -> f32 {
        if self.table.is_empty() {
            (0.5 - distance).clamp(0.0, 1.0)
        } else if distance >= self.tail {
            0.0
        } else if distance <= -self.tail {
            1.0
        } else {
            let position = (distance + self.tail) * Self::STEPS_PER_PX;
            let i = (position as usize).min(self.table.len() - 2);
            let fraction = position - i as f32;
            self.table[i] + (self.table[i + 1] - self.table[i]) * fraction
        }
    }
}
