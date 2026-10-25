//! What the round does when its session stops on an ambiguity, or the
//! breaker refuses to pay for it again: it asks for a decision instead of
//! waiting for a human.
//!
//! The stop is written under the task's `## Decision` (`kind: stop` or
//! `breaker`, `blocks:` the reason, `round:` how many times) and the task is
//! labelled `harness:needs-decision`: the `decide` workflow answers at the
//! next tick, the body changes — so the breaker's fingerprint moves — and the
//! loop takes the task again. Asked a third time, it is no longer the
//! harness's to settle: a `harness:human` task is opened, blocking it.
//!
//! A stop asking for a port ports.json has not (`AGENT_LOOP_STOP: port <id>
//! — <provider>, <cost>, <why>`) is no decision either: a new dependency
//! outside the product is a person's to approve, at once. A `harness:human`
//! task asks it, blocking the stopped one, whose `## Decision` keeps the
//! request (`kind: port`) for its next session.
//!
//! **Without a human gate** (`--no-human-gate`) no person is waited on for
//! a stop: a third stop asks for a decision once more, written under the
//! task's `## Decision` and commented by `decide`, which is what documents
//! it. A port asked is a person's either way: the Ports pane approves it.
//!
//! **A stop on what has not landed is no decision either.** When the reason
//! names an issue, a milestone's branch, or an id or a path the plan's graph
//! attributes to a milestone ([`waits`]), and that issue has not landed
//! ([`tasks::still_blocks`]), the task waits for it through a GitHub
//! blocker: `blocked_by` is written, the stop is kept under `## Decision`
//! (`kind: wait`), and nothing is paid until it lands. `ForgeDesMondes`'
//! #214 and #215 paid 23 code sessions and 23 decide sessions each, every
//! decision saying "keep waiting" — the blocker says it for free.
//!
//! **A stop on a task the plan lacks is no decision either**
//! (`AGENT_LOOP_STOP: task <slice JSON>`, [`missing`]): the session said
//! what to build, in which unit, and why. The harness opens it — split's
//! renderer, a full SPEC, `harness:agent` and `harness:ready` — as a
//! sub-issue of the milestone, or finds it among the milestone's open
//! agent tasks when a stop named it already, and parks the task on it as on
//! any wait. Nothing declared is dropped in silence: a slice declared wrong
//! is refused back to the session — a comment and the `## Decision` say
//! which and why, the unit keys listed — and the next session re-declares
//! it; one only a person can do, or declared wrong three times, goes to a
//! person (`harness:human`, blocking), or, without a human gate, asks a
//! decision once — `decide`'s `task` answer opens it through the same
//! opener (`common::missing_task`). The refusals that follow a ruling carry
//! it (`ruled:`), so the same declaration never pays a second decision; a
//! session that declares it again past [`RULING_RESENT_MAX`] refusals goes
//! to a person, gate or none. A slice whose branch a closed task built is
//! never opened again — one branch is one feature — and is refused back.
//!
//! A halt of any other kind — a gate that refused, an unreadable input — is
//! not a decision to take, and goes on as it was.

use std::rc::Rc;

use harness_architecture::graph::{self, Graph};
use harness_core::domain::{Halt, Issue, Outcome, Verdict};
use harness_core::execution::Context;
use harness_core::ports::shell::github::GitHub;
use harness_core::traces::Event;

use crate::common::decision::Question;
use crate::common::{contract_ask, labels, missing_task, sections};
use crate::contract_manager::data::request as contract;
use crate::dev_loop::data::missing::{self, Asked};
use crate::dev_loop::data::state::Loop;
use crate::dev_loop::data::{tasks, waits};

/// Whether a stop's reason asks for a decision in so many words.
#[must_use]
fn asks_a_decision(reason: &str) -> bool {
    let lower = reason.to_lowercase();
    [
        "needs a decision",
        "need a decision",
        "a decision is needed",
        "decision needed",
        "spec contradicts",
    ]
    .iter()
    .any(|said| lower.contains(said))
}

/// The round a stop on one task goes to a human instead of a decision.
pub const HUMAN_AT_ROUND: u32 = 3;

/// The most of a stop's reason the question keeps, in characters.
const REASON_MAX: usize = 600;

/// What kind of decision a halt asks for, if any: a port to approve, a
/// session's own stop, or the breaker's refusal.
#[must_use]
pub fn kind_of(halt: &Halt) -> Option<&'static str> {
    let Halt::Halted(reason) = halt else {
        return None;
    };
    if contract::asked(reason).is_some() {
        Some("contract")
    } else if port_asked(reason).is_some() {
        Some("port")
    } else if reason.contains("AGENT_LOOP_STOP") {
        Some("stop")
    } else if reason.starts_with("refusing to pay for") {
        Some("breaker")
    } else {
        None
    }
}

/// The port a stop asks for — `payments — Stripe, 2.9%, to charge` — when
/// it asks for one.
fn port_asked(reason: &str) -> Option<&str> {
    let (_, after) = reason.split_once("AGENT_LOOP_STOP")?;
    let asked = after
        .trim_start_matches(':')
        .trim_start()
        .strip_prefix("port ")?
        .trim();
    (!asked.is_empty()).then_some(asked)
}

/// A stop's reason, on one line, cut to what a question holds.
fn one_line(reason: &str) -> String {
    let line = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    line.chars().take(REASON_MAX).collect()
}

/// The `## Decision` section: the question, then `extra` — words for the
/// next session or the decision — under it.
fn with_extra(question: &Question, extra: &str) -> String {
    if extra.is_empty() {
        question.render()
    } else {
        format!("{}\n\n{extra}", question.render())
    }
}

/// What a stop declared, sorted by what becomes of it.
struct Sorted {
    /// Whole slices, to open under the milestone.
    writable: Vec<crate::split::data::plan::TaskItem>,
    /// Slices refused back to the session: title, why.
    unopened: Vec<(String, String)>,
    /// Slices a person must open, or decide on: what was said, why.
    personal: Vec<(String, String)>,
}

/// Sorts a stop's slices: whole ones to open (with a milestone to open
/// under), wrong ones refused back to the session — a person's once
/// declared wrong [`HUMAN_AT_ROUND`] times — and a person's own.
fn sorted(asked: Vec<Asked>, milestone: Option<u64>, round: u32) -> Sorted {
    let mut found = Sorted {
        writable: Vec::new(),
        unopened: Vec::new(),
        personal: Vec::new(),
    };
    for one in asked {
        match (one, milestone) {
            (Asked::Task(item), Some(_)) => found.writable.push(*item),
            (Asked::Task(item), None) => found.personal.push((
                item.title.clone(),
                "the task's milestone is not known, so the missing task would belong to none"
                    .to_string(),
            )),
            (Asked::Refused { said, why, .. }, _) if round >= HUMAN_AT_ROUND => {
                found.personal.push((
                    said,
                    format!("{why} — declared wrong {round} times, the session cannot fix it"),
                ));
            }
            (Asked::Refused { title, why, .. }, _) => found.unopened.push((title, why)),
            (Asked::Unwritable { said, why }, _) => found.personal.push((said, why)),
        }
    }
    found
}

/// A ruling `decide` gave on a declaration: its verdict, and its round.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ruling {
    /// What `decide` decided.
    verdict: String,
    /// The round it decided at.
    at: u32,
}

/// The most times a ruling goes back to a session that declares the same
/// tasks again; the next identical stop is a person's, gate or none — a
/// session that ignores a ruling is no decision to pay for again.
const RULING_RESENT_MAX: u32 = 2;

/// What `decide` ruled on these very slices: the task's last `## Decision`
/// is its answer to the same declaration, or a refusal that carried that
/// answer back to the session.
fn ruled_on(previous: Option<&Question>, declared: &[String]) -> Option<Ruling> {
    let q = previous.filter(|q| !q.declared.is_empty() && q.declared == declared)?;
    if q.decided_by.as_deref() == Some("ai") {
        return Some(Ruling {
            verdict: q.verdict.clone().unwrap_or_default(),
            at: q.round,
        });
    }
    q.ruled.clone().map(|verdict| Ruling {
        verdict,
        at: q.ruled_at.unwrap_or(q.round),
    })
}

/// The note, then the stop whole, quoted, for the decision to read the
/// slices from.
fn with_the_stop(note: &str, whole: &str) -> String {
    let quoted: Vec<String> = whole.lines().map(|line| format!("> {line}")).collect();
    format!(
        "{note}{}The stop, whole:\n\n{}",
        if note.is_empty() { "" } else { "\n\n" },
        quoted.join("\n")
    )
}

