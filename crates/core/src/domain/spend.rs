//! What a turn cost. Pure data: no one here knows how to write it.
//!
//! Every field is an `Option`, and for the same reason as
//! `adapters::agent::Reply::cost`: a session bearer doesn't necessarily
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
    /// only that of the previous call. See `adapters::agent::claude_cli`.
    pub cost_usd: Option<f64>,
    /// How many round-trips with the model this turn requested.
    pub turns: Option<u32>,
    /// Duration of the turn, end to end.
    pub duration_ms: Option<u64>,
    /// The tokens consumed.
    pub tokens: Tokens,
    /// The session identifier that the bearer reported.
    pub session: Option<String>,
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
    fn tokens_default_to_unobserved_not_to_zero() {
        let tokens = Tokens::default();
        assert_eq!(tokens.input, None);
        assert_eq!(tokens.cache_read, None);
    }
}
