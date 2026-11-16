//! Per-element visual state (alpha, pose, video frame, and animated filter amounts) sampled at one
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
        Self {
            alpha: alpha * pose.opacity,
            x: pose.x,
            y: pose.y,
            scale: pose.scale,
            rotate: pose.rotate,
            reveal: pose.reveal,
            media: match &element.content {
                Content::Video(video) => video.frame_at(time - element.start) as u32,
                _ => 0,
            },
            filters,
        }
    }
}
