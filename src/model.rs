//! Parsed composition model: styles, content, timing, and time-sampled motion tracks.

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

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub enum Background {
    Solid(Color),
    Linear {
        from: Color,
        to: Color,
        horizontal: bool,
    },
    /// Any other gradient: angles, corners, several stops, translucency, or radial.
    Gradient(Gradient),
}

impl Background {
    /// The general gradient this background paints (`None` for a solid color).
    pub fn to_gradient(&self) -> Option<Gradient> {
        match self {
            Self::Solid(_) => None,
            Self::Gradient(gradient) => Some(gradient.clone()),
            Self::Linear {
                from,
                to,
                horizontal,
            } => Some(Gradient {
                kind: GradientKind::Linear(LinearDirection::Angle(if *horizontal {
                    90.0
                } else {
                    180.0
                })),
                stops: vec![
                    Stop {
                        pos: 0.0,
                        color: *from,
                    },
                    Stop {
                        pos: 1.0,
                        color: *to,
                    },
                ],
            }),
        }
    }
}

/// Color stop at `pos` (0..=1 along the gradient line or radius).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub pos: f32,
    pub color: Color,
}

/// Direction of a `linear-gradient`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinearDirection {
    /// CSS angle in degrees: 0 points up and angles grow clockwise.
    Angle(f32),
    /// `to <corner>`; `(x, y)` is `(1|-1, 1|-1)` with +1 meaning right / down.
    Corner(i8, i8),
}

#[derive(Clone, Debug, PartialEq)]
pub enum GradientKind {
    Linear(LinearDirection),
    Radial {
        circle: bool,
        /// Center as a fraction of the box.
        center: (f32, f32),
        /// `false`: `farthest-corner` (default); `true`: `farthest-side`.
        farthest_side: bool,
    },
}

/// A gradient with sorted stops. Colors may be translucent and interpolate premultiplied.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<Stop>,
}

/// One `box-shadow` layer (outer or `inset`) or the single `text-shadow` (never inset).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    /// `inset`: the shadow is cast inside the padding box instead of outside the border box.
    pub inset: bool,
}

/// One CSS `filter` color function, applied to unpremultiplied sRGB values in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorFilter {
    Brightness(f32),
    Contrast(f32),
    Saturate(f32),
    /// Amount in 0..=1.
    Grayscale(f32),
    /// Amount in 0..=1.
    Sepia(f32),
    /// Angle in degrees.
    HueRotate(f32),
    /// Amount in 0..=1.
    Invert(f32),
    /// Amount in 0..=1; multiplies alpha.
    Opacity(f32),
}

/// A `filter` function whose amount can be animated, in the order used for animated functions that
/// the static `filter` does not list. `HueRotate` is in degrees and `Blur` in px; the rest are amounts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterChannel {
    Blur,
    Brightness,
    Contrast,
    Saturate,
    Grayscale,
    Sepia,
    HueRotate,
    Invert,
}

impl FilterChannel {
    pub const ALL: [Self; 8] = [
        Self::Blur,
        Self::Brightness,
        Self::Contrast,
        Self::Saturate,
        Self::Grayscale,
        Self::Sepia,
        Self::HueRotate,
        Self::Invert,
    ];

    /// Index into per-channel arrays (`FilterMotion::tracks`, values).
    pub fn index(self) -> usize {
        self as usize
    }

    /// The amount that leaves pixels unchanged.
    pub fn neutral(self) -> f32 {
        match self {
            Self::Brightness | Self::Contrast | Self::Saturate => 1.0,
            _ => 0.0,
        }
    }

    /// Keeps an interpolated amount in the function's valid range (overshooting easings can leave it).
    pub fn clamp(self, value: f32) -> f32 {
        match self {
            Self::Blur => value.clamp(0.0, 200.0),
            Self::Grayscale | Self::Sepia | Self::Invert => value.clamp(0.0, 1.0),
            Self::HueRotate => value,
            Self::Brightness | Self::Contrast | Self::Saturate => value.max(0.0),
        }
    }

