//! `batch`: runs many commands in one long-lived process, one JSON job per stdin line.
//!
//! Each line is `{"id": <any>, "args": ["render", "promo.html", "-o", "out.mp4", ...], "cwd":
//! "<dir>"}` (`cwd` optional). The job runs through the same dispatcher as a separate
//! `aether-motion` process, so every command, flag, and error keeps its meaning; what the process
//! keeps between jobs is what is built once per process (the system font index, the CPU
//! dispatch decisions). Jobs run one after another. When a job ends, one line goes to stdout:
//! `{"id", "exit", "stdout", "elapsedMs", "peakRssBytes"}`, where `stdout` is what the command
//! printed (at most 4 MiB; it is captured, so it never mixes with job lines). Stderr passes
//! through unchanged, including `--progress json` updates. Freed heap memory is returned to the OS
//! after each job (`system::trim_heap`). Unix only, because capturing a command's output
//! redirects file descriptor 1.

use super::args::usage;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// Commands a job may run. Interactive and server commands are left out.
const ALLOWED: &[&str] = &["render", "check", "inspect", "bench", "block", "captions"];

/// Largest captured stdout returned in a job line.
const MAX_STDOUT: u64 = 4 * 1024 * 1024;

pub fn run(rest: &[String]) -> Result<()> {
    if let Some(extra) = rest.first() {
        return Err(usage(format!("batch takes no arguments: {extra}")));
    }
    #[cfg(unix)]
    {
        unix::serve()
    }
    #[cfg(not(unix))]
    {
        bail!("batch is available on Linux and macOS")
    }
}

/// Validates one job line and returns its id, arguments, and working directory.
fn parse_job(line: &str) -> Result<(Value, Vec<String>, Option<String>)> {
    let job: Value = serde_json::from_str(line).context("job is not JSON")?;
    let id = job.get("id").cloned().unwrap_or(Value::Null);
    let Some(args) = job.get("args").and_then(Value::as_array) else {
        bail!("job needs an \"args\" array of strings");
    };
    let args = args
        .iter()
        .map(|a| a.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()
        .context("job \"args\" must all be strings")?;
    match args.first() {
        Some(command) if ALLOWED.contains(&command.as_str()) => {}
        Some(command) => bail!(
            "batch cannot run `{command}`; allowed: {}",
            ALLOWED.join(", ")
        ),
        None => bail!("job \"args\" is empty"),
    }
    let cwd = match job.get("cwd") {
        None | Some(Value::Null) => None,
        Some(Value::String(dir)) => Some(dir.clone()),
        Some(_) => bail!("job \"cwd\" must be a string"),
    };
    Ok((id, args, cwd))
}

fn result_line(id: Value, exit: i32, stdout: String, elapsed: std::time::Duration) -> Value {
    json!({
        "id": id,
        "exit": exit,
        "stdout": stdout,
        "elapsedMs": elapsed.as_millis() as u64,
        "peakRssBytes": crate::system::peak_rss(),
    })
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::fs::File;
    use std::io::{Read, Seek};
    use std::os::fd::{AsRawFd, FromRawFd};

    /// Redirects file descriptor 1 for the lifetime of the value.
    struct Redirect {
        null: File,
    }

    impl Redirect {
        fn point_at(&self, file: &File) -> Result<()> {
            std::io::stdout().flush()?;
            // SAFETY: both descriptors are open; dup2 atomically replaces descriptor 1.
            if unsafe { libc::dup2(file.as_raw_fd(), 1) } < 0 {
                bail!("could not redirect standard output");
            }
            Ok(())
        }

        fn release(&self) -> Result<()> {
            self.point_at(&self.null)
        }
    }

    pub(super) fn serve() -> Result<()> {
        // Job lines go to a duplicate of the original stdout; descriptor 1 then points at
        // /dev/null between jobs and at a capture file during each one.
        // SAFETY: duplicating descriptor 1 has no preconditions; the result is owned below.
        let fd = unsafe { libc::dup(1) };
        if fd < 0 {
            bail!("could not duplicate standard output");
        }
        // SAFETY: `fd` is a fresh descriptor owned by nobody else.
        let mut lines = std::io::BufWriter::new(unsafe { File::from_raw_fd(fd) });
        let redirect = Redirect {
            null: File::options().write(true).open("/dev/null")?,
        };
        redirect.release()?;
        let home = std::env::current_dir()?;
        let capture_path =
            std::env::temp_dir().join(format!("aether-batch-{}.out", std::process::id()));
        eprintln!(
            "aether-motion batch {} ready: one JSON job per line on stdin",
            env!("CARGO_PKG_VERSION")
        );
        for line in std::io::stdin().lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let started = std::time::Instant::now();
            let (id, args, cwd) = match parse_job(&line) {
                Ok(job) => job,
                Err(error) => {
                    let id = serde_json::from_str::<Value>(&line)
                        .ok()
                        .and_then(|job| job.get("id").cloned())
                        .unwrap_or(Value::Null);
                    let mut out = result_line(id, 2, String::new(), started.elapsed());
                    out["error"] = json!(format!("{error:#}"));
                    writeln!(lines, "{out}")?;
                    lines.flush()?;
                    continue;
                }
            };
            let (exit, stdout) = run_job(&redirect, &capture_path, &args, cwd.as_deref());
            std::env::set_current_dir(&home)?;
            crate::system::trim_heap();
            writeln!(
                lines,
                "{}",
                result_line(id, exit, stdout, started.elapsed())
            )?;
            lines.flush()?;
        }
        let _ = std::fs::remove_file(&capture_path);
        Ok(())
    }

    /// Runs one job with its stdout captured; returns the exit code and the captured text.
    fn run_job(
        redirect: &Redirect,
        capture_path: &std::path::Path,
        args: &[String],
        cwd: Option<&str>,
    ) -> (i32, String) {
        if let Some(dir) = cwd {
            if let Err(error) = std::env::set_current_dir(dir) {
                eprintln!("error: cannot enter {dir}: {error}");
                return (1, String::new());
            }
        }
        let capture = match File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(capture_path)
        {
            Ok(file) => file,
            Err(error) => {
                eprintln!("error: cannot create {}: {error}", capture_path.display());
                return (1, String::new());
            }
        };
        if let Err(error) = redirect.point_at(&capture) {
            eprintln!("error: {error:#}");
            return (1, String::new());
        }
        let exit = std::panic::catch_unwind(|| crate::cli::main(args)).unwrap_or_else(|_| {
            eprintln!("error: internal error; please report it with the job");
            101
        });
        let _ = redirect.release();
        let mut text = String::new();
        let mut capture = capture;
        let _ = capture.rewind();
        let _ = capture.take(MAX_STDOUT).read_to_string(&mut text);
        (exit, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_are_validated_before_they_run() {
        let (id, args, cwd) =
            parse_job(r#"{"id": 7, "args": ["check", "a.html"], "cwd": "/tmp"}"#).unwrap();
        assert_eq!(
            (id, args, cwd.as_deref()),
            (
                json!(7),
                vec!["check".to_string(), "a.html".into()],
                Some("/tmp")
            )
        );
        for bad in [
            "not json",
            r#"{"args": []}"#,
            r#"{"args": ["batch"]}"#,
            r#"{"args": ["mcp"]}"#,
            r#"{"args": ["render", 3]}"#,
            r#"{"args": ["check"], "cwd": 1}"#,
        ] {
            assert!(parse_job(bad).is_err(), "{bad}");
        }
    }
}
