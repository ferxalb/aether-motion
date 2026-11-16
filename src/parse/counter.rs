//! `data-hf-counter`: an animated number. The element becomes a flex row holding one text piece per
//! distinct displayed value, each visible only during its time slice, plus an invisible piece with the
//! final value that sizes the layout. Everything compiles to ordinary motion tracks, so frames stay a
//! pure function of time.
//!
//! Attributes on an empty element: `data-hf-counter` (end value, required); `data-hf-counter-from`
//! (default 0); `-duration` (seconds, default 1.5, at most 10); `-delay` (seconds, default 0);
//! `-decimals` (0 to 6, default 0); `-prefix`, `-suffix`; `-separator` (thousands separator, default
//! `,`; an empty value disables it); `-easing` (default `ease-out-cubic`).
//!
//! The value is sampled 60 times per second and consecutive equal strings are merged.
use crate::model::{Content, Easing, Length, Motion, Pose, Style, Track, TrackPoint};
use crate::parse::animation::timing::parse_keyframe_easing;
use crate::parse::html::{attr, number};
use crate::parse::split::{push_piece, row_container_style};
use crate::parse::tree::Pending;
use anyhow::{bail, Result};
use kuchiki::NodeRef;

/// Samples per second of counter time.
const SAMPLES_PER_SECOND: f64 = 60.0;
const MAX_DURATION: f64 = 10.0;

#[derive(Clone, Debug)]
pub(in crate::parse) struct CounterSpec {
    tag: String,
    from: f64,
    to: f64,
    duration: f64,
    delay: f64,
    decimals: usize,
    prefix: String,
    suffix: String,
    separator: String,
    easing: Easing,
}

/// Reads the counter attributes of `node`; `None` when `data-hf-counter` is absent.
pub(in crate::parse) fn parse_counter(node: &NodeRef, tag: &str) -> Result<Option<CounterSpec>> {
    const OPTIONS: [&str; 8] = [
        "data-hf-counter-from",
        "data-hf-counter-duration",
        "data-hf-counter-delay",
        "data-hf-counter-decimals",
        "data-hf-counter-prefix",
        "data-hf-counter-suffix",
        "data-hf-counter-separator",
        "data-hf-counter-easing",
    ];
    let Some(to) = attr(node, "data-hf-counter") else {
        if let Some(name) = OPTIONS.iter().find(|name| attr(node, name).is_some()) {
            bail!("{name} requires data-hf-counter");
        }
        return Ok(None);
    };
    let num = |name: &str, raw: Option<String>, default: f64| -> Result<f64> {
        let value = raw
            .map(|raw| number(&raw, name))
            .transpose()?
            .unwrap_or(default);
        if !value.is_finite() {
            bail!("{name} must be a finite number");
        }
        Ok(value)
    };
    let to = num("data-hf-counter", Some(to), 0.0)?;
    let from = num(
        "data-hf-counter-from",
        attr(node, "data-hf-counter-from"),
        0.0,
    )?;
    let duration = num(
        "data-hf-counter-duration",
        attr(node, "data-hf-counter-duration"),
        1.5,
    )?;
    let delay = num(
        "data-hf-counter-delay",
        attr(node, "data-hf-counter-delay"),
        0.0,
    )?;
    let decimals = num(
        "data-hf-counter-decimals",
        attr(node, "data-hf-counter-decimals"),
        0.0,
    )?;
    if !(0.05..=MAX_DURATION).contains(&duration) {
        bail!("data-hf-counter-duration must be between 0.05 and {MAX_DURATION} seconds");
    }
    if delay < 0.0 {
        bail!("data-hf-counter-delay must not be negative");
    }
    if decimals.fract() != 0.0 || !(0.0..=6.0).contains(&decimals) {
        bail!("data-hf-counter-decimals must be an integer from 0 to 6");
    }
    if from.abs() > 1e12 || to.abs() > 1e12 {
        bail!("data-hf-counter values must be within 1e12");
    }
    let easing = match attr(node, "data-hf-counter-easing") {
        Some(raw) => parse_keyframe_easing(&raw)?,
        None => parse_keyframe_easing("ease-out-cubic")?,
    };
    Ok(Some(CounterSpec {
        tag: tag.to_string(),
        from,
        to,
        duration,
        delay,
        decimals: decimals as usize,
        prefix: attr(node, "data-hf-counter-prefix").unwrap_or_default(),
        suffix: attr(node, "data-hf-counter-suffix").unwrap_or_default(),
        separator: attr(node, "data-hf-counter-separator").unwrap_or_else(|| ",".to_string()),
        easing,
    }))
}

/// Formats `value` with `decimals` places and `separator` between groups of three integer digits.
pub(in crate::parse) fn format_number(value: f64, decimals: usize, separator: &str) -> String {
    let text = format!("{:.*}", decimals, value.abs());
    let (integer, fraction) = match text.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (text.as_str(), None),
    };
    let mut grouped = String::new();
    for (i, digit) in integer.chars().enumerate() {
        if i > 0 && (integer.len() - i) % 3 == 0 {
            grouped.push_str(separator);
        }
        grouped.push(digit);
    }
    if let Some(fraction) = fraction {
        grouped.push('.');
        grouped.push_str(fraction);
    }
    // A negative value that rounds to zero must not print as "-0".
    let negative = value < 0.0 && text.chars().any(|c| c.is_ascii_digit() && c != '0');
    if negative {
        format!("-{grouped}")
    } else {
        grouped
    }
}

