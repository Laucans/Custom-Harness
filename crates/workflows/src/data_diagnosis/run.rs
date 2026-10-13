//! Assembles one diagnosis from ports and a config already built — the
//! concrete adapters stay the launcher's.

use std::cell::Cell;

use crate::data_diagnosis::config::Config;
use crate::data_diagnosis::data::state::DiagnosisState;
use crate::data_diagnosis::orchestration::round;
use crate::data_diagnosis::orchestration::workflow::DiagnosisRun;
use crate::data_diagnosis::ports::Ports;
use crate::data_snapshot::data::index::Head;

/// What an invocation requests.
pub struct Request {
    /// The branch and the commit whose data model diverges.
    pub head: Head,
    /// The version's folder on the archive branch.
    pub dir: String,
    /// What the static comparison found.
    pub divergences: Vec<String>,
}

impl Request {
    /// The state the run starts from.
    #[must_use]
    pub fn state(self) -> DiagnosisState {
        DiagnosisState {
            head: self.head,
            dir: self.dir,
            divergences: self.divergences,
            diagnosis: None,
        }
    }
}

/// Assembles one diagnosis.
#[must_use]
pub fn build(ports: &Ports, config: &Config) -> DiagnosisRun {
    DiagnosisRun {
        remaining: Cell::new(1),
        round: round::build(ports, config),
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use async_trait::async_trait;
    use harness_core::domain::{Outcome, Spend};
    use harness_core::execution::{Context, Guarded, Settings};
    use harness_core::ports::agent::{Base, Reply, Session, SessionFactory, SessionSpec};
    use harness_core::ports::store::spending::{Entry, Spending};
    use harness_core::traces::{Logbook, Sink, Verbosity};

    use super::*;

    /// A session that answers one reply, whatever it is asked, and keeps
    /// the prompt it got.
    struct Answers(String, Rc<std::cell::RefCell<String>>);

    #[async_trait(?Send)]
    impl Session for Answers {
        async fn ask(&mut self, prompt: &str) -> Outcome<Reply> {
            *self.1.borrow_mut() = prompt.to_string();
            Ok(Reply {
                text: self.0.clone(),
                stop_line: None,
                spend: Spend::default(),
            })
        }
    }

    struct Factory(String, Rc<std::cell::RefCell<String>>);

    #[async_trait(?Send)]
    impl SessionFactory for Factory {
        async fn open(&self, spec: &SessionSpec) -> Outcome<Box<dyn Session>> {
            assert_eq!(spec.model, "opus");
            Ok(Box::new(Answers(self.0.clone(), Rc::clone(&self.1))))
        }

        async fn fork(&self, spec: &SessionSpec, _base: &Base) -> Outcome<Box<dyn Session>> {
            self.open(spec).await
        }
    }

    /// A ledger that keeps nothing and has seen no failure.
    struct Ledger;

    impl Spending for Ledger {
        fn record(&self, _entry: &Entry<'_>) -> Outcome<()> {
            Ok(())
        }
    }

    struct Quiet;

    impl Sink for Quiet {
        fn emit(&self, _line: &str) {}
    }

    #[tokio::test]
    async fn a_diagnosis_reads_the_version_and_keeps_its_answer() {
        let asked = Rc::new(std::cell::RefCell::new(String::new()));
        let reply = "```json\n{\"summary\": \"The model is behind.\", \"findings\": [], \"actions\": [\"Describe pj.origine.\"]}\n```";
        let ports = Ports {
            sessions: Rc::new(Factory(reply.to_string(), Rc::clone(&asked))),
            spending: Rc::new(Ledger),
        };
        let config = Config {
            spec: SessionSpec {
                model: "opus".to_string(),
                effort: "high".to_string(),
            },
        };
        let run = build(&ports, &config);
        let request = Request {
            head: ("main_agent".to_string(), "abcdef0123".to_string()),
            dir: "main_agent/20261009T120000Z-abcdef0".to_string(),
            divergences: vec!["field pj.origine is in the database, not in the model".to_string()],
        };
        let mut ctx = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            request.state(),
            Logbook::new(Rc::new(Quiet), Verbosity::Normal),
        );
        Guarded::execute(&run, &mut ctx).await.expect("diagnosed");
        let d = ctx.state.diagnosis.expect("an answer");
        assert_eq!(d.summary, "The model is behind.");
        let prompt = asked.borrow();
        assert!(prompt.contains("- field pj.origine is in the database, not in the model"));
        assert!(prompt.contains("`main_agent/20261009T120000Z-abcdef0/schema.sql`"));
        assert!(prompt.contains("git show abcdef0123:<path>"));
        assert!(
            prompt.contains("{\"summary\""),
            "the JSON example stays literal"
        );
    }
}