    /// The channel of a color function; `opacity()` has none (it is not animatable).
    pub fn of(filter: ColorFilter) -> Option<Self> {
        Some(match filter {
            ColorFilter::Brightness(_) => Self::Brightness,
            ColorFilter::Contrast(_) => Self::Contrast,
            ColorFilter::Saturate(_) => Self::Saturate,
            ColorFilter::Grayscale(_) => Self::Grayscale,
            ColorFilter::Sepia(_) => Self::Sepia,
            ColorFilter::HueRotate(_) => Self::HueRotate,
            ColorFilter::Invert(_) => Self::Invert,
            ColorFilter::Opacity(_) => return None,
        })
    }

    /// The amount of a color function (degrees for `hue-rotate`).
    pub fn amount(filter: ColorFilter) -> f32 {
        match filter {
            ColorFilter::Brightness(v)
            | ColorFilter::Contrast(v)
            | ColorFilter::Saturate(v)
            | ColorFilter::Grayscale(v)
            | ColorFilter::Sepia(v)
            | ColorFilter::HueRotate(v)
            | ColorFilter::Invert(v)
            | ColorFilter::Opacity(v) => v,
        }
    }

    /// The color function of this channel with `value`; `None` for `Blur`.
    pub fn color(self, value: f32) -> Option<ColorFilter> {
        Some(match self {
            Self::Blur => return None,
            Self::Brightness => ColorFilter::Brightness(value),
            Self::Contrast => ColorFilter::Contrast(value),
            Self::Saturate => ColorFilter::Saturate(value),
            Self::Grayscale => ColorFilter::Grayscale(value),
            Self::Sepia => ColorFilter::Sepia(value),
            Self::HueRotate => ColorFilter::HueRotate(value),
            Self::Invert => ColorFilter::Invert(value),
        })
    }

    /// The `data-hf-keyframes` field name.
    pub fn key(self) -> &'static str {
        match self {
            Self::Blur => "blur",
            Self::Brightness => "brightness",
            Self::Contrast => "contrast",
            Self::Saturate => "saturate",
            Self::Grayscale => "grayscale",
            Self::Sepia => "sepia",
            Self::HueRotate => "hueRotate",
            Self::Invert => "invert",
        }
    }
}

/// `mix-blend-mode` separable blend functions (W3C Compositing Level 1), plus `plus-lighter`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    PlusLighter,
}

/// Shape function of a `clip-path`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipKind {
    /// `inset(top right bottom left round radius)`: params `[top, right, bottom, left, radius]`.
    Inset,
    /// `circle(r at x y)`: params `[r, x, y]`; a % radius is relative to
    /// `sqrt(width² + height²) / sqrt(2)`.
    Circle,
    /// `ellipse(rx ry at x y)`: params `[rx, ry, x, y]`.
    Ellipse,
    /// `polygon(x y, ...)`: params `[x0, y0, x1, y1, ...]`.
    Polygon,
}

/// A `clip-path` basic shape in the element's border box. Each parameter keeps the unit it was
/// written in (`px` or `%`); horizontal % values are relative to the width and vertical ones to the
/// height. Px values follow the element's scale.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipPath {
    pub kind: ClipKind,
    pub params: Vec<Length>,
}

impl ClipPath {
    /// The parameters with `values` in place of their numbers (same units), as animated.
    pub fn with_values(&self, values: &[f32]) -> Self {
        Self {
            kind: self.kind,
            params: self
                .params
                .iter()
                .zip(values)
                .map(|(param, &value)| match param {
                    Length::Px(_) => Length::Px(value),
                    Length::Percent(_) => Length::Percent(value),
                })
                .collect(),
        }
    }

    /// The numbers of the parameters, in order.
    pub fn values(&self) -> Vec<f32> {
        self.params
            .iter()
            .map(|param| match *param {
                Length::Px(n) | Length::Percent(n) => n,
            })
            .collect()
    }
}

/// `-webkit-text-stroke`: an outline of the glyphs centered on their edges, painted over the fill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStroke {
    /// Width in composition px; 0 means none.
    pub width: f32,
    /// `None` is `currentColor` (the text's `color`).
    pub color: Option<Color>,
}

