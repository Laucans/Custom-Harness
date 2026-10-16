//! What opening an agent session requires — the port, not the carrier.
//!
//! Mirror of `adapters/agent/base.py` from Python (`AgentRunner`, an ABC, one
//! implementation that imports the SDK): an interface decided now, a concrete
//! carrier decided behind it later. Nothing here chooses a carrier — tmux, a
//! `claude -p --input-format stream-json` process, or something else: that is
//! an implementation decision, and it lives in
//! `harness_adapters::agent`.

use async_trait::async_trait;

use crate::domain::{Outcome, Spend};

/// What a session returns after a turn.
///
/// Honest about what a carrier can actually observe: everything in [`Spend`]
/// is optional, because a terminal pane does not report usage, unlike a
/// structured JSON stream. The trait does not presuppose the generosity of
/// the most capable carrier.
#[derive(Debug, Clone)]
pub struct Reply {
    /// The text the session returned for this turn.
    pub text: String,
    /// The `AGENT_LOOP_STOP` marker, if the session stopped itself.
    pub stop_line: Option<String>,
    /// What this turn cost and consumed.
    ///
    /// The fields the spending ledger demands — tokens, turns, duration, session
    /// identifier — live here rather than as columns of `Reply`: the ledger
    /// revealed which ones, and grouping them keeps `Reply` readable when a
    /// carrier fills in none.
    pub spend: Spend,
}

/// An open session against an agent: multiple turns, one process.
///
/// Never leaves its `Stage` — nothing outside a stage holds a `Session`. This
/// is not a convention: no public type exposes a way to obtain one other than
/// inside a `Stage`'s `perform`.
#[async_trait(?Send)]
pub trait Session {
    /// Sends a message to the open session, waits for the full turn.
    async fn ask(&mut self, prompt: &str) -> Outcome<Reply>;

    /// The conversation's identifier, once the carrier has one — what a
    /// later run passes back to resume it after an interruption. `None` by
    /// default: a carrier with no conversations to resume has no id to give.
    fn id(&self) -> Option<String> {
        None
    }
}

/// How a session is opened beyond its spec: resumed, and bounded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Opening {
    /// The conversation to resume, when a previous run was interrupted in
    /// the same stage of the same task: the turns already paid stay paid.
    pub resume: Option<String>,
    /// How many model turns the session may take, at most — the safety net
    /// against a stage that loops. `None`: the carrier's own limit.
    pub max_turns: Option<u32>,
}

/// What is needed to open a session: the model, the effort.
///
/// Pure data — a tmux or stream-json carrier does what it wants with it, but
/// neither changes what a caller has the right to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    /// The requested model (`"opus"`, `"sonnet"`, …).
    pub model: String,
    /// The effort level (`"low"` … `"max"`).
    pub effort: String,
}

/// A finished conversation that later sessions branch from.
///
/// Why it exists: the prompt cache. A fresh session re-writes every byte of
/// its first message, while a branch re-reads the conversation it starts
/// from — so stages that share a long context (the repo map, the hierarchy)
/// hear it once, in the base, and each branch adds only its own question.
#[derive(Debug, Clone, PartialEq)]
pub struct Base {
    /// The identifier the carrier reported for the base conversation.
    pub session: String,
    /// What the base cost, as its carrier reported it.
    ///
    /// A carrier that reports a branch's cost as the whole conversation's
    /// subtracts this, so the ledger never counts the base twice.
    pub cost_usd: Option<f64>,
}

/// Builds a [`Session`].
///
/// The only thing `harness-core` knows about a concrete carrier: that one
/// exists, injected by the stage that needs it. Mirror of `hub.py`: "only
/// place that builds a client — one test seam for all workflows."
#[async_trait(?Send)]
pub trait SessionFactory {
    /// Opens a new session for this `spec`.
    async fn open(&self, spec: &SessionSpec) -> Outcome<Box<dyn Session>>;

    /// Opens a session that continues a copy of `base`, leaving `base` as is.
    ///
    /// The session already holds everything `base` was told: a caller sends
    /// it only what is new.
    async fn fork(&self, spec: &SessionSpec, base: &Base) -> Outcome<Box<dyn Session>>;

    /// Opens a session for this `spec`, resumed and bounded as `opening`
    /// says. By default the same as [`Self::open`]: a carrier that cannot
    /// resume starts afresh and ignores the cap — honest for a fake.
    async fn open_with(&self, spec: &SessionSpec, opening: &Opening) -> Outcome<Box<dyn Session>> {
        let _ = opening;
        self.open(spec).await
    }
}
