//! Entrance motion of `data-hf-split` pieces and line rows: start values, stagger order, and the
//! eased tracks that bring each piece (or line) to rest.
//!
//! A piece starts at its delay with the start values of `data-hf-split-from` (overridden channel by
//! channel by `data-hf-split-x`, `-y`, `-scale`, `-rotate`, and `-opacity`) and reaches its resting
//! pose `duration` seconds later. Opacity always eases with the quadratic ease-out, so springs and
//! `back` curves never push it past 1. Lines (rows) use the same model with their own options.

use crate::model::{Easing, Motion, Track, TrackPoint};

/// Start values of an entrance; `None` keeps the resting value (no track).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::parse) struct StartPose {
    pub(in crate::parse) x: Option<f32>,
    pub(in crate::parse) y: Option<f32>,
    pub(in crate::parse) scale: Option<f32>,
    pub(in crate::parse) rotate: Option<f32>,
    pub(in crate::parse) opacity: Option<f32>,
}

impl StartPose {
    /// `other`'s channels where it sets them, else this pose's.
    pub(in crate::parse) fn overridden_by(self, other: StartPose) -> StartPose {
        StartPose {
            x: other.x.or(self.x),
            y: other.y.or(self.y),
            scale: other.scale.or(self.scale),
            rotate: other.rotate.or(self.rotate),
            opacity: other.opacity.or(self.opacity),
        }
    }

    pub(in crate::parse) fn is_rest(self) -> bool {
        self == StartPose::default()
    }
}

/// Order in which pieces start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::parse) enum Order {
    /// First to last (the default).
    Forward,
    /// Last to first.
    Reverse,
    /// From the middle outwards.
    Center,
    /// From both ends inwards.
    Edges,
}

impl Order {
    pub(in crate::parse) fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "forward" => Self::Forward,
            "reverse" => Self::Reverse,
            "center" => Self::Center,
            "edges" => Self::Edges,
            _ => return None,
        })
    }

    /// Stagger steps before piece `index` of `count` starts (fractional for `center` and `edges`
    /// with an even count, so both middle pieces start together).
    pub(in crate::parse) fn steps(self, index: usize, count: usize) -> f64 {
        let last = count.saturating_sub(1) as f64;
        let i = index as f64;
        let middle = last / 2.0;
        match self {
            Self::Forward => i,
            Self::Reverse => last - i,
            Self::Center => (i - middle).abs().floor(),
            Self::Edges => (middle - (i - middle).abs()).ceil(),
        }
    }
}

/// One eased entrance: from `from` at `delay` seconds to rest at `delay + duration`.
#[derive(Clone, Copy, Debug)]
pub(in crate::parse) struct Entrance {
    pub(in crate::parse) from: StartPose,
    pub(in crate::parse) delay: f64,
    pub(in crate::parse) duration: f64,
    pub(in crate::parse) easing: Easing,
}

impl Entrance {
    /// Motion tracks that hold the start values before the entrance and the rest after it.
    pub(in crate::parse) fn motion(self) -> Motion {
        let end = self.delay + self.duration;
        let track = |from: f32, to: f32, easing: Easing| {
            Track(vec![
                TrackPoint {
                    at: self.delay,
                    value: from,
                    easing: Easing::Linear,
                },
                TrackPoint {
                    at: end,
                    value: to,
                    easing,
                },
            ])
        };
        let mut motion = Motion {
            hold_before: true,
            hold_after: true,
            ..Motion::default()
        };
        let from = self.from;
        if let Some(x) = from.x {
            motion.x = track(x, 0.0, self.easing);
        }
        if let Some(y) = from.y {
            motion.y = track(y, 0.0, self.easing);
        }
        if let Some(scale) = from.scale {
            motion.scale = track(scale, 1.0, self.easing);
        }
        if let Some(rotate) = from.rotate {
            motion.rotate = track(rotate, 0.0, self.easing);
        }
        // Opacity never overshoots, even with spring or `back` easings.
        if let Some(opacity) = from.opacity {
            motion.opacity = track(opacity, 1.0, Easing::EaseOut);
        }
        motion
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_spread_the_stagger() {
        let steps = |order: Order| (0..5).map(|i| order.steps(i, 5)).collect::<Vec<_>>();
        assert_eq!(steps(Order::Forward), [0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(steps(Order::Reverse), [4.0, 3.0, 2.0, 1.0, 0.0]);
        assert_eq!(steps(Order::Center), [2.0, 1.0, 0.0, 1.0, 2.0]);
        assert_eq!(steps(Order::Edges), [0.0, 1.0, 2.0, 1.0, 0.0]);
        // Both middle pieces of an even count start together.
        let even = (0..4)
            .map(|i| Order::Center.steps(i, 4))
            .collect::<Vec<_>>();
        assert_eq!(even, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn entrance_combines_position_scale_and_opacity() {
        let motion = Entrance {
            from: StartPose {
                y: Some(40.0),
                scale: Some(0.5),
                opacity: Some(0.0),
                ..StartPose::default()
            },
            delay: 0.2,
            duration: 0.5,
            easing: Easing::Linear,
        }
        .motion();
        let start = motion.pose(0.0);
        assert_eq!((start.y, start.scale, start.opacity), (40.0, 0.5, 0.0));
        let mid = motion.pose(0.45);
        assert!((mid.y - 20.0).abs() < 1e-4 && (mid.scale - 0.75).abs() < 1e-4);
        let end = motion.pose(0.7);
        assert_eq!((end.x, end.y, end.scale, end.opacity), (0.0, 0.0, 1.0, 1.0));
        assert!(motion.rotate.0.is_empty() && motion.x.0.is_empty());
    }
}