/// Text style of an inline run (`<span>`, `<strong>`, `<b>`, `<code>`) over its element's text style.
/// `None` fields inherit from the enclosing run or element.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineStyle {
    pub color: Option<Color>,
    pub font_family: Option<String>,
    pub font_weight: Option<u16>,
    pub letter_spacing: Option<f32>,
    pub text_fill_color: Option<Color>,
    pub text_stroke_width: Option<f32>,
    pub text_stroke_color: Option<Color>,
    /// `background` with `background-clip: text`: the run's glyphs are filled with it.
    pub text_background: Option<Background>,
}

/// A styled byte range `[start, end)` of an element's text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    pub style: InlineStyle,
}

/// `line-height`: a multiple of the font size (unitless) or an absolute size in px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Multiple(f32),
    Px(f32),
}

/// `white-space`: `Normal` wraps text at spaces; `Nowrap` keeps one line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WhiteSpace {
    Normal,
    Nowrap,
}

#[derive(Clone, Debug, Default)]
pub struct Style {
    pub left: Option<Length>,
    pub top: Option<Length>,
    pub width: Option<Length>,
    pub height: Option<Length>,
    /// Upper bound for the border-box width (`max-width`); `None` means no limit.
    pub max_width: Option<Length>,
    pub background: Option<Background>,
    pub border_radius: f32,
    pub color: Option<Color>,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    /// Inherited numeric `font-weight` (1..=1000); `None` means 400.
    pub font_weight: Option<u16>,
    /// Inherited `letter-spacing` in composition px (may be negative); `None` means `normal`.
    pub letter_spacing: Option<f32>,
    pub opacity: Option<f32>,
    pub text_align: Option<String>,
    /// Inherited `-webkit-text-fill-color`: the glyph fill, in place of `color`.
    pub text_fill_color: Option<Color>,
    /// Inherited `-webkit-text-stroke-width` (composition px) and `-webkit-text-stroke-color`.
    pub text_stroke_width: Option<f32>,
    pub text_stroke_color: Option<Color>,
    /// `background-clip: text`: the background fills the glyphs instead of the box.
    pub background_clip_text: bool,
    /// Inherited text wrapping controls; see [`LineHeight`] and [`WhiteSpace`].
    /// Inherited `text-shadow`: outer `None` means unset (inherit), inner `None` is `none`.
    pub text_shadow: Option<Option<Shadow>>,
    pub line_height: Option<LineHeight>,
    pub white_space: Option<WhiteSpace>,
    pub display: Option<String>,
    pub align_items: Option<String>,
    pub justify_content: Option<String>,
    pub object_fit: Option<String>,
    pub z_index: Option<i32>,
    pub position: Option<String>,
    /// Static `transform`: translate in px and uniform scale around the element center.
    pub translate: (f32, f32),
    pub scale: Option<f32>,
    /// Static `transform: rotate()` in degrees, clockwise, around the element center.
    pub rotate: f32,
    /// `box-shadow` layers (outer and inset) in CSS order (the first is painted on top); empty means none.
    pub shadows: Vec<Shadow>,
    /// `backdrop-filter: blur(<px>)` radius (standard deviation) in composition px; 0 means none.
    pub backdrop_blur: f32,
    /// `filter: blur(<px>)` radius (standard deviation) in composition px applied to the whole element
    /// (box, shadows, content, children); 0 means none.
    pub filter_blur: f32,
    /// `filter` color functions after the optional blur, applied in order; empty means none.
    pub filter_colors: Vec<ColorFilter>,
    /// `filter: drop-shadow()`: a shadow of the element's painted shape (alpha), drawn under it.
    pub drop_shadow: Option<Shadow>,
    /// `mix-blend-mode`: how the element (with its subtree) combines with what is behind it.
    pub blend: BlendMode,
    /// `clip-path`: only the part of the element inside this shape is drawn.
    pub clip_path: Option<ClipPath>,
    /// `mask-image`: a gradient over the border box whose alpha scales the element's.
    pub mask: Option<Gradient>,
    /// Solid border drawn inside the box (border-box sizing); `None` color means white.
    pub border_width: f32,
    pub border_color: Option<Color>,
    /// `flex-direction: column` (row otherwise).
    pub column: bool,
    pub gap: f32,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    /// Top, right, bottom, left. Non-negative px; applies to flex items and offsets other boxes.
    pub margin: [f32; 4],
    /// `overflow: hidden`: content is clipped to the padding box, following the rounded corners.
    pub overflow_hidden: bool,
    pub flex_grow: f32,
    /// `flex: n` ignores the content size when distributing space.
    pub flex_basis_zero: bool,
}

