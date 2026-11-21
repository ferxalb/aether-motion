//! Marked status lines, section headings, and aligned key/value rows.
//!
//! Every human-facing command reports through these, so a result reads the same everywhere:
//! a two-space indent, a one-character mark, the subject, then dimmed detail.

use super::style::Style;

/// The mark that opens a status line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Success or a passing check.
    Ok,
    /// A failure that needs action.
    Fail,
    /// A warning or a skipped step.
    Warn,
    /// Neutral information.
    Info,
    /// A file that was created.
    Added,
    /// A file that was changed.
    Changed,
    /// Nothing to do.
    Same,
}

impl Mark {
    fn symbol(self, style: Style) -> String {
        match self {
            Mark::Ok => style.green("✓"),
            Mark::Fail => style.red("✗"),
            Mark::Warn => style.yellow("!"),
            Mark::Info => style.cyan("•"),
            Mark::Added => style.green("+"),
            Mark::Changed => style.yellow("~"),
            Mark::Same => style.dim("="),
        }
    }
}

/// `  ✓ subject  detail`, with the detail dimmed; an empty detail is left out.
pub fn line(style: Style, mark: Mark, subject: &str, detail: &str) -> String {
    let mut out = format!("  {} {subject}", mark.symbol(style));
    if !detail.is_empty() {
        out.push_str("  ");
        out.push_str(&style.dim(detail));
    }
    out
}

/// A bold section title.
pub fn heading(style: Style, title: &str) -> String {
    style.bold(title)
}

/// Rows of `key  value`, keys dimmed and padded to the longest one, indented by `indent` spaces.
pub fn rows(style: Style, indent: usize, pairs: &[(&str, String)]) -> Vec<String> {
    let width = pairs
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    pairs
        .iter()
        .map(|(key, value)| {
            let pad = " ".repeat(width - key.chars().count());
            format!("{}{}{pad}  {value}", " ".repeat(indent), style.dim(key))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_rows_align() {
        let s = Style::plain();
        assert_eq!(
            line(s, Mark::Ok, "index.html", "valid"),
            "  ✓ index.html  valid"
        );
        assert_eq!(line(s, Mark::Added, "a.json", ""), "  + a.json");
        assert_eq!(
            rows(
                s,
                4,
                &[("Canvas", "1920x1080".into()), ("Fps", "30".into())]
            ),
            ["    Canvas  1920x1080", "    Fps     30"]
        );
    }
}
