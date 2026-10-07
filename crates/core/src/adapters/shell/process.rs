//! The only place in the crate that spawns a subprocess.
//!
//! Only one, so that "a port decides, it never calls `git`/`gh` itself"
//! has a place to be true. `git.rs` and `github.rs` are translators
//! above this module: they build arguments and read output, they don't talk to the system.

use std::path::Path;
use std::time::Duration;

use crate::domain::{Halt, Outcome};
use crate::ports::shell::process::Ran;

/// How long a `git`/`gh` call may run before it's treated as hung.
///
/// Generous for a CLI round-trip, never for a Claude session — that one
/// carries its own, much longer, budget (`adapters::agent::claude_cli`).
const TIMEOUT: Duration = Duration::from_secs(60);

/// Run `binary` with these arguments, from `cwd`.
///
/// **A non-zero exit code is not an error here.** `git rev-parse
/// --verify` answers "this branch doesn't exist" with a non-zero code,
/// and that's a response, not a failure. Only a binary that couldn't be run
/// returns `Err` — the caller decides what the code means.
///
/// # Errors
///
/// [`Halt::Failed`] if the process couldn't be launched at all: binary
/// not in `PATH`, or working directory doesn't exist. Also [`Halt::Failed`]
/// if it ran past [`TIMEOUT`] — a hung `git`/`gh` call must not block the
/// harness forever.
pub async fn run(binary: &str, args: &[String], cwd: &Path) -> Outcome<Ran> {
    run_within(binary, args, cwd, TIMEOUT).await
}

/// [`run`], with the deadline the caller's own command deserves.
///
/// [`TIMEOUT`] is sized for a `git`/`gh` round-trip. A command that is honestly
/// slow — `npm install` on a cold cache — is not hung at sixty seconds, and
/// killing it there would leave a half-populated `node_modules` behind.
///
/// # Errors
///
/// Same as [`run`], with `timeout` in place of [`TIMEOUT`].
pub async fn run_for(binary: &str, args: &[String], cwd: &Path, timeout: Duration) -> Outcome<Ran> {
    run_within(binary, args, cwd, timeout).await
}

/// [`run`], with the deadline as a parameter — the seam a test uses to prove
/// a hung process is a failure without actually waiting out [`TIMEOUT`].
async fn run_within(binary: &str, args: &[String], cwd: &Path, timeout: Duration) -> Outcome<Ran> {
    let spawn = tokio::process::Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .output();
    let out = tokio::time::timeout(timeout, spawn)
        .await
        .map_err(|_| Halt::Failed(format!("{binary} timed out after {}s", timeout.as_secs())))?
        .map_err(|e| {
            Halt::Failed(format!(
                "couldn't launch {binary} from {}: {e}",
                cwd.display()
            ))
        })?;
    Ok(Ran {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Which pipe a streamed line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// Standard output — for a JSON stream, the events.
    Out,
    /// Standard error — where a rejected flag explains itself.
    Err,
}

/// Run `binary`, handing each output line to `watch` **as it arrives**.
///
/// Same contract as [`run`] for the result: a non-zero code is data, only a
/// process that could not run is an error. What differs is the timing — [`run`]
/// returns when the process is over, so a turn that lasts an hour says nothing
/// for an hour. Here the caller sees each line while it runs, which is what
/// makes a live log possible.
///
/// Both pipes are drained concurrently, and that is not a detail: a process
/// writing a lot to `stderr` while nobody reads it blocks on a full pipe
/// forever, and the deadline would then fire on a process that was only
/// waiting for us.
///
/// # Errors
///
/// [`Halt::Failed`] if the process could not be launched, or if it outlived
/// `timeout` — in which case it is killed rather than left behind.
///
/// # Panics
///
/// Never in practice: both pipes are requested from the builder immediately
/// above the `take`, so neither can be absent. The invariant is not one the
/// type carries.
pub async fn run_streaming(
    binary: &str,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    watch: &mut dyn FnMut(Stream, &str),
) -> Outcome<Ran> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = tokio::process::Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            Halt::Failed(format!(
                "couldn't launch {binary} from {}: {e}",
                cwd.display()
            ))
        })?;
    // Piped just above, so these are `Some` by construction — a fact the type
    // cannot carry.
    let out = child.stdout.take().expect("stdout was piped");
    let err = child.stderr.take().expect("stderr was piped");
    let mut out = BufReader::new(out).lines();
    let mut err = BufReader::new(err).lines();

    let deadline = tokio::time::Instant::now() + timeout;
    let mut stdout = String::new();
    let mut stderr = String::new();
    let drained = tokio::time::timeout_at(
        deadline,
        drain(&mut out, &mut err, &mut stdout, &mut stderr, watch),
    )
    .await;
    let timed_out =
        |binary: &str| Halt::Failed(format!("{binary} timed out after {}s", timeout.as_secs()));
    if drained.is_err() {
        // Nothing is left running behind the harness's back.
        let _ = child.kill().await;
        return Err(timed_out(binary));
    }
    let status = tokio::time::timeout_at(deadline, child.wait())
        .await
        .map_err(|_| timed_out(binary))?
        .map_err(|e| Halt::Failed(format!("{binary} could not be waited on: {e}")))?;
    Ok(Ran {
        code: status.code(),
        stdout,
        stderr,
    })
}

