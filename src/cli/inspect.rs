//! `inspect <file>`: the resolved canvas, segment plan, audio, timed elements, and native physics
//! launch diagnostics as aligned tables. A named file is read as a standalone composition.

use super::args::usage;
use super::options::{self, Options};
use super::ui::{self, display_path, Style};
use crate::encode::segment_count;
use crate::model::{Composition, Content};
use crate::parse::parse_file_with;
use anyhow::Result;

pub fn run(raw: &[String]) -> Result<()> {
    let opts = options::parse("inspect", raw)?;
    let values = super::variables::values(
        opts.variables.as_deref(),
        opts.variables_file.as_deref(),
        &Default::default(),
    )
    .map_err(|e| usage(format!("{e:#}")))?;
    // `options::parse` requires the file for inspect.
    let input = opts
        .input
        .as_deref()
        .ok_or_else(|| usage("inspect needs a composition"))?;
    let composition = parse_file_with(input, &values)?;
    inspect(&composition, &opts, input);
    Ok(())
}

/// Seconds with three decimals, the precision of clip times.
fn seconds(value: f64) -> String {
    format!("{value:.3} s")
}

/// `inspect`: canvas, frame plan, audio clips, and the elements visible for part of the timeline.
fn inspect(composition: &Composition, opts: &Options, input: &std::path::Path) {
    let style = Style::out();
    println!("{}", style.bold(&display_path(input)));
    let mut pairs = vec![
        (
            "Canvas",
            format!("{}x{}", composition.width, composition.height),
        ),
        ("Duration", seconds(composition.duration)),
        ("Elements", composition.elements.len().to_string()),
    ];
    if let Some(fps) = opts.fps {
        let frames = (composition.duration * fps as f64).ceil() as usize;
        pairs.push(("Frames", format!("{frames} @ {fps} fps")));
        pairs.push((
            "Segments",
            segment_count(frames, fps).map_or_else(
                || "none (renders in one piece)".to_string(),
                |n| format!("{n} (one second each; render parts with --segments A..B)"),
            ),
        ));
    }
    for row in ui::rows(style, 2, &pairs) {
        println!("{row}");
    }
    if !composition.audio.is_empty() {
        let rows: Vec<[String; 4]> = composition
            .audio
            .iter()
            .map(|clip| {
                let range = match clip.duration {
                    Some(duration) => {
                        format!("{:.3}..{:.3} s", clip.start, clip.start + duration)
                    }
                    None => format!("{:.3} s..source end", clip.start),
                };
                [
                    clip.id
                        .clone()
                        .unwrap_or_else(|| clip.src.display().to_string()),
                    range,
                    format!("{}", clip.volume),
                    if clip.looped { "loop" } else { "" }.to_string(),
                ]
            })
            .collect();
        println!("\n{}", style.bold("Audio"));
        table(style, ["Clip", "Time", "Volume", ""], &rows);
    }
    let timed: Vec<[String; 4]> = composition
        .elements
        .iter()
        .filter(|e| e.parent.is_some() && (e.end - e.start) < composition.duration)
        .map(|e| {
            [
                e.id.as_deref().unwrap_or("(no id)").to_string(),
                format!("<{}>", e.tag),
                format!("{:.3}..{:.3} s", e.start, e.end),
                e.track.to_string(),
            ]
        })
        .collect();
    if !timed.is_empty() {
        println!("\n{}", style.bold("Timed elements"));
        table(style, ["Id", "Tag", "Time", "Track"], &timed);
    }
    inspect_physics(composition, style);
}

fn inspect_physics(composition: &Composition, style: Style) {
    for element in &composition.elements {
        let Content::Scene3d(scene) = &element.content else {
            continue;
        };
        let Some(bake) = &scene.physics else { continue };
        let mut rows = Vec::new();
        let vector = |[x, y, z]: [f64; 3]| format!("{x:.3}, {y:.3}, {z:.3}");
        for (index, node) in scene.nodes.iter().enumerate() {
            let Some(launch) = bake.launch(index) else {
                continue;
            };
            let body = node.id.clone().unwrap_or_else(|| format!("node {index}"));
            for (label, velocity) in [
                ("Linear (px/s)", &launch.linear),
                ("Angular (deg/s)", &launch.angular),
            ] {
                rows.push([
                    body.clone(),
                    label.into(),
                    vector(velocity.requested),
                    vector(velocity.actual),
                    format!("{:.3}", velocity.limit),
                    velocity
                        .first_tick_affected
                        .map_or_else(|| "–".into(), |tick| tick.to_string()),
                ]);
            }
        }
        if !rows.is_empty() {
            let viewport = element.id.as_deref().unwrap_or("(no id)");
            println!(
                "\n{}",
                style.bold(&format!("Physics launch velocities — {viewport}"))
            );
            table(
                style,
                [
                    "Body",
                    "Quantity",
                    "Requested (x, y, z)",
                    "Actual at tick 1 (x, y, z)",
                    "Speed cap",
                    "First cap tick",
                ],
                &rows,
            );
        }
    }
}

/// Prints `rows` under a dimmed header, each column padded to its widest cell.
fn table<const N: usize>(style: Style, header: [&str; N], rows: &[[String; N]]) {
    let mut widths = header.map(|h| h.chars().count());
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: [&str; N]| {
        let text = cells
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect::<Vec<_>>()
            .join("  ");
        format!("  {}", text.trim_end())
    };
    println!("{}", style.dim(&line(header)));
    for row in rows {
        println!("{}", line(std::array::from_fn(|i| row[i].as_str())));
    }
}
