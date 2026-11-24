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
    /// Sample times over one period (sorted, from 0 to the period) and the hip offset at each.
    times: Vec<f64>,
    table: Vec<f64>,
}

impl Stride {
    /// The forward offset of the hips at `t` seconds (beyond the constant speed).
    pub fn offset(&self, t: f64) -> f64 {
        let u = if self.periodic {
            (t / self.period).rem_euclid(1.0) * self.period
        } else {
            t.clamp(0.0, self.period)
        };
        let i = self
            .times
            .partition_point(|x| *x <= u)
            .clamp(1, self.times.len() - 1);
        let (t0, t1) = (self.times[i - 1], self.times[i]);
        let s = if t1 > t0 { (u - t0) / (t1 - t0) } else { 0.0 };
        self.table[i - 1] + (self.table[i] - self.table[i - 1]) * s
    }
}

/// Most kinematic evaluations one stride may use.
const STRIDE_BUDGET: usize = 200_000;
/// Largest change of any pose channel (degrees) between neighboring stride samples.
const STRIDE_TURN: f64 = 3.0;

/// Follows the lowest sole point through `period` seconds: while one point stays lowest it is held
/// still on the ground, and when another point takes over the hold passes to it without a jump.
/// A `periodic` motion advances by one stride per period, split into a constant speed and a
/// periodic part.
///
/// Samples are a 240 Hz grid (at least 720 per period) plus every authored time in `marks`
/// (timeline keys, sequence boundaries), refined wherever a joint turns more than 3 degrees
/// between neighbors, so a short gait inside a long sequence or a quick keyed move is followed.
/// A motion that would need more than 200,000 samples is an error, not a silently wrong path.
pub fn stride(
    options: &Options,
    period: f64,
    fk: Kinematics,
    periodic: bool,
    marks: &[f64],
) -> anyhow::Result<Stride> {
    let steps = ((period * 240.0).ceil() as usize).max(720);
    if steps > STRIDE_BUDGET {
        anyhow::bail!("travel=auto cannot follow a {period} s motion; keep it under 800 s");
    }
    let mut times: Vec<f64> = (0..=steps)
        .map(|i| period * i as f64 / steps as f64)
        .chain(marks.iter().map(|m| m.clamp(0.0, period)))
        .collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    let pose_at = |t: f64| -> Pose {
        let mut pose = options.pose(t);
        pose[BODY_X] = 0.0;
        pose[BODY_Y] = 0.0;
        pose
    };
    let mut poses: Vec<Pose> = times.iter().map(|t| pose_at(*t)).collect();
    let mut i = 0;
    while i + 1 < times.len() {
        let turn = poses[i]
            .iter()
            .zip(&poses[i + 1])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        if turn > STRIDE_TURN && times[i + 1] - times[i] > 1e-5 {
            if times.len() >= STRIDE_BUDGET {
                anyhow::bail!(
                    "travel=auto needs more than {STRIDE_BUDGET} samples to follow this motion; slow its fastest moves"
                );
            }
            let mid = (times[i] + times[i + 1]) / 2.0;
            times.insert(i + 1, mid);
            poses.insert(i + 1, pose_at(mid));
        } else {
            i += 1;
        }
    }
    // The planted point keeps its hold while it stays within `GRIP` of the lowest point, so two
    // feet near the ground (a stop, a double support) do not trade the hold back and forth.
    const GRIP: f64 = 4.0;
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
    let (mut foot, start) = pick(&fk(&poses[0]), None);
    let mut anchor = start;
    let mut hips = vec![0.0; times.len()];
    for (i, slot) in hips.iter_mut().enumerate().skip(1) {
        let points = fk(&poses[i]);
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
    let last = hips.len() - 1;
    let advance = if periodic { hips[last] - hips[0] } else { 0.0 };
    let table = hips
        .iter()
        .zip(&times)
        .map(|(h, t)| h - hips[0] - advance * t / period)
        .collect();
    Ok(Stride {
        speed: advance / period,
        period,
        periodic,
        times,
        table,
    })
}