impl CounterSpec {
    fn display(&self, value: f64) -> String {
        format!(
            "{}{}{}",
            self.prefix,
            format_number(value, self.decimals, &self.separator),
            self.suffix
        )
    }

    /// Distinct displayed strings with the clip-relative time each one starts; the first starts at 0.
    fn slices(&self) -> Vec<(f64, String)> {
        let steps = (self.duration * SAMPLES_PER_SECOND).ceil() as usize;
        let mut slices: Vec<(f64, String)> = Vec::new();
        for k in 0..=steps {
            let local = (k as f64 / SAMPLES_PER_SECOND).min(self.duration);
            let progress = self.easing.apply((local / self.duration) as f32) as f64;
            let text = self.display(self.from + (self.to - self.from) * progress);
            if slices.last().is_none_or(|(_, last)| *last != text) {
                let at = if slices.is_empty() {
                    0.0
                } else {
                    self.delay + local
                };
                slices.push((at, text));
            }
        }
        // The last sample is always the exact end value, even if rounding made it equal to the previous one.
        let end = self.display(self.to);
        if slices.last().is_some_and(|(_, last)| *last != end) {
            slices.push((self.delay + self.duration, end));
        }
        slices
    }

    /// Turns the original element into the flex row that holds the pieces.
    pub(in crate::parse) fn apply_container_style(&self, style: &mut Style) {
        row_container_style(style, 0.0);
    }

    /// Appends the sizing piece and one visibility-gated piece per displayed value.
    pub(in crate::parse) fn expand(&self, parent: usize, out: &mut Vec<Pending>) -> Result<()> {
        let slices = self.slices();
        let sizer = Motion {
            base: Pose {
                opacity: 0.0,
                ..Pose::default()
            },
            ..Motion::default()
        };
        push_piece(
            out,
            &self.tag,
            parent,
            Content::Text(self.display(self.to)),
            Style::default(),
            sizer,
        );
        for (i, (start, text)) in slices.iter().enumerate() {
            let end = slices.get(i + 1).map(|(next, _)| *next);
            let style = Style {
                position: Some("absolute".to_string()),
                left: Some(Length::Px(0.0)),
                top: Some(Length::Px(0.0)),
                width: Some(Length::Percent(100.0)),
                ..Style::default()
            };
            let motion = Motion {
                opacity: visibility(*start, end),
                hold_before: true,
                hold_after: true,
                ..Motion::default()
            };
            push_piece(
                out,
                &self.tag,
                parent,
                Content::Text(text.clone()),
                style,
                motion,
            );
        }
        Ok(())
    }
}

/// Opacity 1 on `[start, end)` and 0 elsewhere; a missing `end` keeps the piece visible for good.
pub(in crate::parse) fn visibility(start: f64, end: Option<f64>) -> Track {
    let point = |at: f64, value: f32| TrackPoint {
        at,
        value,
        easing: Easing::Linear,
    };
    let mut points = Vec::with_capacity(4);
    if start > 0.0 {
        points.push(point(start, 0.0));
    }
    points.push(point(start, 1.0));
    if let Some(end) = end {
        points.push(point(end, 1.0));
        points.push(point(end, 0.0));
    }
    if points.len() == 1 {
        // A single piece shown from the start to the end needs no track.
        return Track::default();
    }
    Track(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(from: f64, to: f64, decimals: usize) -> CounterSpec {
        CounterSpec {
            tag: "div".to_string(),
            from,
            to,
            duration: 1.0,
            delay: 0.5,
            decimals,
            prefix: String::new(),
            suffix: "%".to_string(),
            separator: ",".to_string(),
            easing: Easing::Linear,
        }
    }

    #[test]
    fn formats_numbers() {
        assert_eq!(format_number(0.0, 0, ","), "0");
        assert_eq!(format_number(1234567.0, 0, ","), "1,234,567");
        assert_eq!(format_number(999.0, 0, ","), "999");
        assert_eq!(format_number(1000.5, 1, " "), "1 000.5");
        assert_eq!(format_number(-1234.0, 0, ","), "-1,234");
        assert_eq!(format_number(-0.2, 0, ","), "0");
        assert_eq!(format_number(12.0, 2, ""), "12.00");
    }

    #[test]
    fn slices_start_at_zero_and_end_on_the_exact_value() {
        let slices = spec(0.0, 60.0, 0).slices();
        assert_eq!(slices.first().unwrap(), &(0.0, "0%".to_string()));
        assert_eq!(slices.last().unwrap().1, "60%");
        assert!((slices.last().unwrap().0 - 1.5).abs() < 1e-9);
        assert!(slices
            .windows(2)
            .all(|w| w[0].0 < w[1].0 && w[0].1 != w[1].1));
        assert_eq!(slices.len(), 61);
    }

    #[test]
    fn a_constant_counter_has_one_slice() {
        let slices = spec(5.0, 5.0, 0).slices();
        assert_eq!(slices.len(), 1);
        assert!(visibility(0.0, None).0.is_empty());
    }

    #[test]
    fn visibility_gates_a_time_slice() {
        let motion = Motion {
            opacity: visibility(1.0, Some(2.0)),
            hold_before: true,
            hold_after: true,
            ..Motion::default()
        };
        assert_eq!(motion.pose(0.5).opacity, 0.0);
        assert_eq!(motion.pose(1.0).opacity, 1.0);
        assert_eq!(motion.pose(1.99).opacity, 1.0);
        assert_eq!(motion.pose(2.0).opacity, 0.0);
        assert_eq!(motion.pose(9.0).opacity, 0.0);
    }
}
