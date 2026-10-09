//! What a draft writes: the clone and the dump (`prepare`), the session's
//! question (`ask`), the commit and the pull request (`publish`).

pub mod ask;
pub mod prepare;
pub mod publish;

use harness_core::domain::{Halt, Outcome};
use harness_core::ports::shell::process::Ran;

/// What `git` said, read as a verdict: refused, the draft stops.
fn said(verb: &str, ran: &Ran) -> Outcome<()> {
    if ran.ok() {
        Ok(())
    } else {
        Err(Halt::Failed(format!("git {verb}: {}", ran.why())))
    }
}
