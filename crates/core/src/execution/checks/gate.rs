//! A gate: multiple checks, the first one that fails wins.

use async_trait::async_trait;

use crate::domain::{Outcome, Verdict};
use crate::execution::data::context::Context;
use crate::execution::traits::Verification;
use crate::traces::Event;

/// Multiple checks surrounding an executable, at any level — around an action
/// to validate individually, around a stage or round to validate the whole.
pub struct Gate<S> {
    /// Named so a reader can list what is verified without reading the bodies,
    /// and so an observability pass has something to log when each passes.
    pub name: &'static str,
    /// In the order they verify. They assume each other: asking `gh` what
    /// labels exist doesn't make sense until we know it's authenticated.
    pub checks: Vec<Box<dyn Verification<S>>>,
}

impl<S> Gate<S> {
    /// A named gate with nothing to verify.
    #[must_use]
    pub fn empty(name: &'static str) -> Self {
        Self {
            name,
            checks: Vec::new(),
        }
    }
}

#[async_trait(?Send)]
impl<S> Verification<S> for Gate<S> {
    async fn verify(&self, ctx: &Context<S>) -> Outcome<Verdict> {
        for check in &self.checks {
            let outcome = check.verify(ctx).await;
            // Every verdict is told, a pass included: a gate nobody can see
            // passing is a gate nobody can tell from one that never ran.
            let (verdict, reason) = match &outcome {
                Ok(Verdict::Continue) => ("pass", String::new()),
                Ok(Verdict::Skip(why) | Verdict::NothingLeft(why)) => ("skip", why.clone()),
                Err(halt) => ("halt", halt.reason().to_string()),
            };
            ctx.traces.event(&Event::GateChecked {
                gate: self.name.to_string(),
                check: check.name(),
                purpose: check.purpose(),
                verdict: verdict.to_string(),
                reason,
            });
            let verdict = outcome?;
            if !matches!(verdict, Verdict::Continue) {
                return Ok(verdict);
            }
        }
        Ok(Verdict::Continue)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::domain::Halt;
    use crate::execution::data::context::Settings;
    use crate::traces::{Logbook, Sink, Verbosity};

    struct AlwaysPasses;

    #[async_trait(?Send)]
    impl Verification<()> for AlwaysPasses {
        async fn verify(&self, _ctx: &Context<()>) -> Outcome<Verdict> {
            Ok(Verdict::Continue)
        }

        fn purpose(&self) -> String {
            "nothing is ever wrong".to_string()
        }
    }

    /// Keeps every line and every event, for the tests to read back.
    #[derive(Default)]
    struct Kept {
        lines: RefCell<Vec<String>>,
        events: RefCell<Vec<Event>>,
    }

    impl Sink for Kept {
        fn emit(&self, line: &str) {
            self.lines.borrow_mut().push(line.to_string());
        }

        fn record(&self, event: &Event) {
            self.events.borrow_mut().push(event.clone());
        }
    }

    fn kept_ctx() -> (Context<()>, Rc<Kept>) {
        let kept = Rc::new(Kept::default());
        let sink: Rc<dyn Sink> = Rc::clone(&kept) as Rc<dyn Sink>;
        let ctx = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            (),
            Logbook::new(sink, Verbosity::Normal),
        );
        (ctx, kept)
    }

    struct AlwaysSkip;

    #[async_trait(?Send)]
    impl Verification<()> for AlwaysSkip {
        async fn verify(&self, _ctx: &Context<()>) -> Outcome<Verdict> {
            Ok(Verdict::Skip("already done".into()))
        }
    }

    struct AlwaysHalts;

    #[async_trait(?Send)]
    impl Verification<()> for AlwaysHalts {
        async fn verify(&self, _ctx: &Context<()>) -> Outcome<Verdict> {
            Err(Halt::Halted("cannot verify".into()))
        }
    }

    fn ctx() -> Context<()> {
        Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            (),
            Logbook::null(),
        )
    }

    #[tokio::test]
    async fn the_first_check_that_does_not_continue_wins() {
        let gate = Gate {
            name: "test",
            checks: vec![Box::new(AlwaysSkip)],
        };
        let verdict = gate.verify(&ctx()).await.unwrap();
        assert_eq!(verdict, Verdict::Skip("already done".into()));
    }

    #[tokio::test]
    async fn an_empty_gate_continues() {
        let gate: Gate<()> = Gate::empty("test");
        assert_eq!(gate.verify(&ctx()).await.unwrap(), Verdict::Continue);
    }

    #[tokio::test]
    async fn a_failing_check_propagates_as_a_halt() {
        let gate = Gate {
            name: "test",
            checks: vec![Box::new(AlwaysHalts)],
        };
        assert!(gate.verify(&ctx()).await.is_err());
    }

    #[tokio::test]
    async fn every_check_tells_its_verdict_with_the_gate_s_name() {
        let gate = Gate {
            name: "code requires",
            checks: vec![
                Box::new(AlwaysPasses),
                Box::new(AlwaysSkip),
                Box::new(AlwaysHalts),
            ],
        };
        let (ctx, kept) = kept_ctx();
        assert_eq!(
            gate.verify(&ctx).await.unwrap(),
            Verdict::Skip("already done".into())
        );
        let events = kept.events.borrow();
        let told: Vec<(String, String, String, String)> = events
            .iter()
            .map(|event| match event {
                Event::GateChecked {
                    check,
                    purpose,
                    verdict,
                    reason,
                    ..
                } => (
                    check.clone(),
                    purpose.clone(),
                    verdict.clone(),
                    reason.clone(),
                ),
                other => panic!("not a gate event: {other:?}"),
            })
            .collect();
        // The halting check never ran: the skip before it won.
        assert_eq!(
            told,
            [
                (
                    "AlwaysPasses".to_string(),
                    "nothing is ever wrong".to_string(),
                    "pass".to_string(),
                    String::new()
                ),
                (
                    "AlwaysSkip".to_string(),
                    String::new(),
                    "skip".to_string(),
                    "already done".to_string()
                ),
            ]
        );
        assert!(
            events
                .iter()
                .all(|e| matches!(e, Event::GateChecked { gate, .. } if gate == "code requires"))
        );
        let lines = kept.lines.borrow();
        assert_eq!(lines[0], "gate \"code requires\" · AlwaysPasses: pass");
    }

    #[tokio::test]
    async fn a_halt_is_told_before_it_propagates() {
        let gate = Gate {
            name: "test",
            checks: vec![Box::new(AlwaysHalts)],
        };
        let (ctx, kept) = kept_ctx();
        assert!(gate.verify(&ctx).await.is_err());
        let events = kept.events.borrow();
        assert!(matches!(
            &events[0],
            Event::GateChecked { verdict, reason, .. } if verdict == "halt" && reason == "cannot verify"
        ));
    }
}
