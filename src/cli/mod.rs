//! The command-line interface: dispatch, help, error reporting, and every subcommand.
//!
//! [`main`] is the whole CLI behind `src/main.rs`: it answers `--help`, `help <command>`,
//! `<command> --help`, and `--version`, runs the command, and prints errors in one format
//! (`error: ...` with a hint; exit code 2 for usage errors, 1 for everything else).
//!
//! Commands, one file each: `init`, `preview`, `check_cmd`, `add`, `block_cmd`, `captions_cmd`,
//! `agent_cmd`, `doctor`, `inspect`, `bench`, and `render/` (project mode without a file, direct
//! mode with one; its flags, shared with `inspect` and `bench`, are parsed once in `options`).
//! Shared pieces: `args` (parsing and [`args::UsageError`]), `suggest` ("did you mean"), `help`
//! (the command catalog and its layout), `ui` (colors, status lines, wrapping), `project`
//! (`aether.json` and entry resolution), `variables` (`--variables` values), and `progress`
//! (`--progress` reporting on stderr). `setup/` holds the agent wiring that `init` and `agent` use.

mod add;
mod agent_cmd;
pub mod args;
mod bench;
mod block_cmd;
mod captions_cmd;
mod check_cmd;
mod doctor;
pub mod help;
mod init;
mod inspect;
mod options;
mod preview;
pub mod progress;
pub mod project;
pub mod render;
mod suggest;
pub mod ui;
pub mod variables;

use anyhow::Result;
use args::UsageError;
use ui::Style;

/// Every command `main` dispatches, in overview order.
pub const COMMAND_NAMES: [&str; 12] = [
    "init", "preview", "render", "check", "add", "block", "captions", "agent", "mcp", "doctor",
    "inspect", "bench",
];

/// Runs the CLI on `args` (without the program name) and returns the process exit code.
pub fn main(args: &[String]) -> i32 {
    let Some((command, rest)) = args.split_first() else {
        restore_sigpipe();
        print!("{}", help::overview(Style::out()));
        return 0;
    };
    if matches!(
        command.as_str(),
        "-h" | "--help" | "help" | "-V" | "--version" | "version"
    ) {
        restore_sigpipe();
    }
    match command.as_str() {
        "-h" | "--help" => {
            print!("{}", help::overview(Style::out()));
            return 0;
        }
        "help" => return help_for(rest.first().map(String::as_str)),
        "-V" | "--version" | "version" => {
            println!("{}", help::version());
            return 0;
        }
        _ => {}
    }
    if !COMMAND_NAMES.contains(&command.as_str()) {
        let error = UsageError::new(format!("unknown command: {command}"));
        let error = match suggest::closest(command, COMMAND_NAMES) {
            Some(near) => error.with_hint(format!("did you mean `aether-motion {near}`?")),
            None => error,
        };
        return print_error(&error.into(), None);
    }
    // Printing commands end quietly when their reader goes away (`... | head`), as Unix tools do.
    // The MCP server and renders keep the default, so a closed pipe stays a reported error there.
    if !matches!(command.as_str(), "mcp" | "render") {
        restore_sigpipe();
    }
    // `-h` and `--help` anywhere after the command show its page instead of running it.
    if rest.iter().any(|a| a == "-h" || a == "--help") {
        return help_for(Some(command));
    }
    match run(command, rest) {
        Ok(code) => code,
        Err(error) => print_error(&error, Some(command)),
    }
}

/// Runs one command; `Ok` carries its exit code.
fn run(command: &str, rest: &[String]) -> Result<i32> {
    let done = |r: Result<()>| r.map(|()| 0);
    match command {
        "init" => done(init::run(rest)),
        "preview" => done(preview::run(rest)),
        "add" => done(add::run(rest)),
        "block" => done(block_cmd::run(rest)),
        "captions" => done(captions_cmd::run(rest)),
        "agent" => done(agent_cmd::run(rest)),
        "doctor" => done(doctor::run()),
        "mcp" => done(crate::agent::serve()),
        // `check` prints JSON when piped, so scripts and agents keep the machine-readable report.
        "check" => check_cmd::run(rest),
        // Project mode without a file, direct mode with one; see `render/mod.rs`.
        "render" => done(render::run(rest)),
        "inspect" => done(inspect::run(rest)),
        "bench" => done(bench::run(rest)),
        _ => unreachable!("dispatched commands are listed in COMMAND_NAMES"),
    }
}

/// Restores the default `SIGPIPE` action (Rust ignores it), so a closed stdout ends the process
/// instead of panicking in `println!`.
fn restore_sigpipe() {
    #[cfg(unix)]
    // SAFETY: resetting a signal to its default action has no preconditions.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// `help [command]`: the overview, or one command's page.
fn help_for(name: Option<&str>) -> i32 {
    let Some(name) = name else {
        print!("{}", help::overview(Style::out()));
        return 0;
    };
    match help::catalog::find(name) {
        Some(command) => {
            print!("{}", help::page(command, Style::out()));
            0
        }
        None => {
            let error = UsageError::new(format!("no help for unknown command: {name}"));
            let error = match suggest::closest(name, COMMAND_NAMES) {
                Some(near) => error.with_hint(format!("did you mean `aether-motion help {near}`?")),
                None => error,
            };
            print_error(&error.into(), None)
        }
    }
}

/// Prints `error` on stderr and returns its exit code: 2 for a [`UsageError`] (with its hint and a
/// pointer to the help of `command`), 1 otherwise.
pub fn print_error(error: &anyhow::Error, command: Option<&str>) -> i32 {
    let style = Style::err();
    let label = style.red(&style.bold("error:"));
    match error.downcast_ref::<UsageError>() {
        Some(usage) => {
            eprintln!("{label} {}", usage.message);
            if let Some(hint) = &usage.hint {
                eprintln!("  {} {hint}", style.cyan("hint:"));
            }
            let help = match command {
                Some(command) => format!("aether-motion {command} --help"),
                None => "aether-motion --help".to_string(),
            };
            eprintln!("  {} {}", style.dim("see:"), style.dim(&help));
            2
        }
        None => {
            eprintln!("{label} {error:#}");
            1
        }
    }
}
