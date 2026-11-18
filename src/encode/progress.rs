//! Render progress reporting: a [`ProgressObserver`] receives throttled [`ProgressUpdate`]s (frames
//! encoded, total, elapsed time, throughput, and an ETA) while an export runs.
//!
//! Progress is observation only: the frames, their order, and the encoded bytes never depend on it.
//! The internal `Tracker` counts finished frames atomically, so the parallel segment workers report
//! through it directly, and it limits callbacks to one per [`REPORT_INTERVAL`] (plus the final frame),
//! so observers can print or send a message on every call.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Minimum time between two reports; the last frame is always reported.
pub const REPORT_INTERVAL: Duration = Duration::from_millis(250);

/// A snapshot of an export's progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressUpdate {
    /// Frames encoded so far.
    pub done: usize,
    /// Frames in the whole export (or part).
    pub total: usize,
    /// Time since the export started.
    pub elapsed: Duration,
}

impl ProgressUpdate {
    /// Finished share of the export, 0 to 1.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 1.0;
        }
        (self.done as f64 / self.total as f64).min(1.0)
    }

    /// Frames encoded per second of wall time so far.
    pub fn frames_per_second(&self) -> f64 {
        let seconds = self.elapsed.as_secs_f64();
        if seconds <= 0.0 {
            return 0.0;
        }
        self.done as f64 / seconds
    }

    /// Estimated time left at the throughput so far, or `None` before the first frame.
    pub fn eta(&self) -> Option<Duration> {
        if self.done == 0 {
            return None;
        }
        let left = self.total.saturating_sub(self.done) as f64;
        Some(Duration::from_secs_f64(
            self.elapsed.as_secs_f64() * left / self.done as f64,
        ))
    }

    /// One-line human summary, for example `42% 126/300 frames, 31.5 fps, ETA 5.5 s`.
    pub fn summary(&self) -> String {
        let eta = match self.eta() {
            Some(eta) => format!(", ETA {:.1} s", eta.as_secs_f64()),
            None => String::new(),
        };
        format!(
            "{:>3.0}% {}/{} frames, {:.1} fps{eta}",
            self.fraction() * 100.0,
            self.done,
            self.total,
            self.frames_per_second()
        )
    }
}

/// Receives progress while an export runs. Called from rendering threads, at most once per
/// [`REPORT_INTERVAL`] and always for the last frame, never concurrently.
pub trait ProgressObserver: Sync {
    fn update(&self, update: ProgressUpdate);
}

/// Any `Fn(ProgressUpdate) + Sync` closure is an observer.
impl<F: Fn(ProgressUpdate) + Sync> ProgressObserver for F {
    fn update(&self, update: ProgressUpdate) {
        self(update)
    }
}

/// Counts finished frames for one export and forwards throttled updates to its observer.
pub(super) struct Tracker<'a> {
    observer: Option<&'a dyn ProgressObserver>,
    total: usize,
    done: AtomicUsize,
    start: Instant,
    /// Time and frame count of the last report; the lock also keeps observer calls from
    /// overlapping.
    last: Mutex<(Option<Instant>, usize)>,
}

impl<'a> Tracker<'a> {
    pub fn new(observer: Option<&'a dyn ProgressObserver>, total: usize) -> Self {
        Self {
            observer,
            total,
            done: AtomicUsize::new(0),
            start: Instant::now(),
            last: Mutex::new((None, 0)),
        }
    }

    /// Records one encoded frame.
    pub fn frame_done(&self) {
        let Some(observer) = self.observer else {
            return;
        };
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        if done > self.total {
            return;
        }
        let now = Instant::now();
        let final_frame = done >= self.total;
        let Ok(mut last) = (if final_frame {
            self.last.lock().map_err(|_| ())
        } else {
            self.last.try_lock().map_err(|_| ())
        }) else {
            // Another thread is reporting right now; this frame shows up in the next report.
            return;
        };
        if !final_frame
            && last
                .0
                .is_some_and(|at| now.duration_since(at) < REPORT_INTERVAL)
        {
            return;
        }
        // Re-read under the lock so reports never go backwards, and never repeat a count (a
        // throttled report can already have seen every frame before the last one reports).
        let done = self.done.load(Ordering::Relaxed).min(self.total);
        if last.0.is_some() && done <= last.1 {
            return;
        }
        *last = (Some(now), done);
        observer.update(ProgressUpdate {
            done,
            total: self.total,
            elapsed: now.duration_since(self.start),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn reports_are_throttled_and_end_with_the_last_frame() {
        let seen = Mutex::new(Vec::new());
        let observer = |update: ProgressUpdate| seen.lock().unwrap().push(update.done);
        let tracker = Tracker::new(Some(&observer), 1000);
        for _ in 0..1000 {
            tracker.frame_done();
        }
        let seen = seen.into_inner().unwrap();
        // The first frame reports immediately; the rest of a fast loop is throttled away.
        assert_eq!(seen.first(), Some(&1));
        assert_eq!(seen.last(), Some(&1000));
        assert!(seen.len() < 10, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn parallel_workers_report_every_frame_once() {
        let last = Mutex::new(0);
        let observer = |update: ProgressUpdate| {
            let mut last = last.lock().unwrap();
            assert!(update.done > *last, "{} after {}", update.done, *last);
            *last = update.done;
        };
        let tracker = Tracker::new(Some(&observer), 4000);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| (0..1000).for_each(|_| tracker.frame_done()));
            }
        });
        assert_eq!(*last.lock().unwrap(), 4000);
    }

    #[test]
    fn update_arithmetic() {
        let update = ProgressUpdate {
            done: 30,
            total: 120,
            elapsed: Duration::from_secs(2),
        };
        assert_eq!(update.fraction(), 0.25);
        assert_eq!(update.frames_per_second(), 15.0);
        assert_eq!(update.eta(), Some(Duration::from_secs(6)));
        assert_eq!(update.summary(), " 25% 30/120 frames, 15.0 fps, ETA 6.0 s");
        let start = ProgressUpdate { done: 0, ..update };
        assert_eq!(start.eta(), None);
    }
}
