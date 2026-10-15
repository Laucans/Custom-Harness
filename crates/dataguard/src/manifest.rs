//! A `DataCapability` as its `data-capability.json` declares it (contract F).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::command::{Effect, Mode};

/// Whether an idempotency key must come with every command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Idempotency {
    /// Every command names one; the same key replays the same result.
    Required,
    /// A command may name one.
    Optional,
}

/// The target of a `DataCapability`: an aggregate, and the id its payload
/// names it by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// The aggregate.
    pub aggregate: String,
    /// The payload field that holds the id (`$accountId`), when one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// The manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataCapability {
    /// `<system>.<name>`.
    pub data_capability: String,
    /// Always `dataguard`.
    pub owner: String,
    /// The manifest's version.
    pub version: u64,
    /// What it does to the data.
    pub effect: Effect,
    /// What it does it to.
    pub target: Target,
    /// The existing fields it changes.
    #[serde(default)]
    pub touches: Vec<String>,
    /// The payload's fields and their types, in words.
    #[serde(default)]
    pub payload: BTreeMap<String, String>,
    /// How it meets what is declared ahead.
    #[serde(default = "confirm_on_stale")]
    pub mode: Mode,
    /// The invariant ids it may affect.
    #[serde(default)]
    pub invariants: Vec<String>,
    /// The permissions a caller needs.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// The Micro-UI ids, systems (`credit.*`) or `*` that may trigger it.
    #[serde(default)]
    pub callable_by: Vec<String>,
    /// Whether a key is required.
    #[serde(default = "optional")]
    pub idempotency_key: Idempotency,
}

const fn confirm_on_stale() -> Mode {
    Mode::ConfirmOnStale
}

const fn optional() -> Idempotency {
    Idempotency::Optional
}

impl DataCapability {
    /// Reads a `data-capability.json`.
    ///
    /// # Errors
    ///
    /// The text is not the contract's JSON.
    pub fn parse(json: &str) -> std::result::Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// `<dataCapability>@<version>`, as the queue names it.
    #[must_use]
    pub fn versioned(&self) -> String {
        format!("{}@{}", self.data_capability, self.version)
    }

    /// True for a pure addition as the manifest declares it: an insert that
    /// touches nothing existing — the fast path, which skips the Resolver.
    #[must_use]
    pub fn is_pure_addition(&self) -> bool {
        self.effect == Effect::Insert && self.touches.is_empty()
    }

    /// True when `caller` — a Micro-UI id or a `<system>.<name>` — is
    /// admitted by `callableBy`: named, in a named system (`credit.*`), or
    /// anyone (`*`).
    #[must_use]
    pub fn admits(&self, caller: &str) -> bool {
        self.callable_by.iter().any(|allowed| {
            allowed == "*"
                || allowed == caller
                || allowed
                    .strip_suffix(".*")
                    .is_some_and(|system| caller.starts_with(&format!("{system}.")))
        })
    }

    /// The permissions `granted` lacks of the ones this command needs.
    #[must_use]
    pub fn missing_permissions(&self, granted: &[String]) -> Vec<String> {
        self.permissions
            .iter()
            .filter(|needed| !granted.contains(needed))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: &str = r#"{
      "dataCapability": "credit.requestLimitChange",
      "owner": "dataguard",
      "version": 2,
      "effect": "update",
      "target": { "aggregate": "Account", "id": "$accountId" },
      "touches": ["Account.creditLimit"],
      "payload": { "newLimit": "Money", "reason": "string" },
      "mode": "confirm_on_stale",
      "invariants": ["limit-under-ceiling", "limit-change-requires-risk"],
      "permissions": ["credit.limit.write"],
      "callableBy": ["credit.*"],
      "idempotencyKey": "required"
    }"#;

    #[test]
    fn the_contract_s_example_parses_whole() {
        let limit = DataCapability::parse(LIMIT).expect("contract F");
        assert_eq!(limit.versioned(), "credit.requestLimitChange@2");
        assert!(!limit.is_pure_addition());
        assert_eq!(limit.mode, Mode::ConfirmOnStale);
        assert_eq!(limit.idempotency_key, Idempotency::Required);
        assert_eq!(limit.target.id.as_deref(), Some("$accountId"));
    }

    #[test]
    fn a_caller_is_admitted_by_name_by_system_or_by_star() {
        let limit = DataCapability::parse(LIMIT).expect("contract F");
        assert!(limit.admits("credit.LimitReviewForm"));
        assert!(!limit.admits("orders.Form"));
        assert!(
            !limit.admits("creditor.Form"),
            "the dot is part of the namespace"
        );
        let open = DataCapability {
            callable_by: vec!["*".to_string()],
            ..limit.clone()
        };
        assert!(open.admits("anyone.Anywhere"));
        let named = DataCapability {
            callable_by: vec!["RiskBadge".to_string()],
            ..limit
        };
        assert!(named.admits("RiskBadge"));
        assert!(!named.admits("LatePaymentsList"));
    }

    #[test]
    fn the_missing_permissions_are_named_and_a_pure_insert_is_the_fast_path() {
        let limit = DataCapability::parse(LIMIT).expect("contract F");
        assert_eq!(limit.missing_permissions(&[]), ["credit.limit.write"]);
        assert!(
            limit
                .missing_permissions(&["credit.limit.write".to_string()])
                .is_empty(),
            "expected nothing"
        );
        let add = DataCapability::parse(
            r#"{"dataCapability":"credit.addPayment","owner":"dataguard","version":1,"effect":"insert","target":{"aggregate":"Payment"}}"#,
        )
        .expect("a minimal manifest");
        assert!(add.is_pure_addition());
        assert_eq!(add.mode, Mode::ConfirmOnStale, "the default");
        assert_eq!(add.idempotency_key, Idempotency::Optional, "the default");
    }
}
