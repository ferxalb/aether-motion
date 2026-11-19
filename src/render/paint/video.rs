//! `<video>` painting: fetches the decoded source frame for this instant, fits it to the element's
//! box with `object-fit`, and blits it like an opaque image.
use super::images::{blit, fit_with, CachedImage};
use super::resample::resize_triangle;
use crate::model::VideoSource;
use crate::render::geometry::Rect;
use crate::render::Renderer;
use crate::video::VideoPlayer;
use anyhow::{Context, Result};
use std::collections::hash_map::Entry;

impl<'a> Renderer<'a> {
    /// Draws the source frame recorded in this element's `FrameState` into `rect`.
    pub(in crate::render) fn draw_video(
        &mut self,
        idx: usize,
        source: &VideoSource,
        rect: Rect,
        fit: &str,
        alpha: f32,
    ) -> Result<()> {
        let w = (rect.w * self.scale).round().max(1.0) as u32;
        let h = (rect.h * self.scale).round().max(1.0) as u32;
        let display = self.current[idx].media as usize;
        let player = match self.videos.entry(idx) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(
                VideoPlayer::open(&source.path, source.index.clone())
                    .with_context(|| format!("<video> {}", source.path.display()))?,
            ),
        };
        let frame = player
            .frame(display)
            .with_context(|| format!("<video> {}", source.path.display()))?;
        let exact = frame.dimensions() == (w, h);
        let fitted = if exact {
            frame.clone()
        } else {
            fit_with(frame, w, h, fit, |source, w, h| {
                Ok(resize_triangle(source, w, h))
            })?
        };
        // `fill` and `cover` cover the whole box; `contain` may leave transparent bars.
        let opaque = exact || fit != "contain";
        let x0 = (rect.x * self.scale).round() as i32;
        let y0 = (rect.y * self.scale).round() as i32;
        blit(
            &mut self.surface,
            &CachedImage::frame(fitted, opaque),
            x0,
            y0,
            w,
            h,
            alpha,
        );
        Ok(())
    }
}
