//! ANSI styling that switches itself off when the stream is not a terminal.
//!
//! Colors are on when the stream is a terminal, `NO_COLOR` is unset or empty, and `TERM` is not
//! `dumb`; `CLICOLOR_FORCE` (any value other than `0`) turns them on regardless. The decision is
//! made once per stream. Styling never changes the text itself, so piped output reads the same
//! without escape codes.

use std::io::IsTerminal;
use std::sync::OnceLock;

/// Painter for one output stream.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    enabled: bool,
}

fn decide(is_terminal: bool) -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    if std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| !v.is_empty() && v != "0") {
        return true;
    }
    is_terminal && !set("NO_COLOR") && std::env::var("TERM").map_or(true, |t| t != "dumb")
}

impl Style {
    /// The painter for stdout.
    pub fn out() -> Self {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        Self {
            enabled: *ENABLED.get_or_init(|| decide(std::io::stdout().is_terminal())),
        }
    }

    /// The painter for stderr.
    pub fn err() -> Self {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        Self {
            enabled: *ENABLED.get_or_init(|| decide(std::io::stderr().is_terminal())),
        }
    }

    /// A painter that never emits escape codes (tests, plain text).
    pub const fn plain() -> Self {
        Self { enabled: false }
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }

    pub fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }

    pub fn green(self, text: &str) -> String {
        self.paint("32", text)
    }

    pub fn red(self, text: &str) -> String {
        self.paint("31", text)
    }

    pub fn yellow(self, text: &str) -> String {
        self.paint("33", text)
    }

    pub fn cyan(self, text: &str) -> String {
        self.paint("36", text)
    }

    /// Bold cyan: command names and section titles in help.
    pub fn accent(self, text: &str) -> String {
        self.paint("1;36", text)
    }
}

#[cfg(test)]
mod tests {
    use super::Style;

    #[test]
    fn plain_style_leaves_text_untouched() {
        let s = Style::plain();
        assert_eq!(s.bold("a"), "a");
        assert_eq!(s.red("b"), "b");
        let on = Style { enabled: true };
        assert_eq!(on.green("ok"), "\x1b[32mok\x1b[0m");
    }
}
