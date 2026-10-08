//! Cross-cutting support: a run's journal.
//!
//! A leaf — it imports nothing else from `harness-core`, and knows neither
//! `Halt`/`Verdict` nor the workflows.

mod logbook;

pub use logbook::{Logbook, Sink, Verbosity};

/// The file a run's process keeps an OS lock on while it lives.
///
/// The OS drops that lock when the process dies, however it dies: a reader
/// that can lock it again knows the run is over.
pub const RUN_ALIVE: &str = "alive.lock";

/// What a stage writes to the run's journal the moment it opens a session.
///
/// `session.log` heads every session with `── turn 1 …`, in the order they
/// opened, and says nothing of the stage; this line, in the same order, is
/// what names it — the view pairs the two to show one stage's session.
pub const SESSION_OPENS: &str = "session opens";

/// The journal line for `stage` opening its session: `[code] session opens`.
#[must_use]
pub fn session_opens(stage: &str) -> String {
    format!("[{stage}] {SESSION_OPENS}")
}

/// The stage a journal line says opened a session, if it is one — the
/// reverse of [`session_opens`], on the text after the line's clock.
#[must_use]
pub fn stage_opening(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix('[')?;
    let (stage, tail) = rest.split_once("] ")?;
    (tail.trim() == SESSION_OPENS && !stage.is_empty()).then_some(stage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_opening_reads_back_as_its_stage() {
        assert_eq!(stage_opening(&session_opens("code")), Some("code"));
        assert_eq!(stage_opening("[code] AGENT_LOOP_OK: done"), None);
        assert_eq!(stage_opening("session opens"), None);
    }
}