/// The issues waited on, as a sentence names them: `#1, #2`.
fn waited(on: &[u64]) -> String {
    on.iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the session is told of the declared tasks the harness did not
/// open, one line each — its title and why — or nothing when all were.
fn refusal_note(unopened: &[(String, String)]) -> String {
    if unopened.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = unopened
        .iter()
        .map(|(title, why)| {
            format!(
                "- \"{title}\": {}",
                why.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        })
        .collect();
    format!(
        "**Declared tasks the harness did not open** — declare each again, fixed, in one \
         `AGENT_LOOP_STOP: tasks [...]` (the unit one of: {}), or build on without it if it \
         is not missing after all:\n\n{}",
        missing::unit_keys(),
        lines.join("\n")
    )
}

/// Hands a stopped task over: a decision asked, or a human task opened.
pub struct RaiseDecision {
    /// What reads the task and writes its decision.
    pub gh: Rc<dyn GitHub>,
    /// `--no-human-gate`: no task is ever opened for a person; the harness
    /// decides, and documents it.
    pub autonomous: bool,
    /// The branch the plan's graph is read on (`plan/graph.json`), to tell
    /// which milestone produces what a stop says is missing.
    pub integration_branch: String,
}

impl RaiseDecision {
    /// What the round does with `halt`: a stop or a refusal is handed over,
    /// and the round goes on; anything else is propagated.
    ///
    /// # Errors
    /// `halt` itself when it is no decision to take, or the GitHub writes'.
    pub async fn on_halt(&self, ctx: &mut Context<Loop>, halt: Halt) -> Outcome<Verdict> {
        let Some(kind) = kind_of(&halt) else {
            return Err(halt);
        };
        if ctx.settings.dry_run || !ctx.state.has_task() {
            return Err(halt);
        }
        let Ok(number) = ctx.state.task.number.parse::<u64>() else {
            return Err(halt);
        };
        let reason = one_line(halt.reason());
        // Whole, not cut: a missing task's slice is longer than a question.
        let whole = halt.reason().to_string();
        let task_asked = missing::asked_all(&whole);
        let issue = self.gh.issue(number).await?;
        let previous = sections::parse(&issue.body)
            .get(sections::DECISION)
            .and_then(|text| Question::parse(text));
        let round = previous.as_ref().map_or(1, |q| match q.kind.as_deref() {
            // A refusal sent the session back to declare again: a round.
            Some("stop" | "breaker" | "refused") => q.round.saturating_add(1),
            // A wait decided nothing: the round it kept is this stop's, so
            // the third stop still goes to a human.
            Some("wait") => q.round,
            _ => 1,
        });
        // A port is a person's to approve, with or without a human gate: a
        // new dependency outside the product — a provider, a cost, an
        // account — is no decision the harness takes (ForgeDesMondes' #53,
        // 2026-10-18: `port archive-storage` asked a decision, and no
        // "Approve the port" issue reached the Ports pane).
        let port = port_asked(&reason);
        if let Some((id, need)) = contract::asked(&reason) {
            // A contract another milestone builds — landed or not — is the
            // contract manager's, with or without a human gate: it is the
            // harness's own work (`common::contract_ask`). One the task may
            // evolve itself — an aggregate, whose invariants are the writer's
            // (contract I), or its own milestone's — is answered here:
            // ForgeDesMondes' #157 asked `Session` changed, an aggregate, and
            // the manager, which reads capability and data-capability
            // manifests only, would have halted on it every tick.
            let requester = contract_ask::Requester {
                issue: number,
                milestone: ctx.state.milestone.number.parse::<u64>().ok(),
                body: issue.body.clone(),
                task: true,
            };
            if let contract_ask::Asked::TasksOwn(why) = contract_ask::ask(
                self.gh.as_ref(),
                &requester,
                (&id, &need),
                &self.integration_branch,
                &ctx.traces,
            )
            .await?
            {
                self.contract_answered(ctx, number, &issue.body, &id, &need, &why)
                    .await?;
            }
        } else if let Some(declared) = task_asked {
            // A missing task is the harness's own work, with or without a
            // human gate: it opens it, and the task waits on it.
            self.missing_task(
                ctx,
                &issue,
                declared,
                previous.as_ref(),
                round,
                &reason,
                &whole,
            )
            .await?;
        } else if let Some(port) = port {
            self.port_to_approve(ctx, number, &issue.title, &issue.body, port)
                .await?;
        } else if let Some(on) = if asks_a_decision(&reason) {
            // A stop that says it needs a decision is one, whatever issues it
            // names on the way (ForgeDesMondes #182: "SPEC contradicts the
            // engine … same pattern in #188 … needs a Decision" waited on
            // #188, which will stop on the same question).
            None
        } else {
            self.waited_for(ctx, number, &reason).await
        } {
            // Something another issue produces has not landed: the task
            // waits for it, for free, however many times it was asked. A
            // blocker GitHub refuses (ForgeDesMondes' #248 named PR #264,
            // which is no issue a dependency can point at) is no wait: the
            // stop asks a decision as before, the round is not lost.
            if let Err(refused) = self
                .wait_for(ctx, number, &issue.body, &[on], round, &reason, "")
                .await
            {
                ctx.traces.warn(&format!(
                    "#{number}: cannot wait for #{on} — {} — a decision is asked instead",
                    refused.reason()
                ));
                self.ask(ctx, number, &issue.body, kind, round, &reason)
                    .await?;
            }
        } else if round >= HUMAN_AT_ROUND && !self.autonomous {
            self.to_a_human(ctx, number, &issue.title, &reason).await?;
        } else {
            self.ask(ctx, number, &issue.body, kind, round, &reason)
                .await?;
        }
        ctx.state.handed_over = true;
        Ok(Verdict::Continue)
    }

    /// Writes the stop under the task's `## Decision` and asks `decide` for
    /// an answer at the next tick (`harness:needs-decision`).
    async fn ask(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        body: &str,
        kind: &str,
        round: u32,
        reason: &str,
    ) -> Outcome<()> {
        let question = Question {
            kind: Some(kind.to_string()),
            blocks: reason.to_string(),
            round,
            ..Question::default()
        };
        self.write_asked(ctx, number, body, &question, "").await
    }

    /// Writes `question`, `extra` under it, as the task's `## Decision`, and
    /// asks `decide` for an answer at the next tick.
    async fn write_asked(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        body: &str,
        question: &Question,
        extra: &str,
    ) -> Outcome<()> {
        let body = sections::replace(body, &[(sections::DECISION, &with_extra(question, extra))]);
        self.gh.set_body(number, &body).await?;
        self.gh.add_label(number, labels::NEEDS_DECISION).await?;
        ctx.traces.say(&format!(
            "#{number}: the session stopped ({}, round {}) — a decision is asked: {}",
            question.kind.as_deref().unwrap_or("stop"),
            question.round,
            question.blocks
        ));
        Ok(())
    }

    /// The issue the stop's reason names that has not landed — a milestone
    /// or a task, never the stopped task itself, its own milestone, nor a
    /// roadmap item (it closes after the task's own milestone lands: waiting
    /// on it would never end) — if any. An issue the board cannot read names
    /// nothing.
    async fn waited_for(&self, ctx: &Context<Loop>, number: u64, reason: &str) -> Option<u64> {
        let graph = self
            .gh
            .file_text(graph::PATH, &self.integration_branch)
            .await
            .ok()
            .flatten()
            .and_then(|text| Graph::parse(&text).ok());
        let own = ctx.state.milestone.number.parse::<u64>().ok();
        for candidate in waits::named(reason, graph.as_ref()) {
            if candidate == number || Some(candidate) == own {
                continue;
            }
            // A milestone or an agent task only: a pull request shares the
            // number space and GitHub refuses it as a dependency; a human
            // task or a roadmap item closes long after this task would.
            if let Ok(found) = self.gh.issue(candidate).await
                && (found.has(labels::MILESTONE) || found.has(labels::AGENT))
                && tasks::still_blocks(&found)
            {
                return Some(candidate);
            }
        }
        None
    }

    /// Parks the task until every issue of `on` lands: a GitHub blocker
    /// each, written once, the stop kept under `## Decision` (`kind: wait`,
    /// carrying `round` so the stops decided before it still count), `extra`
    /// under it, and nothing to decide.
    #[allow(clippy::too_many_arguments)] // The stop, and what it waits on.
    async fn wait_for(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        body: &str,
        on: &[u64],
        round: u32,
        reason: &str,
        extra: &str,
    ) -> Outcome<()> {
        missing_task::block_on(self.gh.as_ref(), number, on).await?;
        let named: Vec<String> = on.iter().map(|n| format!("#{n}")).collect();
        let question = Question {
            kind: Some("wait".to_string()),
            blocks: reason.to_string(),
            round,
            decided_by: Some("harness".to_string()),
            verdict: Some(format!(
                "wait — until {} land{}",
                named.join(", "),
                if on.len() == 1 { "s" } else { "" }
            )),
            ..Question::default()
        };
        let kept = sections::replace(body, &[(sections::DECISION, &with_extra(&question, extra))]);
        self.gh.set_body(number, &kept).await?;
        for blocker in on {
            ctx.traces.event(&Event::TaskWaits {
                task: number,
                on: *blocker,
                why: reason.to_string(),
            });
        }
        Ok(())
    }

    /// Tells the task its contract request is its own to carry out: the
    /// answer under `## Decision`, no label, no issue — the loop takes the
    /// task again at the next tick.
    async fn contract_answered(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        body: &str,
        id: &str,
        need: &str,
        why: &str,
    ) -> Outcome<()> {
        let question = Question {
            kind: Some("contract".to_string()),
            blocks: format!("contract {id} — {need}"),
            decided_by: Some("harness".to_string()),
            verdict: Some(format!(
                "answer — {why}: evolve it in this task, versioned — the manifest or \
                 aggregate.json with its version raised, what reads or judges it, their tests"
            )),
            ..Question::default()
        };
        let kept = sections::replace(body, &[(sections::DECISION, &question.render())]);
        self.gh.set_body(number, &kept).await?;
        ctx.traces.say(&format!(
            "#{number}: asked a change of `{id}` — {why}: told to evolve it in the task"
        ));
        Ok(())
    }

    /// A port asked: kept under the task's `## Decision`, and a person's
    /// to approve in a task blocking it.
    async fn port_to_approve(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        title: &str,
        body: &str,
        port: &str,
    ) -> Outcome<()> {
        let question = Question {
            kind: Some("port".to_string()),
            blocks: format!("port {port}"),
            ..Question::default()
        };
        let kept = sections::replace(body, &[(sections::DECISION, &question.render())]);
        self.gh.set_body(number, &kept).await?;
        let id = port.split_whitespace().next().unwrap_or(port);
        let ask = format!(
            "The session building #{number} needs something outside the product that \
             ports.json has not:\n\n> {port}\n\nA new port is a person's call — a provider, a \
             cost, an account.\n\n- **Approve**: close this issue (the plant's Ports pane does \
             it). #{number}'s next session adds the port to ports.json and builds it, its fake \
             and contract tests first.\n- **Refuse**: say why in a comment on #{number} and \
             label it `{decision}` (the Ports pane's Refuse does both). The harness decides how \
             #{number} does without it.",
            decision = labels::NEEDS_DECISION
        );
        let human = self
            .gh
            .create_issue(
                &format!("Approve the port {id} for #{number}: {title}"),
                &ask,
                &[labels::HUMAN],
            )
            .await?;
        if let Ok(milestone) = ctx.state.milestone.number.parse::<u64>() {
            self.gh.create_sub_issue_link(milestone, human).await?;
        }
        self.gh.add_blocked_by(number, human).await?;
        ctx.traces.event(&Event::HumanTaskOpened {
            task: human,
            blocks: number,
            why: format!("approve the port {port}"),
        });
        Ok(())
    }

    /// The missing tasks a stop named: each opened under the milestone — or
    /// found there, named by an earlier stop — and the task parked on all of
    /// them, in one round. Nothing declared is dropped in silence:
    ///
    /// - a slice declared wrong, or whose branch a closed task built
    ///   already, is refused back to the session — a comment and the task's
    ///   `## Decision` say which and why, and the next session re-declares
    ///   it — until it is asked [`HUMAN_AT_ROUND`] times;
    /// - one only a person can do, or refused that often, goes to a person
    ///   in one task listing them all, or, without a human gate, is asked as
    ///   a decision when nothing could be opened — once: the same
    ///   declaration again after `decide` ruled on it is no new fact, and
    ///   goes back to the session with the ruling, which every refusal
    ///   carries; [`RULING_RESENT_MAX`] times, then it is a person's.
    #[allow(clippy::too_many_arguments)] // The stop, as on_halt read it.
    async fn missing_task(
        &self,
        ctx: &Context<Loop>,
        issue: &Issue,
        asked: Vec<Asked>,
        previous: Option<&Question>,
        round: u32,
        reason: &str,
        whole: &str,
    ) -> Outcome<()> {
        let number = issue.number;
        let milestone = ctx.state.milestone.number.parse::<u64>().ok();
        let declared = missing::declared(&asked);
        let ruled = ruled_on(previous, &declared);
        let Sorted {
            writable,
            mut unopened,
            mut personal,
        } = sorted(asked, milestone, round);
        let found = match milestone {
            Some(milestone) => {
                missing_task::open_or_find(
                    self.gh.as_ref(),
                    number,
                    milestone,
                    &writable,
                    &ctx.traces,
                )
                .await?
            }
            None => missing_task::Found::default(),
        };
        let on = found.on;
        // Built already is declared wrong: a round, and a person's at the
        // third, as any refusal.
        for (title, why) in found.built {
            if round >= HUMAN_AT_ROUND {
                personal.push((
                    title,
                    format!("{why} — declared {round} times, the session cannot fix it"),
                ));
            } else {
                unopened.push((title, why));
            }
        }
        let (asks, carried) = self
            .personal(ctx, issue, personal, ruled, round, &on, &mut unopened)
            .await?;
        let note = refusal_note(&unopened);
        self.say_unopened(ctx, number, &unopened, &note).await?;
        if !on.is_empty() {
            // The wait keeps the stop's whole reason, and what was not
            // opened in words: the next stop declares it again.
            self.wait_for(ctx, number, &issue.body, &on, round, reason, &note)
                .await
        } else if asks {
            let question = Question {
                kind: Some("stop".to_string()),
                blocks: reason.to_string(),
                round,
                milestone,
                declared,
                ..Question::default()
            };
            let extra = with_the_stop(&note, whole);
            self.write_asked(ctx, number, &issue.body, &question, &extra)
                .await
        } else if !unopened.is_empty() {
            let (ruled, ruled_at) = carried.map_or((None, None), |ruling| {
                (Some(ruling.verdict), Some(ruling.at))
            });
            let question = Question {
                kind: Some("refused".to_string()),
                blocks: reason.to_string(),
                round,
                decided_by: Some("harness".to_string()),
                verdict: Some(format!(
                    "refused — re-declare {} declared task{}, as said below",
                    unopened.len(),
                    if unopened.len() == 1 { "" } else { "s" }
                )),
                declared,
                ruled,
                ruled_at,
                ..Question::default()
            };
            let body = sections::replace(
                &issue.body,
                &[(sections::DECISION, &with_extra(&question, &note))],
            );
            self.gh.set_body(number, &body).await
        } else {
            Ok(())
        }
    }

    /// What becomes of the slices only a person can do, or refused too
    /// often: a person's task with a human gate; without, a decision asked
    /// when nothing was opened (`true`), or the ruling already given sent
    /// back (the ruling, carried), or said in the note beside the opened
    /// ones — and a person's once the ruling was ignored too often.
    #[allow(clippy::too_many_arguments)] // The sorted stop, as missing_task holds it.
    async fn personal(
        &self,
        ctx: &Context<Loop>,
        issue: &Issue,
        personal: Vec<(String, String)>,
        ruled: Option<Ruling>,
        round: u32,
        on: &[u64],
        unopened: &mut Vec<(String, String)>,
    ) -> Outcome<(bool, Option<Ruling>)> {
        if personal.is_empty() {
            return Ok((false, None));
        }
        if !self.autonomous {
            self.unwritable(ctx, issue, &personal).await?;
            return Ok((false, None));
        }
        let Some(ruling) = ruled else {
            if on.is_empty() {
                return Ok((true, None));
            }
            let landing = waited(on);
            unopened.extend(personal.into_iter().map(|(said, why)| {
                (
                    missing::title_of(&said),
                    format!("{why} — asked again once {landing} land"),
                )
            }));
            return Ok((false, None));
        };
        let again = round.saturating_sub(ruling.at);
        if again > RULING_RESENT_MAX {
            let ignored: Vec<(String, String)> = personal
                .into_iter()
                .map(|(said, why)| {
                    (
                        said,
                        format!(
                            "{why} — declared again {again} times after the harness decided: {}",
                            ruling.verdict
                        ),
                    )
                })
                .collect();
            self.unwritable(ctx, issue, &ignored).await?;
            return Ok((false, None));
        }
        unopened.extend(personal.into_iter().map(|(said, why)| {
            (
                missing::title_of(&said),
                format!(
                    "{why} — the harness decided on it already: {}",
                    ruling.verdict
                ),
            )
        }));
        Ok((false, Some(ruling)))
    }

    /// Says, in the journal and in a comment on the task, each declared
    /// task the harness did not open, and why.
    async fn say_unopened(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        unopened: &[(String, String)],
        note: &str,
    ) -> Outcome<()> {
        if unopened.is_empty() {
            return Ok(());
        }
        for (title, why) in unopened {
            ctx.traces.warn(&format!(
                "#{number}: the declared task \"{title}\" is not opened — {why}"
            ));
        }
        self.gh
            .post_issue_comment(
                number,
                &format!("{note}\n\nThe next session of #{number} reads this under `## Decision`."),
            )
            .await
    }

    /// The missing tasks with no SPEC to write: a person's to open, in one
    /// task blocking the stopped one.
    async fn unwritable(
        &self,
        ctx: &Context<Loop>,
        issue: &Issue,
        unwritable: &[(String, String)],
    ) -> Outcome<()> {
        let number = issue.number;
        let listed: Vec<String> = unwritable
            .iter()
            .map(|(said, why)| format!("- {why}:\n\n  > {said}"))
            .collect();
        let body = format!(
            "## What a human must decide or schedule\n\nThe session building #{number} stopped \
             on {} the plan lacks, and the harness could not open {} itself:\n\n{}\n\n## \
             Missing task\n\nOpen each as a `{agent}` sub-issue of the milestone with its SPEC \
             (`## Scope` with a `branch:`, `## Architecture`, `## Business Goal`, `## \
             Acceptance Criteria`) and declare it a dependency of #{number}, or answer under \
             #{number}'s `## Decision`; then close this issue — #{number} waits on it.",
            if unwritable.len() == 1 {
                "a task"
            } else {
                "tasks"
            },
            if unwritable.len() == 1 { "it" } else { "them" },
            listed.join("\n"),
            agent = labels::AGENT
        );
        let first = unwritable.first().map_or("", |(said, _)| said.as_str());
        let title: String = first.chars().take(80).collect();
        let human = self
            .gh
            .create_issue(
                &format!("Missing task for #{number}: {title}"),
                &body,
                &[labels::HUMAN],
            )
            .await?;
        if let Ok(milestone) = ctx.state.milestone.number.parse::<u64>() {
            self.gh.create_sub_issue_link(milestone, human).await?;
        }
        self.gh.add_blocked_by(number, human).await?;
        let whys: Vec<&str> = unwritable.iter().map(|(_, why)| why.as_str()).collect();
        ctx.traces.event(&Event::HumanTaskOpened {
            task: human,
            blocks: number,
            why: format!(
                "a missing task the harness cannot write: {}",
                whys.join("; ")
            ),
        });
        Ok(())
    }

    /// A stop the harness already decided twice: a task only a human can
    /// settle, blocking the stopped one.
    async fn to_a_human(
        &self,
        ctx: &Context<Loop>,
        number: u64,
        title: &str,
        reason: &str,
    ) -> Outcome<()> {
        let body = format!(
            "The session building #{number} stopped again after the harness decided twice:\n\n\
             > {reason}\n\nOnly a person can settle it: answer under #{number}'s `## Decision` \
             with `decided-by: owner`, then close this issue — #{number} waits on it."
        );
        let human = self
            .gh
            .create_issue(
                &format!("Decide for #{number}: {title}"),
                &body,
                &[labels::HUMAN],
            )
            .await?;
        if let Ok(milestone) = ctx.state.milestone.number.parse::<u64>() {
            self.gh.create_sub_issue_link(milestone, human).await?;
        }
        self.gh.add_blocked_by(number, human).await?;
        ctx.traces.event(&Event::HumanTaskOpened {
            task: human,
            blocks: number,
            why: format!("#{number} stopped a third time: {reason}"),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fake_github::{FakeGitHub, Wrote};
    use harness_core::domain::Issue;
    use harness_core::domain::Named;
    use harness_core::execution::Settings;
    use harness_core::traces::Logbook;

    fn ctx(dry_run: bool) -> Context<Loop> {
        Context::new(
            Settings {
                dry_run,
                stages: String::new(),
            },
            Loop {
                milestone: Named {
                    number: "4".to_string(),
                    ..Named::default()
                },
                task: Named {
                    number: "11".to_string(),
                    title: "Pay".to_string(),
                    body: String::new(),
                },
                task_key: "11".to_string(),
                ..Loop::default()
            },
            Logbook::null(),
        )
    }

    fn gh(body: &str) -> Rc<FakeGitHub> {
        Rc::new(FakeGitHub {
            issues: vec![Issue {
                number: 11,
                title: "Pay".to_string(),
                state: "open".to_string(),
                body: body.to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        })
    }

    fn stop() -> Halt {
        Halt::Halted("AGENT_LOOP_STOP: which payment provider? (/code)".to_string())
    }

    #[test]
    fn only_a_stop_or_a_refusal_is_a_decision_to_take() {
        assert_eq!(kind_of(&stop()), Some("stop"));
        assert_eq!(
            kind_of(&Halt::Halted(
                "refusing to pay for /code on #11: …".to_string()
            )),
            Some("breaker")
        );
        assert_eq!(
            kind_of(&Halt::Halted("issue #11 carries no SPEC".to_string())),
            None
        );
        assert_eq!(kind_of(&Halt::Failed("AGENT_LOOP_STOP".to_string())), None);
    }

    #[tokio::test]
    async fn a_stop_asks_for_a_decision_and_the_round_goes_on() {
        let gh = gh("## Scope\n\nbranch: feat/pay\n\npay");
        let mut context = ctx(false);
        let verdict = RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: false,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(&mut context, stop())
        .await
        .expect("handed over");
        assert_eq!(verdict, Verdict::Continue);
        assert!(context.state.handed_over);
        let writes = gh.writes();
        let Wrote::Body(11, body) = &writes[0] else {
            panic!("{writes:?}");
        };
        let asked = Question::parse(
            sections::parse(body)
                .get(sections::DECISION)
                .map_or("", String::as_str),
        )
        .expect("a question");
        assert_eq!(asked.kind.as_deref(), Some("stop"));
        assert_eq!(asked.round, 1);
        assert!(asked.blocks.contains("which payment provider"));
        assert!(body.contains("branch: feat/pay"));
        assert_eq!(
            writes[1],
            Wrote::Label(11, labels::NEEDS_DECISION.to_string())
        );
    }

    #[tokio::test]
    async fn a_third_stop_opens_a_task_for_a_human_that_blocks_it() {
        let gh = gh(
            "## Decision\n\nkind: stop\nblocks: which provider\nround: 2\ndecided-by: ai\nverdict: answer — Stripe (3/5)",
        );
        let mut context = ctx(false);
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: false,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(&mut context, stop())
        .await
        .expect("handed over");
        let writes = gh.writes();
        assert!(
            matches!(&writes[0], Wrote::CreatedIssue(title, _, labels) if title == "Decide for #11: Pay" && labels == &[labels::HUMAN.to_string()])
        );
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, Wrote::SubIssueLink(4, _)))
        );
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, Wrote::BlockedByLink(11, _)))
        );
        assert!(
            !writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())),
            "no third decision"
        );
        assert!(context.state.handed_over);
    }

    #[tokio::test]
    async fn without_a_human_gate_a_third_stop_is_a_decision_and_a_port_still_a_person_s() {
        // The owner, on ForgeDesMondes: "I cannot take the decisions — you
        // do it." #41's third stop opened #79 for a person who never comes.
        let third = gh(
            "## Decision\n\nkind: stop\nblocks: which provider\nround: 2\ndecided-by: ai\nverdict: answer — Stripe (3/5)",
        );
        let mut context = ctx(false);
        RaiseDecision {
            gh: Rc::clone(&third) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(&mut context, stop())
        .await
        .expect("handed over");
        let writes = third.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::CreatedIssue(..))),
            "{writes:?}"
        );
        assert!(writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
        let Wrote::Body(11, body) = &writes[0] else {
            panic!("{writes:?}");
        };
        assert!(body.contains("round: 3"), "{body}");

        let asked = Halt::Halted(
            "AGENT_LOOP_STOP: port payments — Stripe, 2.9% a charge, to bill (/code)".to_string(),
        );
        let port = gh("## Scope\n\nbranch: feat/pay\n\npay");
        RaiseDecision {
            gh: Rc::clone(&port) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(&mut ctx(false), asked)
        .await
        .expect("handed over");
        // A port stays a person's, gate or no gate (ForgeDesMondes' #53):
        // the "Approve the port" issue the Ports pane reads, no decision.
        let writes = port.writes();
        assert!(
            writes.iter().any(|w| matches!(
                w,
                Wrote::CreatedIssue(title, _, labels)
                    if title.starts_with("Approve the port payments for #11")
                        && labels == &[labels::HUMAN.to_string()]
            )),
            "{writes:?}"
        );
        assert!(!writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
        let Wrote::Body(11, body) = &writes[0] else {
            panic!("{writes:?}");
        };
        assert!(body.contains("kind: port"), "{body}");
    }

    #[tokio::test]
    async fn a_port_asked_is_a_person_s_to_approve_at_once() {
        let asked = Halt::Halted(
            "AGENT_LOOP_STOP: port payments — Stripe, 2.9% a charge, to bill (/code)".to_string(),
        );
        assert_eq!(kind_of(&asked), Some("port"));
        assert_eq!(
            kind_of(&Halt::Halted("AGENT_LOOP_STOP: port ".to_string())),
            Some("stop"),
            "no port named"
        );
        let gh = gh("## Scope\n\nbranch: feat/pay\n\npay");
        let mut context = ctx(false);
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: false,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(&mut context, asked)
        .await
        .expect("handed over");
        assert!(context.state.handed_over);
        let writes = gh.writes();
        let Wrote::Body(11, body) = &writes[0] else {
            panic!("{writes:?}");
        };
        let kept = Question::parse(
            sections::parse(body)
                .get(sections::DECISION)
                .map_or("", String::as_str),
        )
        .expect("a question");
        assert_eq!(kept.kind.as_deref(), Some("port"));
        assert_eq!(kept.round, 1);
        assert!(
            kept.blocks
                .starts_with("port payments — Stripe, 2.9% a charge"),
            "{}",
            kept.blocks
        );
        assert!(
            matches!(&writes[1], Wrote::CreatedIssue(title, text, labels)
                if title == "Approve the port payments for #11: Pay"
                    && text.contains("> payments — Stripe")
                    && labels == &[labels::HUMAN.to_string()]),
            "{writes:?}"
        );
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, Wrote::SubIssueLink(4, _)))
        );
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, Wrote::BlockedByLink(11, _)))
        );
        assert!(
            !writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())),
            "no decision: a person approves"
        );
    }

    #[tokio::test]
    async fn any_other_halt_or_a_dry_run_goes_on_as_it_was() {
        let gh = gh("");
        let raise = RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: false,
            integration_branch: "main_agent".to_string(),
        };
        let gate = Halt::Halted("issue #11 carries no SPEC".to_string());
        assert!(raise.on_halt(&mut ctx(false), gate).await.is_err());
        assert!(raise.on_halt(&mut ctx(true), stop()).await.is_err());
        let seen = &gh.writes();
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[tokio::test]
    async fn a_contract_asked_goes_to_the_contract_manager_with_or_without_a_human_gate() {
        let asked = || {
            Halt::Halted(
                "AGENT_LOOP_STOP: contract session.listScenes — the scenes' media ids (/code)"
                    .to_string(),
            )
        };
        assert_eq!(kind_of(&asked()), Some("contract"));
        for autonomous in [false, true] {
            let gh = gh("## Scope\n\nbranch: feat/media\n\nmedia");
            let mut context = ctx(false);
            RaiseDecision {
                gh: Rc::clone(&gh) as Rc<dyn GitHub>,
                autonomous,
                integration_branch: "main_agent".to_string(),
            }
            .on_halt(&mut context, asked())
            .await
            .expect("handed over");
            let writes = gh.writes();
            let Wrote::Body(11, body) = &writes[0] else {
                panic!("{writes:?}");
            };
            assert!(body.contains("kind: contract"), "{body}");
            assert!(
                body.contains(
                    "blocks: contract session.listScenes — the scenes' media ids (/code)"
                ),
                "{body}"
            );
            assert!(
                writes.iter().any(|w| matches!(w, Wrote::CreatedLabel(name, colour, said) if name == labels::CONTRACT_CHANGE && colour == "bfdadc" && said.contains("contract manager"))),
                "the label is created on a board that lacks it, as the table describes it: {writes:?}"
            );
            let created = writes
                .iter()
                .find_map(|w| match w {
                    Wrote::CreatedIssue(title, body, labels) => Some((title, body, labels)),
                    _ => None,
                })
                .expect("the request");
            assert_eq!(created.0, "Contract change: session.listScenes for #11");
            assert!(
                created.1.contains("id: session.listScenes") && created.1.contains("milestone: #4")
            );
            assert!(created.1.contains("need: the scenes' media ids (/code)"));
            assert_eq!(created.2, &[labels::CONTRACT_CHANGE.to_string()]);
            assert!(
                writes
                    .iter()
                    .any(|w| matches!(w, Wrote::BlockedByLink(11, _)))
            );
            assert!(
                !writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())),
                "no decision asked: the manager answers"
            );
            assert!(context.state.handed_over);
        }
        // A board that has the label gets no second one.
        let labelled = Rc::new(FakeGitHub {
            labels: vec![labels::CONTRACT_CHANGE.to_string()],
            issues: vec![Issue {
                number: 11,
                title: "Pay".to_string(),
                state: "open".to_string(),
                body: "## Scope\n\nbranch: feat/media".to_string(),
                ..Issue::default()
            }],
            ..FakeGitHub::default()
        });
        RaiseDecision {
            gh: Rc::clone(&labelled) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(&mut ctx(false), asked())
        .await
        .expect("handed over");
        assert!(
            !labelled
                .writes()
                .iter()
                .any(|w| matches!(w, Wrote::CreatedLabel(..))),
            "{:?}",
            labelled.writes()
        );
    }

    // --- a stop on what has not landed waits, for free ----------------------

    /// The plan: #20 produces the session system, #18 the `Entity`
    /// aggregate; the task is #11 of milestone #4.
    const GRAPH: &str = r#"{"version":1,"milestones":[
      {"number":18,"title":"notes","systems":["note"],"produces":["Entity"],"consumes":[],"depends_on":[]},
      {"number":20,"title":"sessions","systems":["session"],"produces":["Session"],"consumes":[],"depends_on":[]},
      {"number":4,"title":"pay","systems":["pay"],"produces":[],"consumes":["Session"],"depends_on":[20]}
    ],"ids":{
      "Entity":{"kind":"aggregate","producer":18,"state":"planned","version":1},
      "Session":{"kind":"aggregate","producer":20,"state":"planned","version":1}
    }}"#;

    /// A board with the task, milestone #20 as `labels` say it, and the
    /// plan's graph on the integration branch.
    fn board(body: &str, producer_labels: &[&str], producer_state: &str) -> Rc<FakeGitHub> {
        let producer = Issue {
            number: 20,
            title: "sessions".to_string(),
            state: producer_state.to_string(),
            labels: producer_labels.iter().map(ToString::to_string).collect(),
            ..Issue::default()
        };
        Rc::new(FakeGitHub {
            issues: vec![
                Issue {
                    number: 11,
                    title: "Pay".to_string(),
                    state: "open".to_string(),
                    body: body.to_string(),
                    labels: vec![labels::AGENT.to_string(), labels::READY.to_string()],
                    ..Issue::default()
                },
                producer,
            ],
            files: std::collections::HashMap::from([(
                ("plan/graph.json".to_string(), "main_agent".to_string()),
                GRAPH.to_string(),
            )]),
            ..FakeGitHub::default()
        })
    }

    #[derive(Default)]
    struct Capture(std::cell::RefCell<Vec<String>>);

    impl harness_core::traces::Sink for Capture {
        fn emit(&self, line: &str) {
            self.0.borrow_mut().push(line.to_string());
        }
    }

    #[tokio::test]
    async fn a_stop_on_a_producer_not_landed_parks_the_task_until_it_lands() {
        // #214 and #215 on ForgeDesMondes: 23 code sessions and 23 decide
        // sessions each, every decision "keep waiting for #20".
        let gh = board(
            "## Scope\n\nbranch: feat/pay\n\npay",
            &[labels::MILESTONE],
            "open",
        );
        let capture = Rc::new(Capture::default());
        let mut context = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            ctx(false).state,
            Logbook::new(
                Rc::clone(&capture) as Rc<dyn harness_core::traces::Sink>,
                harness_core::traces::Verbosity::Normal,
            ),
        );
        let verdict = RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: false,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut context,
            Halt::Halted(
                "AGENT_LOOP_STOP: wait for #20 — origin/main_agent has no crates/session/ (/code)"
                    .to_string(),
            ),
        )
        .await
        .expect("handed over");
        assert_eq!(verdict, Verdict::Continue);
        assert!(context.state.handed_over);
        let writes = gh.writes();
        assert_eq!(writes[0], Wrote::BlockedByLink(11, 20));
        let Wrote::Body(11, body) = &writes[1] else {
            panic!("{writes:?}");
        };
        let kept = Question::parse(
            sections::parse(body)
                .get(sections::DECISION)
                .map_or("", String::as_str),
        )
        .expect("a question");
        assert_eq!(kept.kind.as_deref(), Some("wait"));
        assert_eq!(kept.decided_by.as_deref(), Some("harness"));
        assert_eq!(kept.verdict.as_deref(), Some("wait — until #20 lands"));
        assert!(
            kept.blocks.contains("no crates/session/"),
            "{}",
            kept.blocks
        );
        assert!(
            body.contains("branch: feat/pay"),
            "the rest of the SPEC kept"
        );
        assert_eq!(writes.len(), 2, "no label, no issue: {writes:?}");
        assert!(
            capture.0.borrow().iter().any(|line| line
                .contains("#11 parked until #20 lands (wait): AGENT_LOOP_STOP: wait for #20")),
            "{:?}",
            capture.0.borrow()
        );

        // The next tick: blocked by #20, the task is not startable — no
        // session, no decision — until #20 lands, when it is again.
        let mut task = gh.issues[0].clone();
        task.blocked_by = vec![gh.issues[1].clone()];
        assert!(!task.has(labels::NEEDS_DECISION));
        assert!(
            tasks::startable_tasks(std::slice::from_ref(&task), &[], None, &|_| true).is_empty(),
            "parked"
        );
        task.blocked_by[0]
            .labels
            .push(labels::WAITING_MERGE.to_string());
        assert_eq!(
            tasks::startable_tasks(std::slice::from_ref(&task), &[], None, &|_| true)
                .iter()
                .map(|t| t.number)
                .collect::<Vec<_>>(),
            [11],
            "#20 landed: taken again"
        );
    }

    #[tokio::test]
    async fn a_wait_outranks_the_third_stop_and_is_written_once() {
        // A third stop would open a task for a human; one that names a
        // producer waits instead, and a blocker already there is not posted
        // twice.
        let gh = board(
            "## Decision\n\nkind: stop\nblocks: which provider\nround: 2\ndecided-by: ai\nverdict: answer — Stripe (3/5)",
            &[labels::MILESTONE],
            "open",
        );
        let mut context = ctx(false);
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: false,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut context,
            Halt::Halted("AGENT_LOOP_STOP: milestone/20-sessions is not merged yet".to_string()),
        )
        .await
        .expect("handed over");
        let writes = gh.writes();
        assert_eq!(writes[0], Wrote::BlockedByLink(11, 20));
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::CreatedIssue(..))),
            "{writes:?}"
        );
        assert!(!writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
        let Wrote::Body(11, waited) = &writes[1] else {
            panic!("{writes:?}");
        };
        let kept = Question::parse(
            sections::parse(waited)
                .get(sections::DECISION)
                .map_or("", String::as_str),
        )
        .expect("a question");
        assert_eq!(kept.round, 3, "the two stops decided before it still count");

        // The next stop, on nothing to wait for: the third, a person's.
        let after = board(waited, &[labels::MILESTONE], "open");
        RaiseDecision {
            gh: Rc::clone(&after) as Rc<dyn GitHub>,
            autonomous: false,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(&mut ctx(false), stop())
        .await
        .expect("handed over");
        assert!(
            after
                .writes()
                .iter()
                .any(|w| matches!(w, Wrote::CreatedIssue(title, _, l) if title.starts_with("Decide for #11") && l == &[labels::HUMAN])),
            "{:?}",
            after.writes()
        );

        let mut blocked = gh.issues[0].clone();
        blocked.blocked_by = vec![gh.issues[1].clone()];
        let already = Rc::new(FakeGitHub {
            issues: gh.issues.clone(),
            subs: vec![(4, vec![blocked])],
            ..FakeGitHub::default()
        });
        RaiseDecision {
            gh: Rc::clone(&already) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut ctx(false),
            Halt::Halted("AGENT_LOOP_STOP: still waiting for #20".to_string()),
        )
        .await
        .expect("handed over");
        let writes = already.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::BlockedByLink(..))),
            "{writes:?}"
        );
        assert!(matches!(&writes[0], Wrote::Body(11, body) if body.contains("kind: wait")));
    }

    #[tokio::test]
    async fn what_the_graph_attributes_to_a_milestone_is_waited_for() {
        // No issue number in the reason: the path names #20's system, the
        // migration names #18's aggregate — the graph says whose they are.
        for (reason, producer) in [
            (
                "AGENT_LOOP_STOP: origin/main_agent has no crates/session/ yet",
                20,
            ),
            (
                "AGENT_LOOP_STOP: missing crates/data/migrations/0004_entity.sql",
                18,
            ),
        ] {
            let gh = board("pay", &[labels::MILESTONE], "open");
            let mut issues = gh.issues.clone();
            issues.push(Issue {
                number: 18,
                state: "open".to_string(),
                labels: vec![labels::MILESTONE.to_string()],
                ..Issue::default()
            });
            let gh = Rc::new(FakeGitHub {
                issues,
                files: gh.files.clone(),
                ..FakeGitHub::default()
            });
            RaiseDecision {
                gh: Rc::clone(&gh) as Rc<dyn GitHub>,
                autonomous: false,
                integration_branch: "main_agent".to_string(),
            }
            .on_halt(&mut ctx(false), Halt::Halted(reason.to_string()))
            .await
            .expect("handed over");
            assert_eq!(
                gh.writes()[0],
                Wrote::BlockedByLink(11, producer),
                "{reason}"
            );
        }
    }

    #[tokio::test]
    async fn a_contract_the_task_may_evolve_itself_is_answered_not_managed() {
        // ForgeDesMondes' #157: `Session`, an aggregate — the manager reads
        // capability and data-capability manifests only.
        let gh = board("pay", &[labels::MILESTONE], "open");
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut ctx(false),
            Halt::Halted(
                "AGENT_LOOP_STOP: contract Session — declare oneshot-single-session (/code)"
                    .to_string(),
            ),
        )
        .await
        .expect("handed over");
        let writes = gh.writes();
        assert!(
            !writes
                .iter()
                .any(|w| matches!(w, Wrote::CreatedIssue(..) | Wrote::BlockedByLink(..))),
            "no manager issue for an aggregate: {writes:?}"
        );
        assert!(
            writes.iter().any(|w| matches!(w, Wrote::Body(11, body) if body.contains("decided-by: harness") && body.contains("is an aggregate"))),
            "the task is told: {writes:?}"
        );
        assert!(
            !writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())),
            "no decision to pay: {writes:?}"
        );
        // A capability another milestone still builds stays the manager's.
        let gh = board("pay", &[labels::MILESTONE], "open");
        let mut files = gh.files.clone();
        files.insert(
            ("plan/graph.json".to_string(), "main_agent".to_string()),
            GRAPH.replace(
                r#""Session":{"kind":"aggregate""#,
                r#""session.listScenes":{"kind":"capability","producer":20,"state":"planned","version":1},"Session":{"kind":"aggregate""#,
            ),
        );
        let gh = Rc::new(FakeGitHub {
            issues: gh.issues.clone(),
            files,
            ..FakeGitHub::default()
        });
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut ctx(false),
            Halt::Halted(
                "AGENT_LOOP_STOP: contract session.listScenes — a field (/code)".to_string(),
            ),
        )
        .await
        .expect("handed over");
        let writes = gh.writes();
        assert!(
            writes.iter().any(|w| matches!(w, Wrote::CreatedIssue(..))),
            "the manager's: {writes:?}"
        );
        // The same capability once #20 has landed.
        let gh = board("pay", &[labels::MILESTONE, labels::WAITING_MERGE], "open");
        let mut files = gh.files.clone();
        files.insert(
            ("plan/graph.json".to_string(), "main_agent".to_string()),
            GRAPH.replace(
                r#""Session":{"kind":"aggregate""#,
                r#""session.listScenes":{"kind":"capability","producer":20,"state":"built","version":1},"Session":{"kind":"aggregate""#,
            ),
        );
        let gh = Rc::new(FakeGitHub {
            issues: gh.issues.clone(),
            files,
            ..FakeGitHub::default()
        });
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut ctx(false),
            Halt::Halted(
                "AGENT_LOOP_STOP: contract session.listScenes — a field (/code)".to_string(),
            ),
        )
        .await
        .expect("handed over");
        // Landed, it stays the manager's: a consumer editing a landed
        // manifest is refused by `contracts-untouched` (ForgeDesMondes' #482).
        let writes = gh.writes();
        assert!(
            writes.iter().any(|w| matches!(w, Wrote::CreatedIssue(..))),
            "landed: still the manager's: {writes:?}"
        );
        assert!(
            !writes.iter().any(
                |w| matches!(w, Wrote::Body(11, body) if body.contains("decided-by: harness"))
            ),
            "{writes:?}"
        );
    }

    #[tokio::test]
    async fn a_stop_that_asks_a_decision_is_one_whatever_it_names() {
        let gh = board("pay", &[labels::MILESTONE], "open");
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut ctx(false),
            Halt::Halted(
                "AGENT_LOOP_STOP: SPEC contradicts the engine; same in #20; needs a Decision (/code)"
                    .to_string(),
            ),
        )
        .await
        .expect("handed over");
        let writes = gh.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::BlockedByLink(..))),
            "{writes:?}"
        );
        assert!(writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
        assert!(!asks_a_decision(
            "missing crates/session/read — #20 not landed"
        ));
    }

    #[tokio::test]
    async fn a_pull_request_or_a_human_task_named_is_not_waited_for() {
        // ForgeDesMondes' #248 stopped naming PR #264 (a concurrent repair
        // on its branch): a pull request shares the issue numbers, GitHub
        // refuses it as a dependency. A human task mentioned in passing
        // closes long after the task would. Neither is a wait.
        let gh = board("pay", &[labels::MILESTONE], "open");
        let mut issues = gh.issues.clone();
        issues.push(Issue {
            number: 264,
            title: "BattlemapInstance aggregate (#248)".to_string(),
            state: "open".to_string(),
            labels: vec![labels::TO_REVIEW.to_string()],
            ..Issue::default()
        });
        issues.push(Issue {
            number: 30,
            title: "Provide the secrets".to_string(),
            state: "open".to_string(),
            labels: vec![labels::HUMAN.to_string()],
            ..Issue::default()
        });
        let gh = Rc::new(FakeGitHub {
            issues,
            files: gh.files.clone(),
            ..FakeGitHub::default()
        });
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut ctx(false),
            Halt::Halted(
                "AGENT_LOOP_STOP: a concurrent pr_fix session is repairing PR #264; #30 still open"
                    .to_string(),
            ),
        )
        .await
        .expect("handed over");
        let writes = gh.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::BlockedByLink(..))),
            "a PR and a human task are no producers: {writes:?}"
        );
        assert!(writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
    }

    #[tokio::test]
    async fn a_blocker_github_refuses_asks_a_decision_instead_of_failing_the_round() {
        let board = board("pay", &[labels::MILESTONE], "open");
        let gh = Rc::new(FakeGitHub {
            issues: board.issues.clone(),
            files: board.files.clone(),
            refused_blockers: vec![20],
            ..FakeGitHub::default()
        });
        let sink = Rc::new(Capture::default());
        let mut ctx = Context::new(
            Settings {
                dry_run: false,
                stages: String::new(),
            },
            ctx(false).state,
            Logbook::new(
                Rc::clone(&sink) as Rc<dyn harness_core::traces::Sink>,
                harness_core::traces::Verbosity::Normal,
            ),
        );
        let verdict = RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: true,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut ctx,
            Halt::Halted("AGENT_LOOP_STOP: wait for #20 — no crates/session/ (/code)".to_string()),
        )
        .await
        .expect("the round goes on");
        assert_eq!(verdict, Verdict::Continue);
        let writes = gh.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::BlockedByLink(..))),
            "the refused blocker is not written: {writes:?}"
        );
        assert!(
            writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())),
            "the stop asks a decision instead: {writes:?}"
        );
        let said = sink.0.borrow().join("\n");
        assert!(said.contains("cannot wait for #20"), "{said}");
    }

    #[tokio::test]
    async fn a_landed_producer_itself_its_milestone_or_a_roadmap_asks_a_decision_as_before() {
        let stop = || {
            Halt::Halted("AGENT_LOOP_STOP: wait for #20 — no crates/session/ (/code)".to_string())
        };
        for (producer_labels, state) in [
            (vec![labels::MILESTONE, labels::WAITING_MERGE], "open"),
            (vec![labels::MILESTONE], "closed"),
        ] {
            let gh = board("pay", &producer_labels, state);
            RaiseDecision {
                gh: Rc::clone(&gh) as Rc<dyn GitHub>,
                autonomous: false,
                integration_branch: "main_agent".to_string(),
            }
            .on_halt(&mut ctx(false), stop())
            .await
            .expect("handed over");
            let writes = gh.writes();
            assert!(
                !writes.iter().any(|w| matches!(w, Wrote::BlockedByLink(..))),
                "landed: {writes:?}"
            );
            assert!(writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
        }
        // #4 is the task's own milestone, open and building: on the board,
        // so that skipping it is the guard's doing, not a lookup that failed.
        let gh = board("pay", &[labels::MILESTONE], "open");
        let mut issues = gh.issues.clone();
        issues.push(Issue {
            number: 4,
            title: "pay".to_string(),
            state: "open".to_string(),
            labels: vec![labels::MILESTONE.to_string(), labels::TRIGGERED.to_string()],
            ..Issue::default()
        });
        issues.push(Issue {
            number: 5,
            state: "open".to_string(),
            labels: vec![labels::ROADMAP.to_string()],
            ..Issue::default()
        });
        let gh = Rc::new(FakeGitHub {
            issues,
            files: gh.files.clone(),
            ..FakeGitHub::default()
        });
        RaiseDecision {
            gh: Rc::clone(&gh) as Rc<dyn GitHub>,
            autonomous: false,
            integration_branch: "main_agent".to_string(),
        }
        .on_halt(
            &mut ctx(false),
            Halt::Halted("AGENT_LOOP_STOP: #11 needs #4, #5 and #999 first".to_string()),
        )
        .await
        .expect("handed over");
        let writes = gh.writes();
        assert!(
            !writes.iter().any(|w| matches!(w, Wrote::BlockedByLink(..))),
            "itself, its milestone, a roadmap, an unknown issue: {writes:?}"
        );
        assert!(writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
    }

    // --- a stop on a task the plan lacks opens it -------------------------

    fn missing_stop() -> Halt {
        Halt::Halted(missing::BATTLEMAP.to_string())
    }

    fn raise(gh: &Rc<FakeGitHub>, autonomous: bool) -> RaiseDecision {
        RaiseDecision {
            gh: Rc::clone(gh) as Rc<dyn GitHub>,
            autonomous,
            integration_branch: "main_agent".to_string(),
        }
    }

    fn created(writes: &[Wrote]) -> Vec<(String, String, Vec<String>)> {
        writes
            .iter()
            .filter_map(|w| match w {
                Wrote::CreatedIssue(title, body, labels) => {
                    Some((title.clone(), body.clone(), labels.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn decision_of(writes: &[Wrote]) -> Question {
        let body = writes
            .iter()
            .find_map(|w| match w {
                Wrote::Body(11, body) => Some(body.clone()),
                _ => None,
            })
            .expect("the task's body rewritten");
        Question::parse(
            sections::parse(&body)
                .get(sections::DECISION)
                .map_or("", String::as_str),
        )
        .expect("a question")
    }

    #[tokio::test]
    async fn a_missing_task_is_opened_under_the_milestone_and_the_stopper_parked_on_it() {
        // ForgeDesMondes' #405: #191 needed the BattlemapInstance ProjectPg,
        // and a person was asked to open a task the session had described.
        assert_eq!(kind_of(&missing_stop()), Some("stop"));
        for autonomous in [false, true] {
            let gh = gh("## Scope\n\nbranch: feat/table-get-battlemap-instance\n\nread");
            let mut context = ctx(false);
            let verdict = raise(&gh, autonomous)
                .on_halt(&mut context, missing_stop())
                .await
                .expect("handed over");
            assert_eq!(verdict, Verdict::Continue);
            assert!(context.state.handed_over);
            let writes = gh.writes();
            let opened = created(&writes);
            assert_eq!(opened.len(), 1, "{writes:?}");
            let (title, body, labels) = &opened[0];
            assert!(title.starts_with("BattlemapInstance ProjectPg"), "{title}");
            assert_eq!(
                labels,
                &[
                    labels::AGENT.to_string(),
                    labels::WRITE_SIDE.to_string(),
                    labels::DATA_LAYER.to_string(),
                    labels::READY.to_string()
                ]
            );
            let found = sections::parse(body);
            assert!(
                found.get(sections::SCOPE).is_some_and(|scope| scope
                    .contains("branch: feat/table-battlemap-instance-projection")
                    && scope.contains("Opened by the harness for #11")
                    && scope.contains("#11 waits on this task")),
                "{body}"
            );
            let architecture = found.get(sections::ARCHITECTURE).expect("its architecture");
            assert!(
                architecture.starts_with("unit: aggregate\nid: BattlemapInstance\nsystem: table"),
                "{architecture}"
            );
            for line in [
                "aggregate: BattlemapInstance",
                "produces: BattlemapInstance",
                "side: harness:write-side",
            ] {
                assert!(architecture.contains(line), "{line}: {architecture}");
            }
            let task = Issue {
                body: body.clone(),
                labels: labels.clone(),
                ..Issue::default()
            };
            assert!(tasks::spec_written(&task), "TaskHasASpec passes: {body}");
            assert!(
                found
                    .get("acceptance-criteria")
                    .is_some_and(|criteria| criteria.lines().count() == 2)
            );
            assert!(writes.contains(&Wrote::SubIssueLink(4, 1)), "{writes:?}");
            assert!(writes.contains(&Wrote::BlockedByLink(11, 1)), "{writes:?}");
            let kept = decision_of(&writes);
            assert_eq!(kept.kind.as_deref(), Some("wait"));
            assert_eq!(kept.verdict.as_deref(), Some("wait — until #1 lands"));
            assert!(
                !writes.iter().any(|w| matches!(w, Wrote::Label(..))),
                "no decision asked: {writes:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_second_stop_on_the_same_missing_task_reuses_it() {
        let title = "BattlemapInstance ProjectPg: project the aggregate's rows into \
                     battlemap_instance, token, combat and initiative_entry";
        let sibling = |number: u64, labels: &[&str], state: &str| Issue {
            number,
            title: title.to_string(),
            state: state.to_string(),
            labels: labels.iter().map(ToString::to_string).collect(),
            ..Issue::default()
        };
        let gh = Rc::new(FakeGitHub {
            issues: gh("pay").issues.clone(),
            subs: vec![(
                4,
                vec![
                    sibling(405, &[labels::HUMAN], "open"),
                    sibling(300, &[labels::AGENT], "closed"),
                    sibling(30, &[labels::AGENT, labels::READY], "open"),
                ],
            )],
            ..FakeGitHub::default()
        });
        raise(&gh, false)
            .on_halt(&mut ctx(false), missing_stop())
            .await
            .expect("handed over");
        let writes = gh.writes();
        assert!(created(&writes).is_empty(), "no second task: {writes:?}");
        assert!(!writes.iter().any(|w| matches!(w, Wrote::SubIssueLink(..))));
        assert_eq!(writes[0], Wrote::BlockedByLink(11, 30));
        assert_eq!(
            decision_of(&writes).verdict.as_deref(),
            Some("wait — until #30 lands")
        );
    }

    #[tokio::test]
    async fn a_slice_declared_wrong_is_refused_to_the_session_then_goes_to_a_person() {
        let vague = || {
            Halt::Halted(
                "AGENT_LOOP_STOP: task the BattlemapInstance projection (/code)".to_string(),
            )
        };
        // Round 1, gate or none: refused back to the session, loudly.
        for autonomous in [false, true] {
            let first = gh("## Scope\n\nbranch: feat/pay\n\npay");
            let mut context = ctx(false);
            raise(&first, autonomous)
                .on_halt(&mut context, vague())
                .await
                .expect("handed over");
            assert!(context.state.handed_over);
            let writes = first.writes();
            assert!(created(&writes).is_empty(), "nobody asked: {writes:?}");
            assert!(
                !writes
                    .iter()
                    .any(|w| matches!(w, Wrote::Label(..) | Wrote::BlockedByLink(..)))
            );
            assert!(
                writes.iter().any(|w| matches!(w, Wrote::Comment(11, said)
                    if said.contains("- \"the BattlemapInstance projection (/code)\": the stop \
                                      carries no slice that reads")
                    && said.contains("reads this under `## Decision`"))),
                "{writes:?}"
            );
            let refused = decision_of(&writes);
            assert_eq!(refused.kind.as_deref(), Some("refused"));
            assert_eq!(refused.round, 1);
            assert_eq!(refused.decided_by.as_deref(), Some("harness"));
            assert_eq!(
                refused.verdict.as_deref(),
                Some("refused — re-declare 1 declared task, as said below")
            );
            assert_eq!(
                refused.declared,
                [missing::key_of("the BattlemapInstance projection (/code)")]
            );
        }

        // Refused twice already: the third is a person's.
        let twice = "## Scope\n\nbranch: feat/pay\n\npay\n\n## Decision\n\nkind: refused\nblocks: \
                     x\nround: 2\ndecided-by: harness\nverdict: refused";
        let third = gh(twice);
        raise(&third, false)
            .on_halt(&mut ctx(false), vague())
            .await
            .expect("handed over");
        let writes = third.writes();
        let opened = created(&writes);
        assert_eq!(opened.len(), 1, "{writes:?}");
        let (title, body, labels) = &opened[0];
        assert_eq!(
            title,
            "Missing task for #11: the BattlemapInstance projection (/code)"
        );
        assert_eq!(labels, &[labels::HUMAN.to_string()]);
        assert!(body.contains("## Missing task"), "{body}");
        assert!(
            body.contains("could not open it itself:\n\n- the stop carries no slice that reads"),
            "says why: {body}"
        );
        assert!(body.contains("declared wrong 3 times"), "{body}");
        assert!(
            body.contains("> the BattlemapInstance projection"),
            "{body}"
        );
        assert!(writes.contains(&Wrote::SubIssueLink(4, 1)));
        assert!(writes.contains(&Wrote::BlockedByLink(11, 1)));
        assert!(!writes.iter().any(|w| matches!(w, Wrote::Body(..))));

        // Without a human gate, nobody would come: a decision is asked, with
        // the milestone, the declaration and the stop whole.
        let free = gh(twice);
        raise(&free, true)
            .on_halt(&mut ctx(false), vague())
            .await
            .expect("handed over");
        let writes = free.writes();
        assert!(created(&writes).is_empty(), "{writes:?}");
        assert!(writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
        let asked = decision_of(&writes);
        assert_eq!(asked.kind.as_deref(), Some("stop"));
        assert_eq!(asked.round, 3);
        assert_eq!(asked.milestone, Some(4));
        assert_eq!(
            asked.declared,
            [missing::key_of("the BattlemapInstance projection (/code)")]
        );
        assert!(
            writes.iter().any(|w| matches!(w, Wrote::Body(11, body)
                if body.contains("The stop, whole:\n\n> AGENT_LOOP_STOP: task the \
                                   BattlemapInstance projection (/code)"))),
            "{writes:?}"
        );
    }

    #[tokio::test]
    async fn a_whole_slice_with_no_milestone_to_belong_to_goes_to_a_person() {
        let lone = gh("pay");
        let mut orphan = ctx(false);
        orphan.state.milestone.number = String::new();
        raise(&lone, false)
            .on_halt(&mut orphan, missing_stop())
            .await
            .expect("handed over");
        let writes = lone.writes();
        let opened = created(&writes);
        assert_eq!(opened.len(), 1, "{writes:?}");
        assert_eq!(opened[0].2, [labels::HUMAN.to_string()]);
        assert!(
            opened[0].1.contains("milestone is not known"),
            "{}",
            opened[0].1
        );
        assert!(!writes.iter().any(|w| matches!(w, Wrote::SubIssueLink(..))));
        assert!(writes.contains(&Wrote::BlockedByLink(11, 1)));
    }

    #[tokio::test]
    async fn a_missing_read_side_task_is_labelled_read_side() {
        let stop = missing::BATTLEMAP
            .replace(r#""unit": "aggregate""#, r#""unit": "capability""#)
            .replace(
                r#""produces": ["BattlemapInstance"]"#,
                r#""produces": ["table.listTokens"]"#,
            );
        let gh = gh("pay");
        raise(&gh, false)
            .on_halt(&mut ctx(false), Halt::Halted(stop))
            .await
            .expect("handed over");
        let writes = gh.writes();
        let opened = created(&writes);
        assert_eq!(
            opened[0].2,
            [
                labels::AGENT.to_string(),
                labels::READ_SIDE.to_string(),
                labels::READY.to_string()
            ]
        );
        assert!(
            opened[0].1.contains("id: table.listTokens"),
            "{}",
            opened[0].1
        );
    }

    fn many(titles: &[&str]) -> Halt {
        let slices: Vec<String> = titles
            .iter()
            .enumerate()
            .map(|(k, title)| {
                if title.is_empty() {
                    r#"{"title": "half a slice"}"#.to_string()
                } else {
                    format!(
                        r#"{{"title": "{title}", "branch": "feat/unit-{k}", "brief": "b", "unit": "data-capability", "system": "table", "aggregate": "BattlemapInstance", "produces": ["table.unit{k}"], "goal": "g", "acceptance": ["given a, when b, then c"]}}"#
                    )
                }
            })
            .collect();
        Halt::Halted(format!(
            "AGENT_LOOP_STOP: tasks [{}] (/code)",
            slices.join(", ")
        ))
    }

    #[tokio::test]
    async fn a_stop_naming_three_missing_tasks_opens_them_and_waits_on_all() {
        let gh = gh("## Scope\n\nbranch: feat/pay\n\npay");
        let mut context = ctx(false);
        raise(&gh, true)
            .on_halt(&mut context, many(&["Unit A", "Unit B", "Unit C"]))
            .await
            .expect("handed over");
        let writes = gh.writes();
        assert_eq!(created(&writes).len(), 3, "{writes:?}");
        for blocker in 1..=3 {
            assert!(
                writes.contains(&Wrote::BlockedByLink(11, blocker)),
                "{writes:?}"
            );
        }
        let bodies: Vec<&String> = writes
            .iter()
            .filter_map(|w| match w {
                Wrote::Body(11, body) => Some(body),
                _ => None,
            })
            .collect();
        assert_eq!(bodies.len(), 1, "one body write: {writes:?}");
        assert!(
            bodies[0].contains("wait — until #1, #2, #3 land"),
            "{}",
            bodies[0]
        );
        assert!(!writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
    }

    #[tokio::test]
    async fn a_mixed_stop_opens_the_writable_and_refuses_the_rest_out_loud() {
        for autonomous in [false, true] {
            let gh = gh("## Scope\n\nbranch: feat/pay\n\npay");
            raise(&gh, autonomous)
                .on_halt(&mut ctx(false), many(&["Unit A", ""]))
                .await
                .expect("handed over");
            let writes = gh.writes();
            let opened = created(&writes);
            assert_eq!(
                opened.len(),
                1,
                "no person for a slice to re-declare: {writes:?}"
            );
            assert_eq!(opened[0].0, "Unit A");
            assert!(writes.contains(&Wrote::BlockedByLink(11, 1)));
            assert!(!writes.contains(&Wrote::Label(11, labels::NEEDS_DECISION.to_string())));
            let wait = decision_of(&writes);
            assert_eq!(wait.kind.as_deref(), Some("wait"));
            assert!(
                writes.iter().any(|w| matches!(w, Wrote::Body(11, body)
                    if body.contains("**Declared tasks the harness did not open**")
                    && body.contains("- \"half a slice\": a slice that does not read"))),
                "{writes:?}"
            );
            assert!(
                writes.iter().any(
                    |w| matches!(w, Wrote::Comment(11, said) if said.contains("half a slice"))
                ),
                "{writes:?}"
            );
        }
    }

    // --- #219, 2026-10-25: a slice dropped, a decision that opened nothing --

    const STOP_219_ROUND_1: &str = include_str!("../../../tests/fixtures/stop-219-round-1.txt");
    const STOP_219_ROUND_2: &str = include_str!("../../../tests/fixtures/stop-219-round-2.txt");

    #[tokio::test]
    async fn the_two_stops_of_219_open_both_slices_and_park_on_both() {
        for (stop, fixtures) in [
            (STOP_219_ROUND_1, "plumbing"),
            (STOP_219_ROUND_2, "server-fixtures"),
        ] {
            for autonomous in [false, true] {
                let gh = gh("## Scope\n\nbranch: feat/preparation-seance\n\nprepare");
                raise(&gh, autonomous)
                    .on_halt(&mut ctx(false), Halt::Halted(stop.trim().to_string()))
                    .await
                    .expect("handed over");
                let writes = gh.writes();
                let opened = created(&writes);
                assert_eq!(opened.len(), 2, "{fixtures}: {writes:?}");
                assert!(writes.contains(&Wrote::BlockedByLink(11, 1)), "{writes:?}");
                assert!(writes.contains(&Wrote::BlockedByLink(11, 2)), "{writes:?}");
                assert!(writes.contains(&Wrote::SubIssueLink(4, 1)));
                assert!(writes.contains(&Wrote::SubIssueLink(4, 2)));
                assert!(
                    !writes
                        .iter()
                        .any(|w| matches!(w, Wrote::Label(..) | Wrote::Comment(..)))
                );
                assert_eq!(
                    decision_of(&writes).verdict.as_deref(),
                    Some("wait — until #1, #2 land")
                );
                let (title, body, labels) = &opened[0];
                assert!(title.starts_with("Server fixtures: load"), "{title}");
                assert!(
                    labels.contains(&labels::WRITE_SIDE.to_string()),
                    "{labels:?}"
                );
                let found = sections::parse(body);
                assert!(
                    found
                        .get(sections::ARCHITECTURE)
                        .is_some_and(|a| a.starts_with("unit: infrastructure")),
                    "{body}"
                );
                assert!(
                    body.contains(&format!("The stop declared its unit as `{fixtures}`")),
                    "{body}"
                );
                assert!(body.contains("so this task is on the write side"), "{body}");
            }
        }
    }

    #[tokio::test]
    async fn a_unit_no_key_names_holds_the_task_with_the_slice_the_unit_and_the_keys() {
        let stop = STOP_219_ROUND_2.replace(r#""unit": "server-fixtures""#, r#""unit": "widget""#);
        for autonomous in [false, true] {
            let gh = gh("## Scope\n\nbranch: feat/preparation-seance\n\nprepare");
            raise(&gh, autonomous)
                .on_halt(&mut ctx(false), Halt::Halted(stop.trim().to_string()))
                .await
                .expect("handed over");
            let writes = gh.writes();
            assert!(created(&writes).is_empty(), "{writes:?}");
            assert!(
                !writes
                    .iter()
                    .any(|w| matches!(w, Wrote::Label(..) | Wrote::BlockedByLink(..))),
                "no decision paid, no wait: {writes:?}"
            );
            let comment = writes
                .iter()
                .find_map(|w| match w {
                    Wrote::Comment(11, said) => Some(said.clone()),
                    _ => None,
                })
                .expect("a comment on the task");
            for said in [
                "- \"Server fixtures: load the Session aggregate's scenes",
                "- \"Server fixtures: load the BattlemapInstance rows",
                r#"its `unit` ("widget") is no unit of the architecture — one of: "#,
                &missing::unit_keys(),
            ] {
                assert!(comment.contains(said), "{said}: {comment}");
            }
            let refused = decision_of(&writes);
            assert_eq!(refused.kind.as_deref(), Some("refused"));
            assert_eq!(
                refused.verdict.as_deref(),
                Some("refused — re-declare 2 declared tasks, as said below")
            );
            assert!(
                writes.iter().any(|w| matches!(w, Wrote::Body(11, body)
                    if body.contains("its `unit` (\"widget\") is no unit"))),
                "the next session reads it: {writes:?}"
            );
        }
        // The next stop counts the refusal as a round.
        let again = gh("## Decision\n\nkind: refused\nblocks: x\nround: 1\ndecided-by: harness");
        raise(&again, true)
            .on_halt(&mut ctx(false), Halt::Halted(stop.trim().to_string()))
            .await
            .expect("handed over");
        assert_eq!(decision_of(&again.writes()).round, 2);
    }

    #[tokio::test]
    async fn a_person_s_slice_beside_opened_ones_is_said_not_dropped() {
        let stop = STOP_219_ROUND_1.replacen(
            r#""unit": "micro-ui""#,
            r#""needs_human": true, "unit": "micro-ui""#,
            1,
        );
        let gh = gh("## Scope\n\nbranch: feat/preparation-seance\n\nprepare");
        raise(&gh, true)
            .on_halt(&mut ctx(false), Halt::Halted(stop.trim().to_string()))
            .await
            .expect("handed over");
        let writes = gh.writes();
        assert_eq!(created(&writes).len(), 1, "{writes:?}");
        assert!(writes.contains(&Wrote::BlockedByLink(11, 1)));
        assert!(
            writes.iter().any(|w| matches!(w, Wrote::Comment(11, said)
                if said.contains("Micro-UI PreviousSummary")
                && said.contains("only a person can do it — asked again once #1 land"))),
            "{writes:?}"
        );
    }

    /// The stop's writes, without a human gate, on a task whose body is
    /// `body`.
    async fn stopped_free(body: &str, stop: &str) -> Vec<Wrote> {
        let free = gh(body);
        raise(&free, true)
            .on_halt(&mut ctx(false), Halt::Halted(stop.to_string()))
            .await
            .expect("handed over");
        free.writes()
    }

    /// The task's body once `writes` are done: the last one written.
    fn body_after(body: &str, writes: &[Wrote]) -> String {
        writes
            .iter()
            .rev()
            .find_map(|w| match w {
                Wrote::Body(11, body) => Some(body.clone()),
                _ => None,
            })
            .unwrap_or_else(|| body.to_string())
    }

    fn decisions_asked(writes: &[Wrote]) -> usize {
        writes
            .iter()
            .filter(|w| **w == Wrote::Label(11, labels::NEEDS_DECISION.to_string()))
            .count()
    }

    const VAGUE: &str = "AGENT_LOOP_STOP: task the BattlemapInstance projection (/code)";

    #[tokio::test]
    async fn the_same_declaration_after_a_decision_never_asks_a_second_one() {
        let ruled = format!(
            "## Scope\n\nbranch: feat/pay\n\npay\n\n## Decision\n\nkind: stop\nblocks: x\nround: \
             3\ndecided-by: ai\nverdict: answer — build it in this task (4/5)\nmilestone: \
             #4\ndeclared: {}",
            missing::key_of("the BattlemapInstance projection (/code)")
        );
        let writes = stopped_free(&ruled, VAGUE).await;
        assert_eq!(
            decisions_asked(&writes),
            0,
            "no second decision: {writes:?}"
        );
        assert!(created(&writes).is_empty(), "{writes:?}");
        let refused = decision_of(&writes);
        assert_eq!(refused.kind.as_deref(), Some("refused"));
        assert_eq!(
            refused.ruled.as_deref(),
            Some("answer — build it in this task (4/5)"),
            "the ruling carried on"
        );
        assert_eq!(refused.ruled_at, Some(3));
        assert!(
            writes.iter().any(|w| matches!(w, Wrote::Comment(11, said)
                if said.contains("the harness decided on it already: answer — build it in this task"))),
            "{writes:?}"
        );
        // A new declaration is a new fact: decided again.
        let other = ruled.replace("declared: the-battlemapinstance", "declared: another");
        assert_eq!(decisions_asked(&stopped_free(&other, VAGUE).await), 1);
        // A person's choice, decided by its owner, is no ruling by `decide`.
        let answered = ruled.replace("decided-by: ai", "decided-by: owner");
        assert_eq!(decisions_asked(&stopped_free(&answered, VAGUE).await), 1);
        // A refusal that carries no ruling is none either.
        let bare = ruled.replace("decided-by: ai", "decided-by: harness");
        assert_eq!(decisions_asked(&stopped_free(&bare, VAGUE).await), 1);
    }

    #[tokio::test]
    async fn identical_stops_without_a_gate_pay_one_decision_then_go_to_a_person() {
        let body = "## Scope\n\nbranch: feat/pay\n\npay\n\n## Decision\n\nkind: refused\nblocks: \
                    x\nround: 2\ndecided-by: harness\nverdict: refused";
        // Round 3: a decision asked.
        let first = stopped_free(body, VAGUE).await;
        let body = body_after(body, &first);
        // `decide` answers it.
        let asked = Question::parse(
            sections::parse(&body)
                .get(sections::DECISION)
                .map_or("", String::as_str),
        )
        .expect("a question");
        let decided = crate::decide::data::decision::Decided {
            action: crate::decide::data::decision::Action::Answer,
            summary: "build it in this task".to_string(),
            answer: String::new(),
            confidence: 4,
            tasks: Vec::new(),
        };
        let section = crate::decide::data::decision::section(Some(&asked), "stop", &decided);
        let body = sections::replace(&body, &[(sections::DECISION, &section)]);
        // Rounds 4 and 5: the same stop, the ruling sent back each time.
        let second = stopped_free(&body, VAGUE).await;
        let body = body_after(&body, &second);
        let third = stopped_free(&body, VAGUE).await;
        let body = body_after(&body, &third);
        assert_eq!(
            [&first, &second, &third]
                .iter()
                .map(|writes| decisions_asked(writes))
                .sum::<usize>(),
            1,
            "{second:?}\n{third:?}"
        );
        let carried = decision_of(&third);
        assert_eq!((carried.round, carried.ruled_at), (5, Some(3)));
        assert!(created(&second).is_empty() && created(&third).is_empty());
        // Round 6: the session ignored the ruling twice — a person's.
        let fourth = stopped_free(&body, VAGUE).await;
        assert_eq!(decisions_asked(&fourth), 0);
        let opened = created(&fourth);
        assert_eq!(opened.len(), 1, "{fourth:?}");
        assert_eq!(opened[0].2, [labels::HUMAN.to_string()]);
        assert!(
            opened[0].1.contains(
                "declared again 3 times after the harness decided: answer — build it in this \
                 task (4/5)"
            ),
            "{}",
            opened[0].1
        );
        assert!(fourth.contains(&Wrote::BlockedByLink(11, 1)));
    }

    #[tokio::test]
    async fn a_slice_a_closed_task_built_is_refused_not_opened_again() {
        // One branch is one feature: #300 landed it, the stop declares it again.
        let landed = Issue {
            number: 300,
            title: "Battlemap rows projected".to_string(),
            state: "closed".to_string(),
            labels: vec![labels::AGENT.to_string()],
            body: "## Scope\n\nbranch: feat/table-battlemap-instance-projection\n".to_string(),
            ..Issue::default()
        };
        let board = |body: &str| {
            Rc::new(FakeGitHub {
                issues: gh(body).issues.clone(),
                subs: vec![(4, vec![landed.clone()])],
                ..FakeGitHub::default()
            })
        };
        let refusal = "#300 built `feat/table-battlemap-instance-projection` already (closed): \
                       build on it, or declare a new branch";
        for autonomous in [false, true] {
            let gh = board("## Scope\n\nbranch: feat/pay\n\npay");
            raise(&gh, autonomous)
                .on_halt(&mut ctx(false), missing_stop())
                .await
                .expect("handed over");
            let writes = gh.writes();
            assert!(created(&writes).is_empty(), "{writes:?}");
            assert!(
                !writes
                    .iter()
                    .any(|w| matches!(w, Wrote::Label(..) | Wrote::BlockedByLink(..))),
                "{writes:?}"
            );
            assert!(
                writes
                    .iter()
                    .any(|w| matches!(w, Wrote::Comment(11, said) if said.contains(refusal))),
                "{writes:?}"
            );
            let refused = decision_of(&writes);
            assert_eq!(refused.kind.as_deref(), Some("refused"));
            assert_eq!(refused.round, 1);
            assert_eq!(
                refused.declared,
                ["feat/table-battlemap-instance-projection"]
            );
        }
        // Declared again a third time: a person's, or one decision without
        // a gate — never a second issue on the branch.
        let twice = "## Scope\n\nbranch: feat/pay\n\npay\n\n## Decision\n\nkind: refused\nblocks: \
                     x\nround: 2\ndecided-by: harness\nverdict: refused";
        let gated = board(twice);
        raise(&gated, false)
            .on_halt(&mut ctx(false), missing_stop())
            .await
            .expect("handed over");
        let opened = created(&gated.writes());
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].2, [labels::HUMAN.to_string()]);
        assert!(
            opened[0].1.contains(refusal) && opened[0].1.contains("declared 3 times"),
            "{}",
            opened[0].1
        );
        let free = board(twice);
        raise(&free, true)
            .on_halt(&mut ctx(false), missing_stop())
            .await
            .expect("handed over");
        let writes = free.writes();
        assert!(created(&writes).is_empty(), "{writes:?}");
        assert_eq!(decisions_asked(&writes), 1);
    }

    #[test]
    fn a_ruling_is_the_ai_s_answer_or_a_refusal_carrying_it() {
        let declared = vec!["feat/a".to_string()];
        let question = |decided_by: &str, ruled: Option<&str>, ruled_at: Option<u32>| Question {
            kind: Some("refused".to_string()),
            blocks: "x".to_string(),
            round: 5,
            decided_by: Some(decided_by.to_string()),
            verdict: Some("v".to_string()),
            declared: declared.clone(),
            ruled: ruled.map(ToString::to_string),
            ruled_at,
            ..Question::default()
        };
        let ruling = |verdict: &str, at: u32| {
            Some(Ruling {
                verdict: verdict.to_string(),
                at,
            })
        };
        assert_eq!(
            ruled_on(Some(&question("ai", None, None)), &declared),
            ruling("v", 5)
        );
        assert_eq!(
            ruled_on(Some(&question("harness", Some("r"), Some(3))), &declared),
            ruling("r", 3)
        );
        assert_eq!(
            ruled_on(Some(&question("harness", Some("r"), None)), &declared),
            ruling("r", 5),
            "no round kept: this one"
        );
        assert_eq!(
            ruled_on(Some(&question("harness", None, Some(3))), &declared),
            None
        );
        assert_eq!(
            ruled_on(Some(&question("ai", None, None)), &["feat/b".to_string()]),
            None
        );
        assert_eq!(ruled_on(Some(&question("ai", None, None)), &[]), None);
        let mut nothing_declared = question("ai", None, None);
        nothing_declared.declared.clear();
        assert_eq!(ruled_on(Some(&nothing_declared), &[]), None);
        assert_eq!(ruled_on(None, &declared), None);
    }
}
