//! Terminal and machine-readable render progress for the CLI (`--progress auto|json|off`).
//!
//! `auto` redraws one status line on stderr when stderr is a terminal (a bar, the percentage, frames,
//! speed, and ETA) and prints nothing otherwise,
//! so piped output and logs stay clean; `json` writes one JSON object per update on stderr for
//! scripts and job runners; `off` disables reporting. Progress never changes the rendered output.

use super::ui::Style;
use crate::encode::{ProgressObserver, ProgressUpdate};
use anyhow::{bail, Result};
use std::io::{IsTerminal, Write};

/// How a render reports progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProgressMode {
    /// A status line on an interactive stderr; silent otherwise.
    #[default]
    Auto,
    /// JSON lines on stderr: `{"done":N,"total":N,"fraction":F,"fps":F,"elapsedMs":N,"etaMs":N|null}`.
    Json,
    Off,
}

impl std::str::FromStr for ProgressMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "json" => Ok(Self::Json),
            "off" => Ok(Self::Off),
            _ => bail!("--progress must be auto, json, or off"),
        }
    }
}

/// The observer that implements a [`ProgressMode`] on stderr.
pub struct StderrProgress {
    json: bool,
}

impl StderrProgress {
    /// The observer for `mode`, or `None` when nothing should be reported.
    pub fn new(mode: ProgressMode) -> Option<Self> {
        match mode {
            ProgressMode::Off => None,
            ProgressMode::Json => Some(Self { json: true }),
            ProgressMode::Auto => std::io::stderr()
                .is_terminal()
                .then_some(Self { json: false }),
        }
    }

    /// Ends the status line so later output starts on a fresh line.
    pub fn finish(&self) {
        if !self.json {
            eprintln!();
        }
    }
}

/// Runs an export with the stderr reporter of `mode`, ending its status line afterwards.
pub fn with_progress<T>(
    mode: ProgressMode,
    export: impl FnOnce(Option<&dyn ProgressObserver>) -> Result<T>,
) -> Result<T> {
    let reporter = StderrProgress::new(mode);
    let result = export(reporter.as_ref().map(|r| r as &dyn ProgressObserver));
    if let Some(reporter) = &reporter {
        reporter.finish();
    }
    result
}

/// Width of the terminal progress bar in cells.
const BAR: usize = 24;

/// The terminal status line for one update, such as
/// `  ━━━━━━━━━━━━────────────  50%  45/90 frames · 30.0 fps · ETA 1.5 s`.
pub fn bar_line(update: &ProgressUpdate, style: Style) -> String {
    let fraction = update.fraction().clamp(0.0, 1.0);
    let filled = ((fraction * BAR as f64).round() as usize).min(BAR);
    let eta = match update.eta() {
        Some(eta) if update.done < update.total => {
            format!(" · ETA {}", duration(eta.as_secs_f64()))
        }
        _ => String::new(),
    };
    format!(
        "  {}{} {:>3.0}%  {}",
        style.cyan(&"━".repeat(filled)),
        style.dim(&"─".repeat(BAR - filled)),
        fraction * 100.0,
        style.dim(&format!(
            "{}/{} frames · {:.1} fps{eta}",
            update.done,
            update.total,
            update.frames_per_second()
        ))
    )
}

/// `42.0 s`, `3 m 05 s`, or `1 h 02 m`.
fn duration(seconds: f64) -> String {
    let whole = seconds.round() as u64;
    match whole {
        0..=59 => format!("{seconds:.1} s"),
        60..=3599 => format!("{} m {:02} s", whole / 60, whole % 60),
        _ => format!("{} h {:02} m", whole / 3600, whole / 60 % 60),
    }
}

/// The JSON line for one update.
pub fn json_line(update: &ProgressUpdate) -> String {
    serde_json::json!({
        "done": update.done,
        "total": update.total,
        "fraction": (update.fraction() * 1e4).round() / 1e4,
        "fps": (update.frames_per_second() * 10.0).round() / 10.0,
        "elapsedMs": update.elapsed.as_millis() as u64,
        "etaMs": update.eta().map(|eta| eta.as_millis() as u64),
    })
    .to_string()
}

impl ProgressObserver for StderrProgress {
    fn update(&self, update: ProgressUpdate) {
        let mut stderr = std::io::stderr().lock();
        let _ = if self.json {
            writeln!(stderr, "{}", json_line(&update))
        } else {
            // `\x1b[K` clears what remains of a longer previous line.
            write!(stderr, "\r{}\x1b[K", bar_line(&update, Style::err()))
        };
        let _ = stderr.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn modes_parse_and_json_lines_are_stable() {
        assert_eq!("json".parse::<ProgressMode>().unwrap(), ProgressMode::Json);
        assert!("bar".parse::<ProgressMode>().is_err());
        assert!(StderrProgress::new(ProgressMode::Off).is_none());
        let update = ProgressUpdate {
            done: 45,
            total: 90,
            elapsed: Duration::from_millis(1500),
        };
        assert_eq!(
            bar_line(&update, Style::plain()),
            "  ━━━━━━━━━━━━────────────  50%  45/90 frames · 30.0 fps · ETA 1.5 s"
        );
        assert_eq!(duration(185.0), "3 m 05 s");
        assert_eq!(duration(3720.0), "1 h 02 m");
        assert_eq!(
            json_line(&update),
            r#"{"done":45,"elapsedMs":1500,"etaMs":1500,"fps":30.0,"fraction":0.5,"total":90}"#
        );
    }
}
