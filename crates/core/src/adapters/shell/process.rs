//! The only place in the crate that spawns a subprocess.
//!
//! Only one, so that "a port decides, it never calls `git`/`gh` itself"
//! has a place to be true. `git.rs` and `github.rs` are translators
//! above this module: they build arguments and read output, they don't talk to the system.

use std::path::Path;

use crate::domain::{Halt, Outcome};

/// What a process left behind.
#[derive(Debug, Clone)]
pub struct Ran {
    /// The exit code. `None` when a signal killed the process.
    pub code: Option<i32>,
    /// Standard output, as-is.
    pub stdout: String,
    /// Standard error, as-is.
    pub stderr: String,
}

impl Ran {
    /// True if the process returned zero.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// Standard output, trimmed.
    #[must_use]
    pub fn out(&self) -> &str {
        self.stdout.trim()
    }

    /// Non-empty lines from standard output.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.stdout
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty())
            .map(ToString::to_string)
            .collect()
    }

    /// The last useful line from stderr — the diagnostic part.
    ///
    /// It's the only line a human should read when two paid passes failed to post,
    /// so it must never be empty or blank.
    #[must_use]
    pub fn why(&self) -> String {
        last_line(&self.stderr)
    }
}

/// The last non-empty line of text, or a message saying there is none.
#[must_use]
pub fn last_line(text: &str) -> String {
    text.lines()
        .rfind(|line| !line.trim().is_empty())
        .map_or_else(
            || "(nothing on stderr)".to_string(),
            |line| line.trim().to_string(),
        )
}

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
/// not in `PATH`, or working directory doesn't exist.
pub async fn run(binary: &str, args: &[String], cwd: &Path) -> Outcome<Ran> {
    let out = tokio::process::Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .output()
        .await
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(stdout: &str, stderr: &str, code: i32) -> Ran {
        Ran {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn empty_and_whitespace_lines_are_dropped() {
        let out = ran("a\n\n  \nb\n", "", 0);
        assert_eq!(out.lines(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn no_output_at_all_yields_no_lines_rather_than_one_empty_one() {
        assert!(ran("", "", 0).lines().is_empty());
        assert!(ran("\n\n", "", 0).lines().is_empty());
    }

    #[test]
    fn why_takes_the_last_useful_line_of_stderr() {
        let out = ran("", "warning: blah\nfatal: not a git repository\n", 128);
        assert_eq!(out.why(), "fatal: not a git repository");
    }

    #[test]
    fn an_empty_stderr_says_so_instead_of_being_blank() {
        // Prevents the failure mode: an empty diagnostic line, the only
        // thing a human had to read after two paid passes.
        assert_eq!(ran("", "", 1).why(), "(nothing on stderr)");
        assert_eq!(ran("", "   \n\n", 1).why(), "(nothing on stderr)");
    }

    #[test]
    fn ok_is_exactly_zero_not_merely_absence_of_error() {
        assert!(ran("", "", 0).ok());
        assert!(!ran("", "", 1).ok());
        let killed = Ran {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(!killed.ok());
    }

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
}
