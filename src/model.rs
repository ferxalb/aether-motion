use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
}

#[derive(Clone, Copy, Debug)]
pub enum Length {
    Px(f32),
    Percent(f32),
}

impl Length {
    pub fn resolve(self, parent: f32) -> f32 {
        self.resolve_scaled(parent, 1.0)
    }

    /// Px lengths follow the accumulated CSS scale; percentages follow the (already scaled) parent.
    pub fn resolve_scaled(self, parent: f32, scale: f32) -> f32 {
        match self {
            Self::Px(n) => n * scale,
            Self::Percent(n) => parent * n / 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Background {
    Solid(Color),
    Linear {
        from: Color,
        to: Color,
        horizontal: bool,
    },
}

/// Single outer `box-shadow` (no inset).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

#[derive(Clone, Debug, Default)]
pub struct Style {
    pub left: Option<Length>,
    pub top: Option<Length>,
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub background: Option<Background>,
    pub border_radius: f32,
    pub color: Option<Color>,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub opacity: Option<f32>,
    pub text_align: Option<String>,
    pub display: Option<String>,
    pub align_items: Option<String>,
    pub justify_content: Option<String>,
    pub object_fit: Option<String>,
    pub z_index: Option<i32>,
    pub position: Option<String>,
    /// Static `transform`: translate in px and uniform scale around the element center.
    pub translate: (f32, f32),
    pub scale: Option<f32>,
    pub shadow: Option<Shadow>,
    /// `flex-direction: column` (row otherwise).
    pub column: bool,
    pub gap: f32,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    pub flex_grow: f32,
    /// `flex: n` ignores the content size when distributing space.
    pub flex_basis_zero: bool,
}

#[derive(Clone, Debug)]
pub enum Content {
    Container,
    Text(String),
    Image(PathBuf),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// CSS `cubic-bezier(x1, y1, x2, y2)`.
    Bezier([f32; 4]),
    /// CSS `steps(count, jump-start | jump-end)`.
    Steps {
        count: u32,
        jump_start: bool,
    },
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t,
            Self::EaseOut => t * (2.0 - t),
            Self::EaseInOut => t * t * (3.0 - 2.0 * t),
            Self::Bezier([x1, y1, x2, y2]) => {
                cubic_bezier(x1 as f64, y1 as f64, x2 as f64, y2 as f64, t as f64) as f32
            }
            Self::Steps { count, jump_start } => {
                let n = count as f32;
                let step = (t * n).floor() + if jump_start { 1.0 } else { 0.0 };
                (step / n).clamp(0.0, 1.0)
            }
        }
    }

    /// Curve `t -> 1 - f(1 - t)`, used when a keyframe interval plays backwards.
    pub fn reversed(self) -> Self {
        match self {
            Self::Linear => Self::Linear,
            Self::EaseIn => Self::EaseOut,
            Self::EaseOut => Self::EaseIn,
            Self::EaseInOut => Self::EaseInOut,
            Self::Bezier([x1, y1, x2, y2]) => {
                Self::Bezier([1.0 - x2, 1.0 - y2, 1.0 - x1, 1.0 - y1])
            }
            Self::Steps { count, jump_start } => Self::Steps {
                count,
                jump_start: !jump_start,
            },
        }
    }
}

fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let coefficients = |p1: f64, p2: f64| {
        let c = 3.0 * p1;
        let b = 3.0 * (p2 - p1) - c;
        (1.0 - c - b, b, c)
    };
    let sample = |(a, b, c): (f64, f64, f64), s: f64| ((a * s + b) * s + c) * s;
    let (cx, cy) = (coefficients(x1, x2), coefficients(y1, y2));
    // x(s) is monotonic for x1, x2 in [0, 1]; bisection keeps the result deterministic.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if sample(cx, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    sample(cy, (lo + hi) / 2.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub at: f64,
    pub x: f32,
    pub y: f32,
    pub opacity: f32,
    pub easing: Easing,
}

#[derive(Clone, Debug)]
pub struct Element {
    pub tag: String,
    pub id: Option<String>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub content: Content,
    pub style: Style,
    pub start: f64,
    pub end: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub track: i32,
    pub motion: Motion,
}

#[derive(Debug)]
pub struct Composition {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub background: Color,
    pub elements: Vec<Element>,
    pub ids: HashMap<String, usize>,
}

/// Visual state that time can change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub opacity: f32,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            opacity: 1.0,
        }
    }
}

/// Value reached at `at` (seconds from the clip start). `easing` shapes the segment that ends here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackPoint {
    pub at: f64,
    pub value: f32,
    pub easing: Easing,
}

#[derive(Clone, Debug, Default)]
pub struct Track(pub Vec<TrackPoint>);

impl Track {
    pub fn eval(&self, elapsed: f64, base: f32, hold_before: bool, hold_after: bool) -> f32 {
        let (Some(first), Some(last)) = (self.0.first(), self.0.last()) else {
            return base;
        };
        if elapsed < first.at {
            return if hold_before { first.value } else { base };
        }
        if elapsed >= last.at {
            return if hold_after { last.value } else { base };
        }
        let next = self.0.partition_point(|point| point.at <= elapsed);
        let (from, to) = (self.0[next - 1], self.0[next]);
        let t = ((elapsed - from.at) / (to.at - from.at)) as f32;
        from.value + (to.value - from.value) * to.easing.apply(t)
    }
}

/// Per-channel animation of one element: pure function of time.
#[derive(Clone, Debug, Default)]
pub struct Motion {
    /// Resting state: static `transform` and `opacity`.
    pub base: Pose,
    pub x: Track,
    pub y: Track,
    pub scale: Track,
    pub opacity: Track,
    /// Hold the first value before the first point (`animation-fill-mode: backwards`).
    pub hold_before: bool,
    /// Hold the last value after the last point (`animation-fill-mode: forwards`).
    pub hold_after: bool,
}

impl Motion {
    pub fn pose(&self, elapsed: f64) -> Pose {
        let (b, hb, ha) = (self.base, self.hold_before, self.hold_after);
        Pose {
            x: self.x.eval(elapsed, b.x, hb, ha),
            y: self.y.eval(elapsed, b.y, hb, ha),
            scale: self.scale.eval(elapsed, b.scale, hb, ha),
            opacity: self.opacity.eval(elapsed, b.opacity, hb, ha),
        }
    }

    pub fn is_static(&self) -> bool {
        [&self.x, &self.y, &self.scale, &self.opacity]
            .iter()
            .all(|track| track.0.is_empty())
    }

    /// Sorted, distinct point times across all channels.
    pub fn times(&self) -> Vec<f64> {
        let mut times: Vec<f64> = [&self.x, &self.y, &self.scale, &self.opacity]
            .iter()
            .flat_map(|track| track.0.iter().map(|point| point.at))
            .collect();
        times.sort_by(f64::total_cmp);
        times.dedup();
        times
    }
}
