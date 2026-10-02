//! The carrier of a `--dry-run`: it returns the prompt, it opens nothing.
//!
//! A dry-run **is a wiring choice**, not a framework branch. The stage opens
//! its session without knowing what is behind it; the launcher decides that
//! this time, there is this. On the Python side, each executing function
//! carried its `if cfg.dry_run`, and the blank run path was thus a second
//! pass through code — one that no integration test covers.
//!
//! What a dry-run shows: the exact prompt that would be sent, stage by stage,
//! scope included. That is the question it exists to answer.

use async_trait::async_trait;

use crate::adapters::agent::{Reply, Session, SessionFactory, SessionSpec};
use crate::domain::{Outcome, Spend};
use crate::traces::Logbook;

/// Opens sessions that are not really sessions.
pub struct Rehearsal {
    log: Logbook,
}

impl Rehearsal {
    /// A rehearsal that writes what it would have sent to this logbook.
    #[must_use]
    pub const fn new(log: Logbook) -> Self {
        Self { log }
    }
}

#[async_trait(?Send)]
impl SessionFactory for Rehearsal {
    async fn open(&self, spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
        self.log.say(&format!(
            "dry-run — no session opened (would have been {}, effort {})",
            spec.model, spec.effort
        ));
        Ok(Box::new(Transcript {
            log: self.log.clone(),
        }))
    }
}

/// A session that echoes what it is told and returns nothing.
struct Transcript {
    log: Logbook,
}

#[async_trait(?Send)]
impl Session for Transcript {
    async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
        self.log.say(&format!(
            "dry-run — the prompt that would be sent:\n{prompt}"
        ));
        // Empty text, so no marker: a caller demanding `AGENT_LOOP_OK` here would
        // read the lack of response as a failure, when no one was questioned. It is
        // up to them not to demand it in a dry-run, and `Spend::default()` says
        // "nothing observed", not "free".
        Ok(Reply {
            text: String::new(),
            stop_line: None,
            spend: Spend::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::{Sink, Verbosity};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn spec() -> SessionSpec {
        SessionSpec {
            model: "opus".to_string(),
            effort: "high".to_string(),
        }
    }

    #[tokio::test]
    async fn a_rehearsal_writes_the_prompt_it_would_have_sent() {
        let capture = Rc::new(Capture::default());
        let log = Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal);
        let mut session = Rehearsal::new(log).open(&spec()).await.expect("opened");
        session
            .ask("/code\nthe instructions")
            .await
            .expect("a response");
        let said = capture.0.borrow().join("\n");
        assert!(said.contains("no session opened"));
        assert!(
            said.contains("the instructions"),
            "the exact prompt, not a summary"
        );
    }

    #[tokio::test]
    async fn a_rehearsal_observes_nothing_rather_than_reporting_zero() {
        // A zero ledger line would read back as a free session.
        let mut session = Rehearsal::new(Logbook::null())
            .open(&spec())
            .await
            .expect("opened");
        let reply = session.ask("whatever").await.expect("a response");
        assert!(reply.spend.is_blind());
        assert!(reply.stop_line.is_none());
    }
}
