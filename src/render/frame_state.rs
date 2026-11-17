//! Per-element visual state (alpha, pose, video frame, animated filter, pixel-effect, and clip-path values) sampled at one
//! instant; compared across frames to reuse pixels.

use crate::model::{Content, Element};

#[derive(Clone, Copy, Default, PartialEq)]
pub(in crate::render) struct FrameState {
    pub(in crate::render) alpha: f32,
    pub(in crate::render) x: f32,
    pub(in crate::render) y: f32,
    pub(in crate::render) scale: f32,
    pub(in crate::render) rotate: f32,
    pub(in crate::render) reveal: f32,
    /// Source frame of a `<video>`, so a new frame is never mistaken for an unchanged one.
    pub(in crate::render) media: u32,
    /// Animated `filter` amounts by `FilterChannel::index`; all zero when the filter is static.
    pub(in crate::render) filters: [f32; 8],
    /// `[tint, glitch, rgb_split]` (see `model::FxMotion`); all zero when there are none.
    pub(in crate::render) fx: [f32; 3],
    /// Glitch pattern index (`GLITCH_RATE` steps since the clip start); 0 without a glitch.
    pub(in crate::render) fx_seed: u32,
    /// Bits of the animated `clip-path` values, so a moving clip is never mistaken for a still one.
    pub(in crate::render) clip_key: u64,
}

impl FrameState {
    pub(in crate::render) fn at(element: &Element, time: f64, alpha: f32) -> Self {
        if alpha <= 0.0 {
            return Self::default();
        }
        let elapsed = time - element.start;
        let pose = element.motion.pose(elapsed);
        let motion = &element.motion;
        let filters = if motion.filter.is_static() {
            [0.0; 8]
        } else {
            motion
                .filter
                .eval(elapsed, motion.hold_before, motion.hold_after)
        };
        let fx = if motion.fx.is_static() {
            [0.0; 3]
        } else {
            motion
                .fx
                .eval(elapsed, motion.hold_before, motion.hold_after)
        };
        // Quantized so the pattern does not depend on the frame rate or on sub-frame samples.
        let fx_seed = if fx[1] > 0.0 {
            (elapsed.max(0.0) * crate::model::GLITCH_RATE + 1e-6).floor() as u32
        } else {
            0
        };
        let clip_key = motion.clip.iter().fold(0_u64, |key, track| {
            let value = track.eval(elapsed, 0.0, true, true);
            (key.rotate_left(17) ^ u64::from(value.to_bits())).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        });
        Self {
            alpha: alpha * pose.opacity,
            x: pose.x,
            y: pose.y,
            scale: pose.scale,
            rotate: pose.rotate,
            reveal: pose.reveal,
            media: match &element.content {
                Content::Video(video) => video.frame_at(time - element.start) as u32,
                Content::DrawnSvg(drawing) => drawing.key(time - element.start),
                _ => 0,
            },
            filters,
            fx,
            fx_seed,
            clip_key,
        }
    }
}
