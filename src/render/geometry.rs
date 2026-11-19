//! Pixel-space and layout-space rectangles shared by layout and painting.

#[derive(Clone, Copy, Debug)]
pub(in crate::render) struct Rect {
    pub(in crate::render) x: f32,
    pub(in crate::render) y: f32,
    pub(in crate::render) w: f32,
    pub(in crate::render) h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::render) struct PixelRect {
    pub(in crate::render) x0: usize,
    pub(in crate::render) y0: usize,
    pub(in crate::render) x1: usize,
    pub(in crate::render) y1: usize,
}

impl PixelRect {
    pub(in crate::render) fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

pub(in crate::render) fn clip_pixels(
    (width, height): (u32, u32),
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) -> Option<PixelRect> {
    let x0 = x0.clamp(0, width as i32) as usize;
    let y0 = y0.clamp(0, height as i32) as usize;
    let x1 = x1.clamp(0, width as i32) as usize;
    let y1 = y1.clamp(0, height as i32) as usize;
    (x0 < x1 && y0 < y1).then_some(PixelRect { x0, y0, x1, y1 })
}
