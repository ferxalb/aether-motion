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
        match self {
            Self::Px(n) => n,
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
    pub keyframes: Vec<Keyframe>,
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
