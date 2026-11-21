//! Help screens: the command overview (`aether-motion --help`) and one page per command
//! (`aether-motion help <command>` or `aether-motion <command> --help`).
//!
//! The text lives in `catalog`; this facade lays it out in aligned, wrapped columns sized to the
//! terminal, with section titles and command names styled for stdout.

pub mod catalog;

use super::ui::{term, wrap, Style};
use catalog::{Command, Group, COMMANDS};

/// Version line shared by `--version` and the help header.
pub fn version() -> String {
    format!("AETHER Motion {}", env!("CARGO_PKG_VERSION"))
}

/// Appends `left` padded to `pad` columns followed by `text` wrapped to the terminal width, with
/// continuation lines aligned under the text.
fn columns(out: &mut String, indent: usize, left: &str, styled_left: &str, pad: usize, text: &str) {
    let width = term::width();
    let start = indent + pad + 2;
    let lines = wrap(text, width.saturating_sub(start).max(24));
    let left_len = left.chars().count();
    out.push_str(&" ".repeat(indent));
    out.push_str(styled_left);
    if left_len > pad {
        // A flag too long for the column takes its own line.
        out.push('\n');
        out.push_str(&" ".repeat(start));
    } else {
        out.push_str(&" ".repeat(pad + 2 - left_len));
    }
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
            out.push_str(&" ".repeat(start));
        }
        out.push_str(line);
    }
    out.push('\n');
}

/// Appends wrapped paragraphs at `indent`.
fn paragraph(out: &mut String, indent: usize, text: &str) {
    for line in wrap(text, term::width().saturating_sub(indent)) {
        out.push_str(&" ".repeat(indent));
        out.push_str(&line);
        out.push('\n');
    }
}

/// The overview: what the tool is, the commands by group, and where to read more.
pub fn overview(style: Style) -> String {
    let mut out = String::new();
    out.push_str(&style.bold(&version()));
    out.push('\n');
    out.push_str(
        "Deterministic video from HTML compositions: every frame is a function of time.\n\n",
    );
    out.push_str(&style.accent("Usage"));
    out.push_str("\n  aether-motion <command> [arguments] [options]\n");
    let label = |c: &Command| match c.args {
        "" => c.name.to_string(),
        args => format!("{} {args}", c.name),
    };
    let pad = COMMANDS
        .iter()
        .map(|c| label(c).chars().count())
        .max()
        .unwrap_or(0);
    for group in Group::ALL {
        out.push('\n');
        out.push_str(&style.accent(group.title()));
        out.push('\n');
        for command in COMMANDS.iter().filter(|c| c.group == group) {
            let text = label(command);
            let styled = match command.args {
                "" => style.bold(command.name),
                args => format!("{} {}", style.bold(command.name), style.dim(args)),
            };
            columns(&mut out, 2, &text, &styled, pad, command.summary);
        }
    }
    out.push('\n');
    out.push_str(&style.accent("Options"));
    out.push('\n');
    for (flag, text) in [
        (
            "-h, --help",
            "Show help; `aether-motion help <command>` shows one command",
        ),
        ("-V, --version", "Print the version"),
    ] {
        columns(&mut out, 2, flag, &style.bold(flag), pad, text);
    }
    out.push('\n');
    paragraph(
        &mut out,
        0,
        "Without [file], render, preview, and check use the project: \"entry\" in aether.json (else index.html, else the only .html file here) and its settings. With a file they run standalone and never read aether.json.",
    );
    out.push_str(&format!(
        "Run {} for a command's options and examples.\n",
        style.bold("aether-motion help <command>")
    ));
    out
}

/// The page of one command.
pub fn page(command: &Command, style: Style) -> String {
    let mut out = String::new();
    out.push_str(&style.bold(&format!("aether-motion {}", command.name)));
    out.push_str(&format!(" · {}\n\n", command.summary));
    out.push_str(&style.accent("Usage"));
    out.push('\n');
    for usage in command.usage {
        out.push_str("  ");
        out.push_str(usage);
        out.push('\n');
    }
    if !command.about.is_empty() {
        out.push('\n');
        for (i, text) in command.about.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            paragraph(&mut out, 2, text);
        }
    }
    let flag = |o: &catalog::Opt| match o.value {
        "" => o.flags.to_string(),
        value => format!("{} {value}", o.flags),
    };
    // One column width for every section, so the descriptions line up down the page.
    let pad = command
        .sections
        .iter()
        .flat_map(|s| s.options)
        .map(|o| flag(o).chars().count())
        .max()
        .unwrap_or(0)
        .min(28);
    for section in command.sections {
        out.push('\n');
        out.push_str(&style.accent(section.title));
        out.push('\n');
        for option in section.options {
            let text = flag(option);
            let styled = match option.value {
                "" => style.bold(option.flags),
                value => format!("{} {}", style.bold(option.flags), style.dim(value)),
            };
            columns(&mut out, 2, &text, &styled, pad, option.help);
        }
    }
    if !command.examples.is_empty() {
        out.push('\n');
        out.push_str(&style.accent("Examples"));
        out.push('\n');
        for (line, what) in command.examples {
            out.push_str(&format!("  {}\n", style.dim(&format!("# {what}"))));
            out.push_str(&format!("  {line}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_a_page_and_appears_in_the_overview() {
        let text = overview(Style::plain());
        for command in COMMANDS {
            assert!(text.contains(command.summary), "{} missing", command.name);
            let page = page(command, Style::plain());
            assert!(page.starts_with(&format!("aether-motion {}", command.name)));
            assert!(!command.usage.is_empty(), "{} has no usage", command.name);
        }
        for name in crate::cli::COMMAND_NAMES {
            assert!(catalog::find(name).is_some(), "{name} has no help entry");
        }
    }
}