/// Reads both pipes until each reaches its end, keeping what they said.
///
/// A read error ends that pipe rather than the call: the process may still
/// have something to say on the other one, and its exit code remains the
/// answer.
async fn drain<O, E>(
    out: &mut tokio::io::Lines<O>,
    err: &mut tokio::io::Lines<E>,
    stdout: &mut String,
    stderr: &mut String,
    watch: &mut dyn FnMut(Stream, &str),
) where
    O: tokio::io::AsyncBufRead + Unpin,
    E: tokio::io::AsyncBufRead + Unpin,
{
    let mut out_open = true;
    let mut err_open = true;
    while out_open || err_open {
        tokio::select! {
            line = out.next_line(), if out_open => match line {
                Ok(Some(text)) => {
                    watch(Stream::Out, &text);
                    stdout.push_str(&text);
                    stdout.push('\n');
                }
                _ => out_open = false,
            },
            line = err.next_line(), if err_open => match line {
                Ok(Some(text)) => {
                    watch(Stream::Err, &text);
                    stderr.push_str(&text);
                    stderr.push('\n');
                }
                _ => err_open = false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_missing_binary_is_a_failure_not_a_non_zero_code() {
        let err = run("nonexistent-binary", &[], Path::new("."))
            .await
            .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[tokio::test]
    async fn a_non_zero_exit_comes_back_as_data() {
        // `false` exits with 1: it's a response, not a failure.
        let out = run("false", &[], Path::new(".")).await.expect("launched");
        assert!(!out.ok());
        assert_eq!(out.code, Some(1));
    }

    #[tokio::test]
    async fn a_streamed_run_hands_over_each_line_while_it_runs() {
        let mut seen = Vec::new();
        let out = run_streaming(
            "sh",
            &[
                "-c".to_string(),
                "echo one; echo two 1>&2; echo three".to_string(),
            ],
            Path::new("."),
            Duration::from_secs(10),
            &mut |stream, line| seen.push((stream, line.to_string())),
        )
        .await
        .expect("launched");
        assert!(out.ok());
        assert!(seen.contains(&(Stream::Out, "one".to_string())));
        assert!(seen.contains(&(Stream::Err, "two".to_string())));
        assert!(seen.contains(&(Stream::Out, "three".to_string())));
        // And what was handed over is also what the result carries.
        assert_eq!(out.lines(), ["one".to_string(), "three".to_string()]);
        assert_eq!(out.why(), "two");
    }

    #[tokio::test]
    async fn a_streamed_run_reports_a_non_zero_code_as_data() {
        let out = run_streaming(
            "false",
            &[],
            Path::new("."),
            Duration::from_secs(10),
            &mut |_, _| {},
        )
        .await
        .expect("launched");
        assert_eq!(out.code, Some(1));
    }

    #[tokio::test]
    async fn a_streamed_run_that_outlives_its_deadline_is_killed_not_awaited() {
        let mut seen = 0_u32;
        let err = run_streaming(
            "sh",
            &["-c".to_string(), "echo starting; sleep 5".to_string()],
            Path::new("."),
            Duration::from_millis(80),
            &mut |_, _| seen += 1,
        )
        .await
        .expect_err("a process that outlives the deadline must fail");
        assert!(matches!(err, Halt::Failed(_)));
        // The deadline did not cost us what it had already said.
        assert_eq!(seen, 1);
    }

    #[tokio::test]
    async fn a_streamed_missing_binary_is_a_failure_not_a_non_zero_code() {
        let err = run_streaming(
            "nonexistent-binary",
            &[],
            Path::new("."),
            Duration::from_secs(10),
            &mut |_, _| {},
        )
        .await
        .expect_err("must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }

    #[tokio::test]
    async fn a_hung_process_is_a_failure_not_an_infinite_wait() {
        // A short deadline against a process that outlives it — proves the
        // enforcement itself, without waiting out the real TIMEOUT (60s).
        let err = run_within(
            "sleep",
            &["5".to_string()],
            Path::new("."),
            Duration::from_millis(50),
        )
        .await
        .expect_err("a process that outlives the deadline must fail");
        assert!(matches!(err, Halt::Failed(_)));
    }
}
