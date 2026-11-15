//! CSS animation model: declarations, resolved frames, and shared limits.

pub(in crate::parse) mod camera;
pub(in crate::parse) mod decl;
pub(in crate::parse) mod keyframes;
pub(in crate::parse) mod motion;
pub(in crate::parse) mod resolve;
pub(in crate::parse) mod reveal;
pub(in crate::parse) mod timing;

use crate::model::Easing;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::parse) enum Iterations {
    Count(u32),
    Infinite,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::parse) enum Direction {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

#[derive(Clone, Debug)]
pub(in crate::parse) struct AnimationDecl {
    pub(in crate::parse) name: Option<String>,
    pub(in crate::parse) duration: Option<f64>,
    pub(in crate::parse) delay: f64,
    pub(in crate::parse) easing: Easing,
    pub(in crate::parse) iterations: Iterations,
    pub(in crate::parse) direction: Direction,
    pub(in crate::parse) backwards: bool,
    pub(in crate::parse) forwards: bool,
}

impl Default for AnimationDecl {
    fn default() -> Self {
        Self {
            name: None,
            duration: None,
            delay: 0.0,
            easing: CSS_EASE,
            iterations: Iterations::Count(1),
            direction: Direction::Normal,
            backwards: false,
            forwards: false,
        }
    }
}

/// One keyframe with values already resolved for a specific element.
#[derive(Clone, Debug)]
pub(in crate::parse) struct CssFrame {
    pub(in crate::parse) offset: f64,
    /// `transform` as (translate x, translate y, scale).
    pub(in crate::parse) transform: Option<(f32, f32, f32, f32)>,
    pub(in crate::parse) opacity: Option<f32>,
    pub(in crate::parse) easing: Option<Easing>,
}

#[derive(Clone, Debug)]
pub(in crate::parse) struct AnimationSpec {
    pub(in crate::parse) name: String,
    pub(in crate::parse) frames: Vec<CssFrame>,
    pub(in crate::parse) decl: AnimationDecl,
    pub(in crate::parse) duration: f64,
}

pub(in crate::parse) const CSS_EASE: Easing = Easing::Bezier([0.25, 0.1, 0.25, 1.0]);

pub(in crate::parse) const MAX_TRACK_POINTS: usize = 50_000;
