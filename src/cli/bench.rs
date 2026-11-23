//! `bench <file>`: draws frames on one renderer without encoding and reports the time per frame.
//! Like every command that names a file, it never reads `aether.json`.

use super::args::usage;
use super::options::{self, Options};
use super::render::backend_note;
use super::ui::{self, display_path, Mark, Style};
use crate::parse::parse_file_with;
use crate::render::Renderer;
use anyhow::Result;
use std::time::Instant;

pub fn run(raw: &[String]) -> Result<()> {
    let opts = options::parse("bench", raw)?;
    let values = super::variables::values(
        opts.variables.as_deref(),
        opts.variables_file.as_deref(),
        &Default::default(),
    )
    .map_err(|e| usage(format!("{e:#}")))?;
    // `options::parse` requires the file for bench.
    let input = opts
        .input
        .as_deref()
        .ok_or_else(|| usage("bench needs a composition"))?;
    let composition = parse_file_with(input, &values)?;
    let mut renderer = match opts.size {
        Some(size) => Renderer::with_output_size(&composition, size)?,
        None => Renderer::new(&composition, opts.scale.unwrap_or(1.0))?,
    };
    if opts.low_priority {
        crate::encode::lower_priority();
    }
    bench(&mut renderer, &opts, input)
}

/// `bench`: draws the requested frames and reports the time per frame.
fn bench(renderer: &mut Renderer<'_>, opts: &Options, input: &std::path::Path) -> Result<()> {
    let fps = opts.fps.unwrap_or(30);
    let total = renderer.frame_count(fps);
    let first = opts.frame.unwrap_or(0);
    if first >= total {
        return Err(usage(format!(
            "--frame {first} is outside the composition ({total} frames, 0 to {})",
            total.saturating_sub(1)
        )));
    }
    let last = opts.count.map_or(total, |n| first + n.min(total - first));
    let start = Instant::now();
    for i in first..last {
        renderer.render_frame(i, fps)?;
    }
    let elapsed = start.elapsed().as_secs_f64();
    crate::profile::report();
    backend_note(Some(renderer));
    let style = Style::out();
    let (w, h) = renderer.dimensions();
    let drawn = last - first;
    println!(
        "{}",
        ui::line(
            style,
            Mark::Ok,
            &format!("Benchmarked {}", display_path(input)),
            &format!("{w}x{h} · {fps} fps"),
        )
    );
    let stats = renderer.stats();
    let per_frame_ms = elapsed * 1000.0 / drawn.max(1) as f64;
    let precision = if per_frame_ms < 1.0 { 3 } else { 1 };
    for row in ui::rows(
        style,
        4,
        &[
            ("Frames", format!("{drawn} ({first}..{last} of {total})")),
            (
                "Time",
                format!(
                    "{elapsed:.3} s · {per_frame_ms:.precision$} ms per frame · {:.1} frames/s",
                    if elapsed > 0.0 {
                        drawn as f64 / elapsed
                    } else {
                        0.0
                    }
                ),
            ),
            (
                "Drawing",
                format!("{} rasterized · {} reused", stats.rasterized, stats.reused),
            ),
        ],
    ) {
        println!("{row}");
    }
    Ok(())
}
