//! A command as the queue holds it (contract G) and answers it (contract H).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a `DataCapability` does to the data (contract F).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// A new row, a new identifier.
    Insert,
    /// Changes an existing row.
    Update,
    /// Removes a row — a soft delete, by default.
    Delete,
    /// Inserts or updates: touches existing data by construction.
    Upsert,
}

/// How a command meets what was declared ahead of it (contract F).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Stale against a value declared ahead: the author confirms or cancels.
    ConfirmOnStale,
    /// Overwrites whatever is ahead, no question asked.
    Overwrite,
    /// Composes with the commands ahead (`stock -= 2`): never stale.
    Relative,
}

/// Where a command is in its life (contract G).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// In its partition, nothing ahead of it to confirm.
    Queued,
    /// A value is declared ahead: its author confirms or cancels.
    AwaitingConfirmation,
    /// Confirmed: advances in its place to the head.
    Confirmed,
    /// A critical field or a plan in `human_review`: an approver decides.
    AwaitingReview,
    /// Reached the head unconfirmed: set aside, the queue went on.
    Parked,
    /// Applied by the single applier; `dataVersion` taken.
    Applied,
    /// Refused at the head: an invariant, a cascade, a review.
    Rejected,
    /// Withdrawn by its author.
    Cancelled,
    /// Parked past its `ttl`, withdrawn.
    Expired,
}

impl State {
    /// True once nothing more happens to the command.
    #[must_use]
    pub const fn is_final(self) -> bool {
        matches!(
            self,
            Self::Applied | Self::Rejected | Self::Cancelled | Self::Expired
        )
    }
}

/// What the author saw when they wrote the command (contract G `basedOn`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasedOn {
    /// The `dataVersion` the author read.
    pub version: u64,
    /// The values they read, by field.
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
}

/// A value declared ahead of a command, in its partition (contract G).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAhead {
    /// The command that declares it.
    pub command: String,
    /// Its author.
    pub by: String,
    /// The value, when the field is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Its state.
    pub state: State,
}

/// What a command sees of its partition: the confirmed state, and what
/// waits ahead of it (contract G `projection`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projection {
    /// The values the applied commands left, by field.
    pub confirmed: BTreeMap<String, Value>,
    /// The commands ahead that await their author's word.
    pub pending_ahead: Vec<PendingAhead>,
}

/// How a parked command waits (contract G `parked`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parked {
    /// How long, in seconds, before it expires.
    pub ttl_seconds: u64,
    /// The instant it was parked, seconds since the epoch.
    pub since: u64,
    /// Whether the author was warned of the expiry.
    #[serde(default)]
    pub warned: bool,
}

/// One command of the queue (contract G).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The command's id.
    pub command: String,
    /// The `DataCapability`, versioned: `credit.requestLimitChange@2`.
    pub data_capability: String,
    /// The author.
    pub by: String,
    /// `<Aggregate>/<id>`: one partition per aggregate.
    pub partition: String,
    /// Its place in the partition, from zero, fixed while it waits.
    pub position: u64,
    /// What the author read.
    pub based_on: BasedOn,
    /// The payload: the fields the command writes, and their values.
    pub payload: BTreeMap<String, Value>,
    /// The effect the manifest declares.
    pub effect: Effect,
    /// The fields it touches, as the manifest declares them.
    pub touches: Vec<String>,
    /// How it meets what is declared ahead.
    pub mode: Mode,
    /// What it saw at enqueue.
    pub projection: Projection,
    /// Where it is.
    pub state: State,
    /// Who confirmed, when confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<String>,
    /// How it waits, when parked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked: Option<Parked>,
    /// The command it was requeued from, when requeued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requeued_from: Option<String>,
    /// The idempotency key, when the manifest requires one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl Entry {
    /// The aggregate of its partition: `Account` of `Account/881`.
    #[must_use]
    pub fn aggregate(&self) -> &str {
        self.partition.split('/').next().unwrap_or(&self.partition)
    }

    /// The id of its partition: `881` of `Account/881`.
    #[must_use]
    pub fn target_id(&self) -> &str {
        self.partition.split_once('/').map_or("", |(_, id)| id)
    }

    /// The fields the command writes: its `touches` when it declares some,
    /// else the keys of its payload.
    #[must_use]
    pub fn fields(&self) -> Vec<String> {
        if self.touches.is_empty() {
            self.payload.keys().cloned().collect()
        } else {
            self.touches
                .iter()
                .map(|t| t.rsplit('.').next().unwrap_or(t).to_string())
                .collect()
        }
    }
}

/// What the `DataGuard` answers a caller (contract H).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    /// The command's id.
    pub command_id: String,
    /// Where it ended, or where it waits.
    pub status: State,
    /// The global version the application took.
    pub data_version: Option<u64>,
    /// The invariants violated, when rejected.
    pub violations: Vec<String>,
    /// The review it waits on, when it does.
    pub review_id: Option<String>,
}

impl CommandResult {
    /// The result of a command that waits, or just applied, with no
    /// violation and no review.
    #[must_use]
    pub fn of(entry: &Entry, data_version: Option<u64>) -> Self {
        Self {
            command_id: entry.command.clone(),
            status: entry.state,
            data_version,
            violations: Vec::new(),
            review_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            command: "c_1".to_string(),
            data_capability: "credit.requestLimitChange@2".to_string(),
            by: "carol".to_string(),
            partition: "Account/881".to_string(),
            position: 0,
            based_on: BasedOn::default(),
            payload: BTreeMap::from([("newLimit".to_string(), Value::from(4500))]),
            effect: Effect::Update,
            touches: vec!["Account.creditLimit".to_string()],
            mode: Mode::ConfirmOnStale,
            projection: Projection::default(),
            state: State::Queued,
            confirmation: None,
            parked: None,
            requeued_from: None,
            idempotency_key: None,
        }
    }

    #[test]
    fn a_partition_names_its_aggregate_and_its_id() {
        let e = entry();
        assert_eq!(e.aggregate(), "Account");
        assert_eq!(e.target_id(), "881");
        assert_eq!(e.fields(), ["creditLimit"], "touches, by field name");
        let pure = Entry {
            touches: Vec::new(),
            ..entry()
        };
        assert_eq!(
            pure.fields(),
            ["newLimit"],
            "no touches: the payload's keys"
        );
    }

    #[test]
    fn the_final_states_are_the_four_the_contract_names() {
        for state in [
            State::Applied,
            State::Rejected,
            State::Cancelled,
            State::Expired,
        ] {
            assert!(state.is_final());
        }
        for state in [
            State::Queued,
            State::AwaitingConfirmation,
            State::Confirmed,
            State::AwaitingReview,
            State::Parked,
        ] {
            assert!(!state.is_final());
        }
    }

    #[test]
    fn an_entry_and_a_result_serialise_as_the_contracts_spell_them() {
        let json = serde_json::to_value(entry()).expect("json");
        assert_eq!(json["state"], "queued");
        assert_eq!(json["mode"], "confirm_on_stale");
        assert_eq!(json["effect"], "update");
        let result = serde_json::to_value(CommandResult::of(&entry(), Some(7))).expect("json");
        assert_eq!(result["commandId"], "c_1");
        assert_eq!(result["dataVersion"], 7);
        assert_eq!(result["reviewId"], Value::Null);
    }
}
