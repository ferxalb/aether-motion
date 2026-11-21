//! Terminal measurements and text helpers: output width, word wrapping, and display paths.

use std::path::{Component, Path};

/// Width used to wrap help and lists: the terminal's columns (stdout), else `COLUMNS`, else 100,
/// kept between 60 and 110 so lines stay readable on very narrow or very wide windows.
pub fn width() -> usize {
    let columns = terminal_columns()
        .or_else(|| std::env::var("COLUMNS").ok()?.trim().parse().ok())
        .unwrap_or(100);
    columns.clamp(60, 110)
}

#[cfg(unix)]
fn terminal_columns() -> Option<usize> {
    // SAFETY: TIOCGWINSZ fills a `winsize` for a terminal file descriptor and fails otherwise;
    // the struct is plain data and zero-initialized before the call.
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        (libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_col > 0)
            .then_some(size.ws_col as usize)
    }
}

#[cfg(not(unix))]
fn terminal_columns() -> Option<usize> {
    None
}

/// Splits `text` into lines of at most `width` characters at spaces (a longer word keeps a line
/// of its own). Always returns at least one line.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let fits = current.chars().count() + 1 + word.chars().count() <= width.max(1);
        if !current.is_empty() && !fits {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    lines.push(current);
    lines
}

/// `path` without leading `./` components, for messages (`./index.html` reads `index.html`).
pub fn display_path(path: &Path) -> String {
    let trimmed: std::path::PathBuf = path
        .components()
        .skip_while(|c| matches!(c, Component::CurDir))
        .collect();
    if trimmed.as_os_str().is_empty() {
        ".".to_string()
    } else {
        trimmed.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_spaces_and_trims_current_dir() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("averyverylongword x", 5), ["averyverylongword", "x"]);
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(display_path(Path::new("./index.html")), "index.html");
        assert_eq!(display_path(Path::new("./out/a.mp4")), "out/a.mp4");
        assert_eq!(display_path(Path::new(".")), ".");
    }
}
