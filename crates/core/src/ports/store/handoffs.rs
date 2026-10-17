//! The relay between two tasks of one system: a `code` session that ended
//! light leaves its conversation for the next task to branch from.
//!
//! Why it exists: every task opens a fresh session that reads its spec, the
//! repository and the plumbing again; the next task of the same system, on
//! the same milestone, needs most of what the last one just read. A session
//! that ended with a small context is worth continuing; a heavy one is not —
//! the decision is the workflow's, the register only keeps the relay.

use crate::domain::Outcome;

/// What a finished session leaves for the next task.
#[derive(Debug, Clone, PartialEq)]
pub struct Handoff {
    /// The carrier's id of the conversation to branch from.
    pub session: String,
    /// What the conversation cost, as its carrier reported it: a branch that
    /// reports the whole conversation's cost takes it out.
    pub cost_usd: Option<f64>,
    /// The task it delivered, which never takes its own relay back.
    pub task: String,
    /// The context it ended with, in tokens.
    pub context_tokens: u64,
}

/// The register of relays, one per key (a milestone and a system).
pub trait Handoffs {
    /// Leaves `handoff` under `key`, in place of an older one.
    ///
    /// # Errors
    /// The register cannot be written.
    fn leave(&self, key: &str, handoff: &Handoff) -> Outcome<()>;
    /// Takes the relay under `key`, once: a second taker finds none. `None`
    /// as well when the relay is too old for its cache to still be warm.
    ///
    /// # Errors
    /// The register cannot be read.
    fn take(&self, key: &str) -> Outcome<Option<Handoff>>;
}