#[derive(Clone, Debug)]
pub enum Content {
    Container,
    Text(String),
    Image(PathBuf),
    /// Serialized `<svg>` markup from the document, rasterized at the target size.
    InlineSvg(std::sync::Arc<str>),
    /// A local H.264 video drawn one source frame per output frame.
    Video(VideoSource),
    /// Inline `<svg>` with `data-hf-draw`: strokes reveal themselves over time.
    DrawnSvg(std::sync::Arc<crate::vector::DrawOnSvg>),
}

/// A `<video>` clip: which file, where in it playback starts, and its frame table.
#[derive(Clone, Debug)]
pub struct VideoSource {
    pub path: PathBuf,
    /// Seconds into the source where the clip starts (`data-media-start`).
    pub media_start: f64,
    /// Repeat the source when the clip outlasts it; otherwise the last frame holds.
    pub looped: bool,
    /// The source's audio track is mixed with the clip (it has one and the element is not `muted`).
    pub audio: bool,
    /// `data-hf-speed` or `data-hf-speed-ramp`; `None` plays at 1x.
    pub speed: Option<std::sync::Arc<crate::speed::SpeedCurve>>,
    pub index: std::sync::Arc<crate::video::VideoIndex>,
}

impl VideoSource {
    /// Seconds into the source shown `local` seconds after the clip starts, before looping.
    pub fn media_time(&self, local: f64) -> f64 {
        let local = local.max(0.0);
        self.media_start
            + match &self.speed {
                Some(curve) => curve.media(local),
                None => local,
            }
    }

    /// Display frame shown `local` seconds after the clip starts.
    pub fn frame_at(&self, local: f64) -> usize {
        let mut media = self.media_time(local);
        if self.looped && self.index.duration > 0.0 {
            media = media.rem_euclid(self.index.duration);
        }
        self.index.frame_at(media)
    }
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

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub at: f64,
    pub x: f32,
    pub y: f32,
    pub opacity: f32,
    pub easing: Easing,
    /// Filter amounts given explicitly at this keyframe, indexed by [`FilterChannel::index`].
    pub filters: [Option<f32>; 8],
    /// `glitch` and `rgbSplit` given explicitly at this keyframe.
    pub effects: [Option<f32>; 2],
    /// `clipPath` given explicitly at this keyframe.
    pub clip: Option<ClipPath>,
}

/// Layout-check opt-outs from `data-layout-allow-*` attributes; descendants inherit them when checked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutHints {
    pub allow_safe_zone: bool,
    pub allow_caption_zone: bool,
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
    pub layout: LayoutHints,
    /// Inline runs of styled text (sorted, non-overlapping); empty for plain text.
    pub spans: Vec<TextSpan>,
}

#[derive(Debug)]
pub struct Composition {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub background: Color,
    /// `data-hf-vignette` on the root: corner darkening strength, 0..=1.
    pub vignette: f32,
    /// `data-hf-grain` on the root: film grain amplitude, 0..=1.
    pub grain: f32,
    /// `filter` color functions on the root: a grade applied to every finished frame.
    pub grade: Vec<ColorFilter>,
    /// `data-motion-blur` on the root: shutter settings, or `None` for sharp frames.
    pub motion_blur: Option<MotionBlur>,
    pub elements: Vec<Element>,
    pub ids: HashMap<String, usize>,
    /// `<audio>` clips in document order; they take no part in layout or painting.
    pub audio: Vec<AudioClip>,
    /// `data-composition-variables`: the declared variables and the values this parse used.
    pub variables: Variables,
}

/// Declared composition variables, already applied to the document before it was parsed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Variables {
    pub declared: Vec<Variable>,
    /// Elements (with their subtrees) removed because their `data-var-if` was false.
    pub removed: usize,
}

