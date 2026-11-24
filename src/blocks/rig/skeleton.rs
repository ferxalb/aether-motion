//! The human skeleton shared by the 2D and 3D rigs: pose channels, bone lengths, and joint radii.
//!
//! All lengths are in rig units for a figure 600 units tall (the rigs scale them to the requested
//! `size`). The origin is the hip center, y points down, and the soles rest on y = `GROUND`.
//! Angles are anatomical degrees, independent of view and facing: positive flexion swings a limb
//! forward (the knee bends back, the ankle lifts the toes), positive `-side` values raise a limb
//! away from the body, and positive torso and head `-side`/`-twist`/`-turn` values lean or turn
//! toward the figure's own right.

/// Pose channel names, in `Pose` index order. They double as block parameters (static offsets) and
/// as the keys of `timeline` keyframes.
pub const CHANNELS: [&str; 25] = [
    "body-x",
    "body-y",
    "body",
    "torso",
    "torso-side",
    "torso-twist",
    "head",
    "head-side",
    "head-turn",
    "r-shoulder",
    "r-shoulder-side",
    "r-elbow",
    "r-wrist",
    "l-shoulder",
    "l-shoulder-side",
    "l-elbow",
    "l-wrist",
    "r-hip",
    "r-hip-side",
    "r-knee",
    "r-ankle",
    "l-hip",
    "l-hip-side",
    "l-knee",
    "l-ankle",
];

/// One value per channel of `CHANNELS`: px for `body-x`/`body-y`, degrees for the rest.
pub type Pose = [f64; CHANNELS.len()];

pub const BODY_X: usize = 0;
pub const BODY_Y: usize = 1;
pub const BODY: usize = 2;
pub const TORSO: usize = 3;
pub const TORSO_SIDE: usize = 4;
pub const TORSO_TWIST: usize = 5;
pub const HEAD: usize = 6;
pub const HEAD_SIDE: usize = 7;
pub const HEAD_TURN: usize = 8;

/// The channels of one side of the body.
#[derive(Clone, Copy, Debug)]
pub struct Side {
    /// `r` or `l`, the prefix of the side's channel and node names.
    pub tag: &'static str,
    /// +1 for the figure's right, -1 for its left: the sign of x of its joints when the figure
    /// faces the viewer (its right side is on the screen's left).
    pub sign: f64,
    pub shoulder: usize,
    pub shoulder_side: usize,
    pub elbow: usize,
    pub wrist: usize,
    pub hip: usize,
    pub hip_side: usize,
    pub knee: usize,
    pub ankle: usize,
}

pub const RIGHT: Side = Side {
    tag: "r",
    sign: 1.0,
    shoulder: 9,
    shoulder_side: 10,
    elbow: 11,
    wrist: 12,
    hip: 17,
    hip_side: 18,
    knee: 19,
    ankle: 20,
};

pub const LEFT: Side = Side {
    tag: "l",
    sign: -1.0,
    shoulder: 13,
    shoulder_side: 14,
    elbow: 15,
    wrist: 16,
    hip: 21,
    hip_side: 22,
    knee: 23,
    ankle: 24,
};

/// Height the rig units describe; `size` scales by `size / STATURE`.
pub const STATURE: f64 = 600.0;
/// Hip center to the ground at rest (thigh + shin + ankle height).
pub const GROUND: f64 = THIGH + SHIN + ANKLE_HEIGHT;

pub const HIP_HALF: f64 = 30.0;
pub const THIGH: f64 = 138.0;
pub const SHIN: f64 = 134.0;
pub const ANKLE_HEIGHT: f64 = 22.0;

/// Waist pivot above the hip center (the torso turns around it).
pub const WAIST: f64 = 18.0;
/// Shoulder joints above the waist pivot, and their half spacing seen from the front.
pub const SHOULDER_RISE: f64 = 178.0;
pub const SHOULDER_HALF: f64 = 60.0;
/// The neck pivot above the waist pivot (the head turns around it).
pub const NECK_RISE: f64 = 188.0;

pub const UPPER_ARM: f64 = 116.0;
pub const FOREARM: f64 = 104.0;
pub const HAND: f64 = 40.0;

/// Joint radii: each limb segment is the tangent hull of the circles (spheres in 3D) at its two
/// joints, so consecutive segments meet on a shared circle at every angle.
pub const R_SHOULDER: f64 = 19.0;
pub const R_ELBOW: f64 = 15.0;
pub const R_WRIST: f64 = 11.0;
pub const R_FINGERS: f64 = 8.5;
pub const R_HIP: f64 = 26.0;
pub const R_KNEE: f64 = 19.5;
pub const R_ANKLE: f64 = 13.0;
pub const R_NECK: f64 = 13.0;

/// Sole contact points relative to the ankle joint (side view, facing +x): the ends of the flat
/// part of the drawn sole, at heel and toe.
pub const HEEL: f64 = -12.0;
pub const TOE: f64 = 50.0;
/// Half width of the 3D sole.
pub const SOLE_HALF: f64 = 15.0;
/// Half width of the flat part of the 2D sole seen from the front.
pub const FRONT_SOLE_HALF: f64 = 11.0;

/// The standing pose every motion starts from: arms hang slightly away from the body.
pub fn rest() -> Pose {
    let mut pose = [0.0; CHANNELS.len()];
    pose[RIGHT.shoulder_side] = 5.0;
    pose[LEFT.shoulder_side] = 5.0;
    pose[RIGHT.elbow] = 6.0;
    pose[LEFT.elbow] = 6.0;
    pose
}

/// Index of a channel name.
pub fn channel(name: &str) -> Option<usize> {
    CHANNELS.iter().position(|c| *c == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_indices_match_channel_names() {
        for side in [RIGHT, LEFT] {
            let t = side.tag;
            assert_eq!(CHANNELS[side.shoulder], format!("{t}-shoulder"));
            assert_eq!(CHANNELS[side.shoulder_side], format!("{t}-shoulder-side"));
            assert_eq!(CHANNELS[side.elbow], format!("{t}-elbow"));
            assert_eq!(CHANNELS[side.wrist], format!("{t}-wrist"));
            assert_eq!(CHANNELS[side.hip], format!("{t}-hip"));
            assert_eq!(CHANNELS[side.hip_side], format!("{t}-hip-side"));
            assert_eq!(CHANNELS[side.knee], format!("{t}-knee"));
            assert_eq!(CHANNELS[side.ankle], format!("{t}-ankle"));
        }
        assert_eq!(CHANNELS[HEAD_TURN], "head-turn");
        assert_eq!(GROUND, 294.0);
    }
}
