//! Per-element visual state (alpha and pose) sampled at one instant; compared across frames to reuse pixels.

use crate::model::Element;

#[derive(Clone, Copy, Default, PartialEq)]
pub(in crate::render) struct FrameState {
    pub(in crate::render) alpha: f32,
    pub(in crate::render) x: f32,
    pub(in crate::render) y: f32,
    pub(in crate::render) scale: f32,
    pub(in crate::render) rotate: f32,
}

impl FrameState {
    pub(in crate::render) fn at(element: &Element, time: f64, alpha: f32) -> Self {
        if alpha <= 0.0 {
            return Self::default();
        }
        let pose = element.motion.pose(time - element.start);
        Self {
            alpha: alpha * pose.opacity,
            x: pose.x,
            y: pose.y,
            scale: pose.scale,
            rotate: pose.rotate,
        }
    }
}