/// One declared variable with the value in use.
#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub id: String,
    pub kind: VariableKind,
    /// The value as text, as `data-var-text` writes it (numbers in shortest decimal form).
    pub value: String,
    /// The declared default, in the same form.
    pub default: String,
    /// True when a value passed by the caller replaced the default.
    pub overridden: bool,
    pub uses: VariableUses,
}

/// Declared type of a composition variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariableKind {
    String,
    Number,
    Color,
    Boolean,
    Enum,
    Image,
    Font,
}

impl VariableKind {
    /// The `type` name used in declarations.
    pub fn name(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Color => "color",
            Self::Boolean => "boolean",
            Self::Enum => "enum",
            Self::Image => "image",
            Self::Font => "font",
        }
    }
}

/// How often a variable is referenced, counted before `data-var-if` removes anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VariableUses {
    /// `var(--id)` references in `<style>` sheets and `style` attributes.
    pub css: usize,
    pub text: usize,
    pub src: usize,
    pub condition: usize,
}

/// Shutter for `data-motion-blur`: each output frame averages several instants spread evenly
/// over a window of `shutter` degrees of one frame, centered on the frame time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlur {
    /// Shutter angle in degrees, in (0, 720]; 360 spans one whole frame interval.
    pub shutter: f32,
    /// Sub-frame samples per frame, 2..=64; `None` sizes them to the motion in each frame.
    pub samples: Option<u32>,
}

impl MotionBlur {
    /// Fewest and most samples chosen automatically.
    pub const AUTO_SAMPLES: (u32, u32) = (8, 64);

    /// `(first, last)` instants of the shutter window for the frame at `time`, clamped like samples.
    pub fn window(&self, time: f64, fps: u32, duration: f64) -> (f64, f64) {
        let half = f64::from(self.shutter) / 720.0 / f64::from(fps.max(1));
        let last = (duration - 1e-6).max(0.0);
        (
            (time - half).clamp(0.0, last),
            (time + half).clamp(0.0, last),
        )
    }

    /// `n` sample instants for the frame at `time` (seconds) at `fps`, clamped to `[0, duration)`.
    pub fn sample_times(&self, time: f64, fps: u32, duration: f64, n: u32) -> Vec<f64> {
        let window = f64::from(self.shutter) / 360.0 / f64::from(fps.max(1));
        let n = n.max(1);
        let last = (duration - 1e-6).max(0.0);
        (0..n)
            .map(|i| {
                let offset = (f64::from(i) + 0.5) / f64::from(n) - 0.5;
                (time + offset * window).clamp(0.0, last)
            })
            .collect()
    }
}

/// One `<audio>` clip on the composition timeline. Times are in seconds; the mixer converts
/// them to whole samples (see `crate::audio`).
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    pub id: Option<String>,
    /// Local source file (WAV, FLAC, Ogg Vorbis, MP3, or AAC in MP4/M4A).
    pub src: PathBuf,
    /// Resolved start on the composition timeline.
    pub start: f64,
    /// `data-duration`; `None` plays the source (after the trim) to its end, capped at the
    /// composition end.
    pub duration: Option<f64>,
    /// `data-hf-trim-start`: seconds skipped at the start of the source.
    pub trim: f64,
    /// `data-volume`: linear gain applied before the fades.
    pub volume: f32,
    pub fade_in: f64,
    pub fade_out: f64,
    /// `data-hf-loop`: repeat the source to fill `duration`.
    pub looped: bool,
    /// Source seconds where each loop pass restarts: the trim for `<audio>`, and 0 for a video's
    /// sound, so it wraps exactly like the picture (which loops over the whole source).
    pub loop_from: f64,
    /// `data-hf-speed` or `data-hf-speed-ramp` (varispeed: pitch follows the rate); `None` is 1x.
    pub speed: Option<std::sync::Arc<crate::speed::SpeedCurve>>,
    /// `data-hf-volume-ramp`, `data-hf-eq`, and `data-hf-duck` (see `crate::audio::effects`).
    pub effects: crate::audio::effects::AudioEffects,
}

/// Edge a reveal grows from: `Left` uncovers the element starting at its left edge, and so on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RevealFrom {
    Left,
    Right,
    Top,
    Bottom,
}

