//! Shared helpers for integration tests: render a small composition and inspect its pixels.
#![allow(dead_code)]

use aether_motion::parse::parse_file;
use aether_motion::render::Renderer;

pub struct Shot {
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

impl Shot {
    pub fn at(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 4;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }

    /// Bounding box `(left, top, right, bottom)`, all inclusive, of pixels matching `test`.
    pub fn bbox(&self, test: impl Fn([u8; 3]) -> bool) -> Option<(usize, usize, usize, usize)> {
        let mut found: Option<(usize, usize, usize, usize)> = None;
        for y in 0..self.height {
            for x in 0..self.width {
                if test(self.at(x, y)) {
                    found = Some(found.map_or((x, y, x, y), |(l, t, r, b)| {
                        (l.min(x), t.min(y), r.max(x), b.max(y))
                    }));
                }
            }
        }
        found
    }

    pub fn count(&self, test: impl Fn([u8; 3]) -> bool) -> usize {
        (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .filter(|&(x, y)| test(self.at(x, y)))
            .count()
    }

    pub fn lit(&self, x: usize, y: usize) -> bool {
        self.at(x, y).iter().any(|&channel| channel > 40)
    }

    /// Vertical runs of rows that contain painted pixels, as `(top, bottom_exclusive)`.
    pub fn bands(&self) -> Vec<(usize, usize)> {
        let mut bands: Vec<(usize, usize)> = Vec::new();
        let mut open: Option<usize> = None;
        for y in 0..=self.height {
            let row = y < self.height && (0..self.width).any(|x| self.lit(x, y));
            match (row, open) {
                (true, None) => open = Some(y),
                (false, Some(top)) => {
                    bands.push((top, y));
                    open = None;
                }
                _ => {}
            }
        }
        bands
    }

    /// Leftmost and rightmost painted columns in rows `top..bottom`.
    pub fn extent(&self, (top, bottom): (usize, usize)) -> (usize, usize) {
        let columns: Vec<usize> = (0..self.width)
            .filter(|&x| (top..bottom).any(|y| self.lit(x, y)))
            .collect();
        (columns[0], columns[columns.len() - 1])
    }
}

/// Unique scratch directory per render, so tests running in parallel never share a file.
pub fn scratch(name: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("aether-hf-wrap-{name}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn render(
    name: &str,
    (width, height): (usize, usize),
    scale: f32,
    css: &str,
    body: &str,
) -> Shot {
    let html = format!(
        r#"<html><head><style>{css}</style></head><body>
        <main data-composition-id="wrap" data-width="{width}" data-height="{height}" data-duration="1">{body}</main>
        </body></html>"#
    );
    let path = scratch(name).join("wrap.html");
    std::fs::write(&path, html).unwrap();
    let composition = parse_file(&path).unwrap();
    let mut renderer = Renderer::new(&composition, scale).unwrap();
    let (w, h) = renderer.dimensions();
    Shot {
        pixels: renderer.render_at(0.0).unwrap().to_vec(),
        width: w as usize,
        height: h as usize,
    }
}
