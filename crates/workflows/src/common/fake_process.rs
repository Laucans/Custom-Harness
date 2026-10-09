//! An in-memory process runner: answers what the test told it, by program
//! and first argument, and records every run.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use harness_core::domain::Outcome;
use harness_core::ports::shell::process::{Process, Ran};

/// A process runner in memory.
#[derive(Default)]
pub struct FakeProcess {
    /// What each run answers, in order; past the end, an empty success.
    pub answers: RefCell<Vec<Ran>>,
    /// Every run: the program, its arguments, its folder.
    pub ran: RefCell<Vec<(String, Vec<String>, PathBuf)>>,
}

impl FakeProcess {
    /// A runner whose runs answer these, in order.
    pub fn answering(answers: Vec<Ran>) -> Self {
        Self {
            answers: RefCell::new(answers),
            ran: RefCell::default(),
        }
    }

    /// A run that printed `stdout` and exited with `code`.
    pub fn ran(stdout: &str, stderr: &str, code: i32) -> Ran {
        Ran {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }
}

#[async_trait(?Send)]
impl Process for FakeProcess {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        _timeout: Duration,
    ) -> Outcome<Ran> {
        self.ran
            .borrow_mut()
            .push((program.to_string(), args.to_vec(), cwd.to_path_buf()));
        let mut answers = self.answers.borrow_mut();
        Ok(if answers.is_empty() {
            Self::ran("", "", 0)
        } else {
            answers.remove(0)
        })
    }
}