/// Visual state that time can change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub rotate: f32,
    pub opacity: f32,
    /// Fraction of the box that is revealed (1 = all); only meaningful with [`Motion::wipe`].
    pub reveal: f32,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            rotate: 0.0,
            opacity: 1.0,
            reveal: 1.0,
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
    pub rotate: Track,
    pub opacity: Track,
    /// Revealed fraction of the box over time; always holds its first and last values.
    pub reveal: Track,
    /// Edge the reveal grows from; `None` means the element is never clipped by a reveal.
    pub wipe: Option<RevealFrom>,
    /// Animated `filter` amounts.
    pub filter: FilterMotion,
    /// Animated tint, glitch, and channel split.
    pub fx: FxMotion,
    /// `data-hf-motion-path`: an offset (and optional turn) along a vector path.
    pub path: Option<std::sync::Arc<MotionPath>>,
    /// Animated `clip-path` parameters (one track per [`ClipPath::params`] entry, in its unit);
    /// empty when the clip path is static. Always holds its first and last values.
    pub clip: Vec<Track>,
    /// Hold the first value before the first point (`animation-fill-mode: backwards`).
    pub hold_before: bool,
    /// Hold the last value after the last point (`animation-fill-mode: forwards`).
    pub hold_after: bool,
}

/// Animated `filter` amounts of one element. A channel with an empty track keeps its static value.
#[derive(Clone, Debug, Default)]
pub struct FilterMotion {
    /// Resting amount of each channel (the static `filter`, or the neutral amount).
    pub base: [f32; 8],
    pub tracks: [Track; 8],
    /// Animated color channels the static `filter` does not list, applied after its functions.
    pub appended: Vec<FilterChannel>,
}

impl FilterMotion {
    pub fn is_static(&self) -> bool {
        self.tracks.iter().all(|track| track.0.is_empty())
    }

    pub fn is_animated(&self, channel: FilterChannel) -> bool {
        !self.tracks[channel.index()].0.is_empty()
    }

    /// Every channel's amount `elapsed` seconds after the clip start, clamped to its range.
    pub fn eval(&self, elapsed: f64, hold_before: bool, hold_after: bool) -> [f32; 8] {
        FilterChannel::ALL.map(|channel| {
            let i = channel.index();
            let value = self.tracks[i].eval(elapsed, self.base[i], hold_before, hold_after);
            channel.clamp(value)
        })
    }
}

/// Pixel effects outside CSS `filter`, applied in the effect layer after the color functions and
/// before `drop-shadow`, in this order: tint, channel split, glitch. All are neutral at 0.
#[derive(Clone, Debug, Default)]
pub struct FxMotion {
    /// Share (0..=1) of the color replaced by `tint_color`, keeping the element's alpha.
    pub tint: Track,
    pub tint_color: Option<Color>,
    /// Glitch strength (0..=1): horizontal slices displaced by a pattern that changes 24 times a second.
    pub glitch: Track,
    /// Red and blue channel offset in composition px (red moves left, blue right).
    pub rgb_split: Track,
}

/// `data-hf-motion-path`: the element travels along a path, offset by the path's displacement from
/// its first point, so it starts where it rests.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionPath {
    pub measure: crate::vector::PathMeasure,
    /// Seconds from the clip start at which the travel starts.
    pub at: f64,
    /// Seconds the travel takes.
    pub duration: f64,
    pub easing: Easing,
    /// `Some(offset)`: the element turns with the path's direction plus `offset` degrees.
    pub rotate: Option<f32>,
}

impl MotionPath {
    /// `(dx, dy, turn)` `elapsed` seconds after the clip start; the ends hold outside the travel.
    pub fn offset(&self, elapsed: f64) -> (f32, f32, f32) {
        let linear = if self.duration <= 0.0 {
            if elapsed >= self.at {
                1.0
            } else {
                0.0
            }
        } else {
            ((elapsed - self.at) / self.duration).clamp(0.0, 1.0) as f32
        };
        // Overshooting easings may run past the ends; the path clamps there.
        let (x, y, angle) = self
            .measure
            .sample(self.easing.apply(linear) * self.measure.length());
        let (x0, y0) = self.measure.start();
        let turn = self.rotate.map_or(0.0, |offset| angle + offset);
        (x - x0, y - y0, turn)
    }
}

