//! Talk to the open session of a stage: send, re-read markers,
//! record spending, keep the response for the next stage.
//!
//! **Common to all three workflows**, and written here for this exact reason:
//! the dev loop composed this inline in its only session action, because it was
//! alone. PR review and refinement each need it identically — only the prompt
//! and resume point change — so it lives here now, and the loop delegates.

use crate::domain::{Halt, Outcome, Spend, breaker, markers};
use crate::execution::action::kinds::Open;
use crate::ports::agent::Reply;
use crate::ports::store::spending::{Entry, Spending};

/// Sends `prompt` to the open session, and does what every paid stage does
/// with its response.
///
/// Writes the log entry **before** propagating a failure: a dead stage is
/// exactly the one we want traces for, and an entry with `None` rather than
/// zero — an unobserved cost should never read back as a free session.
///
/// Keeps the response in `ctx.results[stage]` so that a later stage in the
/// same round can read it — this is how the « brief » pass of the review reads
/// the « inline » pass, and how refinement coherence reads its five sections.
///
/// Refuses to send it at all when the identical session — same task, same
/// stage, same prompt — already ended in error [`breaker::LIMIT`] times: the
/// answer is bought and known, and paying for it again is the failure mode
/// this guards (see [`breaker`]).
///
/// # Errors
///
/// The session failure, after the log entry has been written. Or
/// [`Halt::Halted`] if the session stopped itself
/// (`AGENT_LOOP_STOP`) — a correct result, and the reason is its own — or if
/// the breaker refused to pay for it.
pub async fn ask_and_record<S>(
    open: &mut Open<'_, S>,
    prompt: &str,
    stage: &str,
    round: u32,
    task: &str,
    spending: &dyn Spending,
) -> Outcome<Reply> {
    let log = open.traces.bind(stage);
    let dry_run = open.settings.dry_run;
    let fingerprint = breaker::fingerprint(prompt);

    // Before the call, because the point is not to make it.
    if !dry_run {
        let failures = spending.failures(task, stage, &fingerprint)?;
        if breaker::tripped(failures) {
            return Err(Halt::Halted(refusal(stage, task, failures)));
        }
    }

    let asked = open.session.ask(prompt).await;
    // A session that stopped itself did not succeed, and the ledger has to
    // say so: this is the row the breaker counts, so reading it back as `ok`
    // is what let the same refusal be paid for five times.
    let stopped = asked
        .as_ref()
        .ok()
        .and_then(|reply| markers::stop_line(&reply.text));
    if !dry_run {
        let blind = Spend::default();
        let (spend, outcome) = match (&asked, &stopped) {
            (Ok(reply), None) => (&reply.spend, "ok"),
            (Ok(reply), Some(_)) => (&reply.spend, breaker::STOP),
            (Err(halt), _) => (&blind, halt.prefix()),
        };
        spending.record(&Entry {
            round,
            task,
            stage,
            spend,
            outcome,
            fingerprint: &fingerprint,
        })?;
        // After the row, and never fatal: the row is what the budget and the
        // breaker read, the reading is only what spares the *next* run a stage
        // it cannot finish. A convenience that could not be kept must not undo
        // a turn already paid for.
        if let Some(reading) = spend.quota.as_ref()
            && let Err(why) = spending.remember_quota(reading)
        {
            log.warn(&format!(
                "the rate-limit reading could not be kept ({why}) — the next \
                 run will start without it, as it did before"
            ));
        }
    }
    let reply = asked?;

    if let Some(stop) = stopped {
        // A session that responds with AGENT_LOOP_STOP stopped itself:
        // a correct result, and the reason is its own.
        return Err(Halt::Halted(format!("{stop} (/{stage})")));
    }
    match markers::ok_line(&reply.text) {
        Some(ok) => log.say(&ok),
        None if !dry_run => log.say(&format!(
            "/{stage} produced no {} marker — relying on the structural checks",
            markers::OK
        )),
        None => {}
    }

    open.ctx.results.insert(stage.to_string(), reply.clone());
    Ok(reply)
}

