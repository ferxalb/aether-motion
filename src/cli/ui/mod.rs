//! Terminal presentation shared by every subcommand: colors, status lines, sections, and wrapping.
//!
//! `style` decides per stream whether ANSI colors are used (a terminal, `NO_COLOR` unset, or
//! `CLICOLOR_FORCE` set) and paints text; `term` measures the terminal width, wraps text, and
//! shortens paths for display; `status` prints the marked lines (`✓`, `✗`, `!`, `+`, ...) and
//! headings that give every command the same layout. Machine-readable output (JSON, HTML, subtitle
//! files, `--progress json`) never goes through this module.

pub mod status;
pub mod style;
pub mod term;

pub use status::{heading, line, rows, Mark};
pub use style::Style;
pub use term::{display_path, wrap};