/// Rate at which the glitch pattern changes, independent of the output frame rate.
pub const GLITCH_RATE: f64 = 24.0;

impl FxMotion {
    pub fn is_static(&self) -> bool {
        self.tint.0.is_empty() && self.glitch.0.is_empty() && self.rgb_split.0.is_empty()
    }

    /// `[tint, glitch, rgb_split]` amounts `elapsed` seconds after the clip start.
    pub fn eval(&self, elapsed: f64, hold_before: bool, hold_after: bool) -> [f32; 3] {
        let value = |track: &Track| track.eval(elapsed, 0.0, hold_before, hold_after);
        [
            value(&self.tint).clamp(0.0, 1.0),
            value(&self.glitch).clamp(0.0, 1.0),
            value(&self.rgb_split).clamp(-500.0, 500.0),
        ]
    }

    fn tracks(&self) -> [&Track; 3] {
        [&self.tint, &self.glitch, &self.rgb_split]
    }
}

impl Motion {
    /// The element's `clip-path` `elapsed` seconds after the clip start: `base` (the static
    /// `clip-path`, or the shape of the first animated one) with its animated values, if any.
    pub fn clip_at(&self, base: Option<&ClipPath>, elapsed: f64) -> Option<ClipPath> {
        let base = base?;
        if self.clip.is_empty() {
            return Some(base.clone());
        }
        let values: Vec<f32> = self
            .clip
            .iter()
            .zip(base.values())
            .map(|(track, rest)| track.eval(elapsed, rest, true, true))
            .collect();
        Some(base.with_values(&values))
    }

    /// Whether the element can be drawn rotated (static, animated, or turning along a path).
    pub fn rotates(&self) -> bool {
        self.base.rotate != 0.0
            || !self.rotate.0.is_empty()
            || self.path.as_ref().is_some_and(|path| path.rotate.is_some())
    }

    pub fn pose(&self, elapsed: f64) -> Pose {
        let (b, hb, ha) = (self.base, self.hold_before, self.hold_after);
        let mut pose = self.track_pose(elapsed, b, hb, ha);
        if let Some(path) = &self.path {
            let (dx, dy, turn) = path.offset(elapsed);
            pose.x += dx;
            pose.y += dy;
            pose.rotate += turn;
        }
        pose
    }

    fn track_pose(&self, elapsed: f64, b: Pose, hb: bool, ha: bool) -> Pose {
        Pose {
            x: self.x.eval(elapsed, b.x, hb, ha),
            y: self.y.eval(elapsed, b.y, hb, ha),
            scale: self.scale.eval(elapsed, b.scale, hb, ha),
            rotate: self.rotate.eval(elapsed, b.rotate, hb, ha),
            opacity: self.opacity.eval(elapsed, b.opacity, hb, ha),
            reveal: self.reveal.eval(elapsed, b.reveal, true, true),
        }
    }

    pub fn is_static(&self) -> bool {
        [
            &self.x,
            &self.y,
            &self.scale,
            &self.rotate,
            &self.opacity,
            &self.reveal,
        ]
        .iter()
        .all(|track| track.0.is_empty())
            && self.filter.is_static()
            && self.fx.is_static()
            && self.clip.is_empty()
            && self.path.is_none()
    }

    /// Sorted, distinct point times across all channels.
    pub fn times(&self) -> Vec<f64> {
        let mut times: Vec<f64> = [
            &self.x,
            &self.y,
            &self.scale,
            &self.rotate,
            &self.opacity,
            &self.reveal,
        ]
        .into_iter()
        .chain(&self.filter.tracks)
        .chain(self.fx.tracks())
        .chain(&self.clip)
        .flat_map(|track| track.0.iter().map(|point| point.at))
        .chain(
            self.path
                .iter()
                .flat_map(|path| [path.at, path.at + path.duration]),
        )
        .collect();
        times.sort_by(f64::total_cmp);
        times.dedup();
        times
    }
}
