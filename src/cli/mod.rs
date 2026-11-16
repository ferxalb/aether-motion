//! Human-facing subcommands: `init`, `preview`, `render`, `check`, `add`, `captions`, `agent`, and `doctor`.
//!
//! `run` is the single entry point used by `main`; it returns `None` for commands it does not own
//! (the legacy flag-based forms stay in `main.rs`). Shared pieces: `args` (parsing), `project`
//! (`aether.json` and entry resolution), and `variables` (`--variables` values). `setup/` holds the agent wiring that `init` and `agent` use.

mod add;
mod agent_cmd;
mod args;
mod captions_cmd;
mod check_cmd;
mod doctor;
mod init;
mod preview;
pub mod project;
pub mod render_cmd;
pub mod variables;

use anyhow::Result;

/// Runs a short command; `None` means "not mine, use the legacy path". `Some(Ok(code))` is the exit code.
pub fn run(raw: &[String]) -> Option<Result<i32>> {
    let (command, rest) = raw.split_first()?;
    let done = |r: Result<()>| Some(r.map(|()| 0));
    match command.as_str() {
        "init" => done(init::run(rest)),
        "preview" => done(preview::run(rest)),
        "add" => done(add::run(rest)),
        "captions" => done(captions_cmd::run(rest)),
        "agent" => done(agent_cmd::run(rest)),
        "doctor" => done(doctor::run()),
        // `check` prints JSON when piped, so scripts and agents keep the legacy output.
        "check" => Some(check_cmd::run(rest)),
        // `render <file> --output <out>` keeps its legacy flag-based path in `main.rs`.
        "render" if rest.iter().any(|a| a == "--output") => None,
        "render" => done(render_cmd::run(rest)),
        _ => None,
    }
}
