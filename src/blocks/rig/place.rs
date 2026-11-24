//! Ground placement shared by the rigs: per-sample vertical offsets that keep the lowest sole point
//! on the ground, and slide-free travel for gaits (a stride speed plus a periodic hip offset that
//! holds the planted sole point still).
//!
//! Both work from the rig's own forward kinematics (passed in as a function from a pose to the
//! figure's sole contact points), so what is computed is exactly what is drawn.

use super::options::Options;
use super::skeleton::{Pose, BODY_X, BODY_Y};

/// One sole contact point of a posed figure, in figure units with the hips at rest height: `foot`
/// identifies the point, `forward` is its coordinate along the facing direction, and `down` is its
/// height measured downward (0 on the ground at rest, positive below it).
pub struct Contact {
    pub foot: usize,
    pub forward: f64,
    pub down: f64,
}

/// Contact points of a pose; `body-x` and `body-y` are ignored by the caller's kinematics.
pub type Kinematics<'a> = &'a dyn Fn(&Pose) -> Vec<Contact>;

/// The pose at `t` with `body-y` raised or lowered so the lowest contact touches the ground (when
/// the ground is locked; the motion's own `body-y`, a jump's flight, is added on top), and with the
/// periodic part of a slide-free `stride` added to `body-x`.
pub fn placed(options: &Options, t: f64, fk: Kinematics, stride: Option<&Stride>) -> Pose {
    let mut pose = options.pose(t);
    if let Some(stride) = stride {
        pose[BODY_X] += stride.offset(t);
    }
    if options.ground_lock {
        let mut bare = pose;
        bare[BODY_X] = 0.0;
        bare[BODY_Y] = 0.0;
        let lowest = fk(&bare)
            .iter()
            .map(|c| c.down)
            .fold(f64::NEG_INFINITY, f64::max);
        if lowest.is_finite() {
            pose[BODY_Y] -= lowest;
        }
    }
    pose
}

/// Slide-free travel driven by the feet. For a looping motion it is a constant forward speed plus a
/// periodic forward offset of the hips; for a motion played once it is the hips' whole forward
/// path (and no speed). Either way the planted sole point stays where it touched the ground.
pub struct Stride {
    /// Figure units per second (0 for a motion played once).
    pub speed: f64,
    period: f64,
    periodic: bool,
    /// The hip offset at `table.len()` evenly spaced times over one period.
    table: Vec<f64>,
}

impl Stride {
    /// The forward offset of the hips at `t` seconds (beyond the constant speed).
    pub fn offset(&self, t: f64) -> f64 {
        let steps = self.table.len() - 1;
        let u = if self.periodic {
            (t / self.period).rem_euclid(1.0)
        } else {
            (t / self.period).clamp(0.0, 1.0)
        } * steps as f64;
        let i = (u.floor() as usize).min(steps - 1);
        let s = u - i as f64;
        self.table[i] + (self.table[i + 1] - self.table[i]) * s
    }
}

/// Follows the lowest sole point through `period` seconds: while one point stays lowest it is held
/// still on the ground, and when another point takes over the hold passes to it without a jump.
/// A `periodic` motion advances by one stride per period, split into a constant speed and a
/// periodic part.
pub fn stride(options: &Options, period: f64, fk: Kinematics, periodic: bool) -> Stride {
    // 240 steps per second resolves every gait (the shortest preset cycle has 170 of them);
    // periods are at most 600 s, so this stays under 144,000 kinematic evaluations.
    let steps = ((period * 240.0).ceil() as usize).clamp(720, 144_000);
    // The planted point keeps its hold while it stays within `GRIP` of the lowest point, so two
    // feet near the ground (a stop, a double support) do not trade the hold back and forth.
    const GRIP: f64 = 4.0;
    let contacts = |i: usize| -> Vec<Contact> {
        let mut pose = options.pose(period * i as f64 / steps as f64);
        pose[BODY_X] = 0.0;
        pose[BODY_Y] = 0.0;
        fk(&pose)
    };
    let pick = |points: &[Contact], held: Option<usize>| -> (usize, f64) {
        let lowest = points
            .iter()
            .max_by(|a, b| a.down.total_cmp(&b.down).then(b.foot.cmp(&a.foot)));
        let Some(lowest) = lowest else {
            return (usize::MAX, 0.0);
        };
        held.and_then(|h| points.iter().find(|c| c.foot == h))
            .filter(|c| c.down >= lowest.down - GRIP)
            .map_or((lowest.foot, lowest.forward), |c| (c.foot, c.forward))
    };
    // hips(t) = anchor - forward of the planted point; the anchor moves only at a hand-over.
    let (mut foot, start) = pick(&contacts(0), None);
    let mut anchor = start;
    let mut hips = vec![0.0; steps + 1];
    for (i, slot) in hips.iter_mut().enumerate().skip(1) {
        let points = contacts(i);
        let (f, x) = pick(&points, Some(foot));
        if f != foot {
            // Keep the hips continuous: the new point is held where it is now.
            let previous = points
                .iter()
                .find(|c| c.foot == foot)
                .map_or(x, |c| c.forward);
            anchor += x - previous;
            foot = f;
        }
        *slot = anchor - x;
    }
    let advance = if periodic { hips[steps] - hips[0] } else { 0.0 };
    let table = hips
        .iter()
        .enumerate()
        .map(|(i, h)| h - hips[0] - advance * i as f64 / steps as f64)
        .collect();
    Stride {
        speed: advance / period,
        period,
        periodic,
        table,
    }
}
