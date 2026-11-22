//! Per-element visual state (alpha, pose, 2.5D depth, 3D scene key, image-sequence or video frame, animated filter and drop-shadow, pixel-effect, and clip-path values) sampled at one
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
    pub(in crate::render) camera_pan: [f32; 2],
    /// Source frame of a video or image sequence, so a new frame is never mistaken for an unchanged one.
    pub(in crate::render) media: u32,
    /// Animated `filter` amounts by `FilterChannel::index`; all zero when the filter is static.
    pub(in crate::render) filters: [f32; crate::model::FilterChannel::COUNT],
    /// Animated `drop-shadow()` channels (see `model::ShadowChannels`); all zero when static.
    pub(in crate::render) shadow: crate::model::ShadowChannels,
    /// `[tint, glitch, rgb_split, bloom]` (see `model::FxMotion`); all zero when there are none.
    pub(in crate::render) fx: [f32; 4],
    /// Glitch pattern index (`GLITCH_RATE` steps since the clip start); 0 without a glitch.
    pub(in crate::render) fx_seed: u32,
    /// Bits of the animated `clip-path` values, so a moving clip is never mistaken for a still one.
    pub(in crate::render) clip_key: u64,
    /// Whether the scale of this element or an ancestor is changing at this instant (set by
    /// `states_at`), so vector artwork drawn at a passing size can reuse a zoom-ladder raster.
    pub(in crate::render) zooming: bool,
    /// A plane's `[z, rotateX, rotateY]` (see `crate::space::Depth::at`); zero for 2D elements.
    pub(in crate::render) depth: [f32; 3],
    /// A space camera's `[dolly, rotateX, rotateY, rotate]` (see `crate::space::Depth::camera_at`);
    /// zero for every other element.
    pub(in crate::render) camera_3d: [f32; 4],
    /// Key of a `data-hf-3d` scene's animated state (see `Scene3d::key`): 0 when it never moves,
    /// otherwise different whenever a node, light, or the camera changes. Counts as movement.
    pub(in crate::render) scene_3d: u32,
}

/// How far ahead (seconds) a scale is compared to decide that it is changing. Comparing values at
/// two instants keeps the decision a pure function of time, independent of the frame rate.
const ZOOM_PROBE: f64 = 1e-3;

impl FrameState {
    pub(in crate::render) fn at(element: &Element, time: f64, alpha: f32) -> Self {
        if alpha <= 0.0 {
            return Self::default();
        }
        let elapsed = time - element.start;
        let pose = element.motion.pose(elapsed);
        let motion = &element.motion;
        let filters = if motion.filter.is_static() {
            [0.0; crate::model::FilterChannel::COUNT]
        } else {
            motion
                .filter
                .eval(elapsed, motion.hold_before, motion.hold_after)
        };
        let shadow = if motion.filter.shadow_animated() {
            motion
                .filter
                .eval_shadow(elapsed, motion.hold_before, motion.hold_after)
        } else {
            crate::model::NO_SHADOW
        };
        let fx = if motion.fx.is_static() {
            [0.0; 4]
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
            camera_pan: if element.layout.has_parallax {
                motion.camera_pan(elapsed)
            } else {
                [0.0; 2]
            },
            media: match &element.content {
                Content::Sequence(sequence) => sequence.frame_at(elapsed) as u32,
                Content::Video(video) => video.frame_at(time - element.start) as u32,
                Content::DrawnSvg(drawing) => drawing.key(time - element.start),
                _ => 0,
            },
            filters,
            shadow,
            fx,
            fx_seed,
            clip_key,
            zooming: scale_changes(element, elapsed, pose.scale),
            depth: element.depth.at(elapsed),
            camera_3d: element.depth.camera_at(elapsed),
            scene_3d: match &element.content {
                Content::Scene3d(scene) => scene.key(elapsed),
                _ => 0,
            },
        }
    }
}

/// Whether the element's own scale (keyframes, CSS animation, or camera zoom) or its 2.5D depth
/// (plane depth and tilt, or a space camera's dolly) differs between `elapsed` and a moment later.
fn scale_changes(element: &Element, elapsed: f64, scale: f32) -> bool {
    let motion = &element.motion;
    ((!motion.scale.0.is_empty() || motion.rig.is_some())
        && motion.pose(elapsed + ZOOM_PROBE).scale != scale)
        || (element.depth.is_animated()
            && (element.depth.at(elapsed + ZOOM_PROBE) != element.depth.at(elapsed)
                || element.depth.camera_at(elapsed + ZOOM_PROBE)
                    != element.depth.camera_at(elapsed)))
}
