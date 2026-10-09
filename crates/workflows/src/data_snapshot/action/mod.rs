//! What a data snapshot writes: the clone brought to a head and its schema
//! dumped (`capture`), then the versions committed on the archive branch
//! (`archive`).

pub mod archive;
pub mod capture;

use harness_core::ports::shell::process::Ran;

/// What `git` said, read as a verdict: its last stderr line when it refused.
fn said(verb: &str, ran: &Ran) -> Result<(), String> {
    if ran.ok() {
        Ok(())
    } else {
        Err(format!("git {verb}: {}", ran.why()))
    }
}
