//! What opening an agent session requires — the port, not the carrier.
//!
//! Mirror of `adapters/agent/base.py` from Python (`AgentRunner`, an ABC, one
//! implementation that imports the SDK): an interface decided now, a concrete
//! carrier decided behind it later. Nothing here chooses a carrier — tmux, a
//! `claude -p --input-format stream-json` process, or something else: that is
//! an implementation decision, and it lives in
//! [`adapters::agent`](crate::adapters::agent).

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
}

/// What is needed to open a session: the model, the effort.
///
/// Pure data — a tmux or stream-json carrier does what it wants with it, but
/// neither changes what a caller has the right to ask for.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// The requested model (`"opus"`, `"sonnet"`, …).
    pub model: String,
    /// The effort level (`"low"` … `"max"`).
    pub effort: String,
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
}
