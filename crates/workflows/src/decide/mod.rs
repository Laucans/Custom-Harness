//! A decision taken by the harness, where a human used to be asked.
//!
//! Triggered by `harness:needs-decision` on an open issue: the business
//! advice wants one, the arbiter could not settle a graph question, a task's
//! PR still does not merge after every repair (`milestone_merge::escalate`),
//! a session stopped on an ambiguity or the breaker refused (the dev loop's
//! `## Decision`, kind `stop` or `breaker`).
//!
//! **Decide, never ask.** One session reads the issue, its `## Decision`,
//! its comments and, for a task waiting on its PR, the PR and its review,
//! and answers what to do: `answer` (the decision written, the task goes
//! on), `merge` (the PR as it is), `repair` (once more, the repairs counted
//! anew), or `cut` (the task closed, with why). What it decided is written
//! under `## Decision` (`decided-by: ai`), commented, labelled
//! `harness:ai-decided`, and told: the human reads it after the fact and
//! overrules it by answering under `## Decision` (`decided-by: owner`) and
//! posing `harness:needs-decision` again. An owner's answer is never decided
//! over.
//!
//! **One label, one decision.** `harness:needs-decision` is taken before the
//! session is paid for, as `pr_fix` takes its own.
//!
//! Same skeleton as the other workflows (`ARCHITECTURE.md`).

pub mod action;
pub mod checks;
pub mod config;
pub mod data;
pub mod orchestration;
pub mod ports;
pub mod run;
