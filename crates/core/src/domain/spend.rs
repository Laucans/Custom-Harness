//! What a turn cost. Pure data: no one here knows how to write it.
//!
//! Every field is an `Option`, and for the same reason as
//! `ports::agent::Reply::cost`: a session bearer doesn't necessarily
//! return everything. A terminal pane returns neither tokens nor cost; a JSON
//! stream returns both. `None` reads as « not observed », never « zero » — the
//! difference is what prevents a register from counting a free session where
//! it measured nothing.

/// The tokens of a turn, by nature.
///
/// Separate from cost: cost is an estimate derived from a price table, tokens
/// are what the API actually reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Input tokens, cache excluded.
    pub input: Option<u64>,
    /// Output tokens.
    pub output: Option<u64>,
    /// Tokens read from cache, billed at reduced rate.
    pub cache_read: Option<u64>,
    /// Tokens written to cache, billed at increased rate.
    pub cache_write: Option<u64>,
}

/// What the billed task is, for the ledger: its unit of the architecture,
/// the profile it was cut for, its side, and the milestone it belongs to.
///
/// Plain strings, empty when unknown: `core` names no unit of the
/// target's architecture, it only carries what the workflow read of it to
/// the row. A run that works on no task (a review, a split) leaves them
/// empty, and empty reads back as "not declared", never as a unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tags {
    /// The unit the task builds (`capability`, `aggregate`, …).
    pub unit: String,
    /// The profile the slice was cut for (`strong`, `mid`, `small`).
    pub profile: String,
    /// The side of the architecture (`harness:read-side`, `harness:write-side`).
    pub side: String,
    /// `#<n> <title>` of the task's milestone ([`milestone_label`]), empty
    /// if unknown. Written where the money is spent, so a milestone closed
    /// long ago still owns what it cost.
    pub milestone: String,
}

/// How a milestone is named in the ledger and in the view: `#<n> <title>`.
///
/// One spelling for the writer and every reader, so a row written by a run
/// and a row rebuilt from the board land in the same bucket. `number` is the
/// issue number, as a `u64` or as the text a `Named` keeps it in.
#[must_use]
pub fn milestone_label(number: impl std::fmt::Display, title: &str) -> String {
    format!("#{number} {title}")
}

/// The issue number a ledger `task` key names: `61` and `#61` are the same
/// issue.
///
/// The dev loop bills a task under its bare number, every other workflow
/// under its reference (`#61`); a reader that looks the key up on the board
/// strips the `#` here rather than miss half the rows.
#[must_use]
pub fn task_number(key: &str) -> &str {
    key.strip_prefix('#').unwrap_or(key)
}

/// What a turn cost, and what it consumed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spend {
    /// Cost in dollars.
    ///
    /// **Client-side estimate**, calculated from a price table embedded in
    /// Claude Code — not billing data. Good for a budget, never for billing.
    ///
    /// Beware the cumulation semantics: on a resumed session, Claude Code
    /// returns the total of **the entire** conversation since v2.1.277, and
    /// only that of the previous call. See `harness_adapters::agent::claude_cli`.
    pub cost_usd: Option<f64>,
    /// How many round-trips with the model this turn requested.
    pub turns: Option<u32>,
    /// Duration of the turn, end to end.
    pub duration_ms: Option<u64>,
    /// The tokens consumed.
    pub tokens: Tokens,
    /// The session identifier that the bearer reported.
    pub session: Option<String>,
    /// What the carrier observed of its rate-limit windows on this turn.
    ///
    /// **On the contract and not inside one adapter**, so swapping the carrier
    /// swaps only the *translation*. A carrier that reports no window leaves this
    /// `None` and the preflight that reads it starts anyway — but the field is
    /// here, in the type every carrier fills, rather than a file one adapter
    /// happened to write. The earlier shape had `claude_cli` call `std::fs::write`
    /// directly: a second client would have silently provided nothing, and the
    /// gate would have become dead code with no compile error to say so.
    pub quota: Option<crate::domain::quota::Reading>,
}

impl Spend {
    /// True if nothing was observed — no field filled.
    ///
    /// What a register reads to write « not measured » rather than a column of
    /// zeros, which would read back as a free session.
    #[must_use]
    pub const fn is_blind(&self) -> bool {
        self.cost_usd.is_none()
            && self.turns.is_none()
            && self.duration_ms.is_none()
            && self.tokens.input.is_none()
            && self.tokens.output.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_spend_is_blind() {
        assert!(Spend::default().is_blind());
    }

    #[test]
    fn one_observed_field_is_enough_to_stop_being_blind() {
        let spend = Spend {
            cost_usd: Some(0.0),
            ..Spend::default()
        };
        // A cost of zero **observed** is not the same as nothing observed: the
        // first is a measurement, the second is ignorance.
        assert!(!spend.is_blind());
    }

    #[test]
    fn a_milestone_is_named_by_its_number_then_its_title() {
        assert_eq!(milestone_label(12, "Billing"), "#12 Billing");
        assert_eq!(milestone_label("12", "Billing"), "#12 Billing");
    }

    #[test]
    fn a_task_key_names_the_same_issue_with_or_without_its_hash() {
        assert_eq!(task_number("#61"), "61");
        assert_eq!(task_number("61"), "61");
        assert_eq!(task_number(""), "");
        assert_eq!(task_number("main_agent@e0b409e"), "main_agent@e0b409e");
    }

    #[test]
    fn tokens_default_to_unobserved_not_to_zero() {
        let tokens = Tokens::default();
        assert_eq!(tokens.input, None);
        assert_eq!(tokens.cache_read, None);
    }
}