/// Why nothing was sent, and the gesture that makes the next session
/// different.
///
/// Names what changes the prompt rather than a flag to bypass the breaker:
/// bypassing it buys the same answer, and the fingerprint only moves when
/// what is asked moves.
fn refusal(stage: &str, task: &str, failures: u32) -> String {
    let subject = if task.is_empty() {
        format!("/{stage}")
    } else {
        format!("/{stage} on {task}")
    };
    format!(
        "refusing to pay for {subject}: this exact prompt already ended in \
         error {failures} times in a row, and nothing in it has changed \
         since. The next session would cost the same and answer the same. \
         Change what is asked — for a task, the issue body is what the prompt \
         is built from — and this runs again on its own."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::data::context::{Context, Settings};
    use crate::ports::store::spending::Entry as SpendingEntry;
    use crate::traces::{Logbook, Sink, Verbosity};
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Scripted(Result<Reply, Halt>);

    #[async_trait(?Send)]
    impl crate::ports::agent::Session for Scripted {
        async fn ask(&mut self, _prompt: &str) -> Outcome<Reply> {
            self.0.clone()
        }
    }

    /// One row, as the in-memory ledger keeps it.
    struct Kept {
        outcome: String,
        cost_usd: Option<f64>,
        fingerprint: String,
    }

    /// A ledger in memory: what it was told, and what it answers about the
    /// past. A fake, not a mock — the breaker reads a real history here.
    #[derive(Default)]
    struct Recorded {
        rows: RefCell<Vec<Kept>>,
        past: u32,
    }

    impl Spending for Recorded {
        fn record(&self, entry: &SpendingEntry<'_>) -> Outcome<()> {
            self.rows.borrow_mut().push(Kept {
                outcome: entry.outcome.to_string(),
                cost_usd: entry.spend.cost_usd,
                fingerprint: entry.fingerprint.to_string(),
            });
            Ok(())
        }

        fn failures(&self, _task: &str, _stage: &str, _fingerprint: &str) -> Outcome<u32> {
            Ok(self.past)
        }
    }

    #[derive(Default)]
    struct Capture(RefCell<Vec<String>>);

    impl Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    fn answered(text: &str) -> Reply {
        Reply {
            text: text.to_string(),
            stop_line: None,
            spend: Spend {
                cost_usd: Some(0.1),
                ..Spend::default()
            },
        }
    }

    async fn run(
        reply: Result<Reply, Halt>,
        dry_run: bool,
    ) -> (Outcome<Reply>, Context<()>, String) {
        let (result, ctx, said, _) = run_with(reply, dry_run, 0).await;
        (result, ctx, said)
    }

    async fn run_with(
        reply: Result<Reply, Halt>,
        dry_run: bool,
        past: u32,
    ) -> (Outcome<Reply>, Context<()>, String, Recorded) {
        let spending = Recorded {
            past,
            ..Recorded::default()
        };
        let capture = Rc::new(Capture::default());
        let mut ctx = Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            (),
            Logbook::new(Rc::clone(&capture) as Rc<dyn Sink>, Verbosity::Normal),
        );
        let mut session = Scripted(reply);
        let result = {
            let mut open = Open {
                ctx: &mut ctx,
                session: &mut session,
            };
            ask_and_record(&mut open, "le prompt", "brief", 2, "32", &spending).await
        };
        let said = capture.0.borrow().join("\n");
        (result, ctx, said, spending)
    }

    #[tokio::test]
    async fn a_successful_reply_is_kept_in_results_under_its_stage_name() {
        let (result, ctx, _) = run(Ok(answered("AGENT_LOOP_OK: fait")), false).await;
        result.expect("a response");
        assert_eq!(ctx.results["brief"].text, "AGENT_LOOP_OK: fait");
    }

    #[tokio::test]
    async fn a_stop_marker_halts_and_names_the_stage() {
        let (result, _, _) = run(Ok(answered("AGENT_LOOP_STOP: blocked")), false).await;
        let err = result.expect_err("must stop");
        assert!(matches!(err, Halt::Halted(_)));
        assert!(err.reason().contains("(/brief)"));
    }

    #[tokio::test]
    async fn a_missing_ok_marker_is_said_not_treated_as_a_failure() {
        let (result, _, said) = run(Ok(answered("I finished")), false).await;
        assert!(result.is_ok());
        assert!(said.contains("no AGENT_LOOP_OK marker"));
    }

    #[tokio::test]
    async fn a_dead_stage_still_gets_its_line_with_nothing_observed() {
        let (result, _, _) = run(Err(Halt::Quota("exhausted".into())), false).await;
        assert!(matches!(result, Err(Halt::Quota(_))));
    }

    #[tokio::test]
    async fn a_self_stopped_session_is_recorded_as_an_error_not_as_ok() {
        // The row the breaker counts. Recorded as `ok`, the same refusal
        // gets paid for at every tick of the polling loop.
        let (result, _, _, spending) =
            run_with(Ok(answered("AGENT_LOOP_STOP: blocked")), false, 0).await;
        assert!(result.is_err());
        let rows = spending.rows.borrow();
        assert_eq!(rows.len(), 1, "it ran, so it is billed");
        assert_eq!(rows[0].outcome, breaker::STOP);
        assert_eq!(rows[0].cost_usd, Some(0.1), "what it cost is not forgotten");
        assert_eq!(
            rows[0].fingerprint.len(),
            16,
            "under the prompt that produced it"
        );
    }

    #[tokio::test]
    async fn a_fourth_identical_failure_is_refused_before_it_is_paid_for() {
        let (result, ctx, _, spending) =
            run_with(Ok(answered("AGENT_LOOP_OK: fait")), false, breaker::LIMIT).await;
        let err = result.expect_err("must refuse");
        assert!(
            matches!(err, Halt::Halted(_)),
            "a correct refusal, not a crash"
        );
        assert!(err.reason().contains("refusing to pay"), "{}", err.reason());
        assert!(err.reason().contains("the issue body"), "names the gesture");
        assert!(
            spending.rows.borrow().is_empty(),
            "nothing is billed, because nothing was sent"
        );
        assert!(
            !ctx.results.contains_key("brief"),
            "and no response is invented for the next stage"
        );
    }

    #[tokio::test]
    async fn two_identical_failures_still_leave_a_third_session_to_run() {
        let (result, _, _, spending) = run_with(
            Ok(answered("AGENT_LOOP_OK: fait")),
            false,
            breaker::LIMIT - 1,
        )
        .await;
        result.expect("a response");
        assert_eq!(spending.rows.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_dry_run_pays_nothing_so_the_breaker_does_not_speak() {
        let (result, _, _, _) =
            run_with(Ok(answered("AGENT_LOOP_OK: fait")), true, breaker::LIMIT).await;
        result.expect("a dry run asks for no money and is refused nothing");
    }

    #[tokio::test]
    async fn a_dry_run_records_no_spend_and_asks_for_no_marker() {
        let (result, _, said) = run(Ok(answered("")), true).await;
        assert!(result.is_ok());
        assert!(!said.contains("no AGENT_LOOP_OK marker"));
    }
}

#[cfg(test)]
mod carrier_agnostic {
    //! That the rate-limit reading belongs to the **contract**, not to one
    //! adapter — so a second carrier can supply it without touching the first.
    use std::cell::RefCell;
    use std::rc::Rc;

    use async_trait::async_trait;

    use crate::domain::{Halt, Outcome, Spend, quota};
    use crate::execution::action::ask::ask_and_record;
    use crate::execution::action::kinds::Open;
    use crate::execution::data::context::{Context, Settings};
    use crate::ports::agent::{Reply, Session};
    use crate::ports::store::spending::{Entry, Spending};
    use crate::traces::Logbook;

    /// A carrier that is not `claude`, reporting a window in its own words.
    ///
    /// This is the whole point of the test: it implements [`Session`] only, knows
    /// nothing of `rate_limit_event`, and still fills the field the preflight
    /// reads. An earlier shape had `claude_cli` write the file itself — a carrier
    /// like this one would have silently supplied nothing, and the gate would
    /// have become dead code with no compile error to say so.
    struct OtherCarrier;

    #[async_trait(?Send)]
    impl Session for OtherCarrier {
        async fn ask(&mut self, _prompt: &str) -> Outcome<Reply> {
            Ok(Reply {
                text: "AGENT_LOOP_OK: done".to_string(),
                stop_line: None,
                spend: Spend {
                    quota: Some(quota::Reading {
                        windows: vec![quota::Window {
                            name: "its_own_window".to_string(),
                            utilization: 0.5,
                            resets_at: 9_999,
                        }],
                        at: 7,
                    }),
                    ..Spend::default()
                },
            })
        }
    }

    #[derive(Default)]
    struct Keeps(RefCell<Vec<quota::Reading>>);

    impl Spending for Keeps {
        fn record(&self, _entry: &Entry<'_>) -> Outcome<()> {
            Ok(())
        }
        fn remember_quota(&self, reading: &quota::Reading) -> Outcome<()> {
            self.0.borrow_mut().push(reading.clone());
            Ok(())
        }
    }

    /// A store that refuses to keep anything, as a full disk would.
    struct Refuses;

    impl Spending for Refuses {
        fn record(&self, _entry: &Entry<'_>) -> Outcome<()> {
            Ok(())
        }
        fn remember_quota(&self, _reading: &quota::Reading) -> Outcome<()> {
            Err(Halt::Failed("the disk is full".to_string()))
        }
    }

    /// One turn through `ask_and_record`, against this carrier and this store.
    async fn asked_with(spending: &dyn Spending) -> Outcome<Reply> {
        let mut ctx = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            (),
            Logbook::null(),
        );
        let mut session = OtherCarrier;
        let mut open = Open {
            ctx: &mut ctx,
            session: &mut session,
        };
        ask_and_record(&mut open, "a prompt", "code", 1, "64", spending).await
    }

    #[tokio::test]
    async fn a_carrier_that_is_not_claude_still_supplies_the_reading() {
        let kept = Rc::new(Keeps::default());
        let reply = asked_with(kept.as_ref()).await;
        assert!(reply.is_ok(), "{reply:?}");
        let seen = kept.0.borrow();
        assert_eq!(seen.len(), 1, "the store was handed the reading");
        assert_eq!(seen[0].windows[0].name, "its_own_window");
        assert_eq!(seen[0].windows[0].left_percent(), 50);
    }

    #[tokio::test]
    async fn a_store_that_cannot_keep_the_reading_does_not_undo_the_turn() {
        // The turn is already paid for. Losing a convenience file must not cost
        // it — the next run simply starts without a reading, as it did before.
        let reply = asked_with(&Refuses).await;
        assert!(reply.is_ok(), "the turn stands: {reply:?}");
    }
}
