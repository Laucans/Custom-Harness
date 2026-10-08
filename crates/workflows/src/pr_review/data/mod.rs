//! What the review reads and decides: its state, the skip rules, the text it
//! publishes.
//!
//! Pure domain — nothing here calls `gh` or writes anywhere. What writes
//! lives in `action`, what judges in `checks`.

pub mod findings;
pub mod notes;
pub mod skip_rules;
pub mod state;
